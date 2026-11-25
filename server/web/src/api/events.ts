import { useEffect, useState } from 'react';
import { useQueryClient, type QueryClient } from '@tanstack/react-query';
import { API_BASE } from './client';
import { keys } from './queries';
import type { AccountView, HealthItem, HistoryItem, QueueItem, ScheduledTask } from './types';

export type EventStreamStatus = 'connecting' | 'open' | 'reconnecting';

const FINISHED: ReadonlySet<QueueItem['state']> = new Set(['completed', 'cancelled']);

/** Coalesces bursts of events (a sync adding 200 books) into one refetch per key. */
function makeDebouncedInvalidator(client: QueryClient, delayMs: number) {
  const pending = new Map<string, readonly unknown[]>();
  let timer: ReturnType<typeof setTimeout> | null = null;
  const flush = () => {
    timer = null;
    pending.forEach((queryKey) => void client.invalidateQueries({ queryKey }));
    pending.clear();
  };
  return {
    invalidate(queryKey: readonly unknown[]) {
      pending.set(JSON.stringify(queryKey), queryKey);
      if (timer === null) timer = setTimeout(flush, delayMs);
    },
    cancel() {
      if (timer !== null) clearTimeout(timer);
      pending.clear();
    },
  };
}

function parse<T>(e: MessageEvent): T | null {
  try {
    return JSON.parse(e.data as string) as T;
  } catch {
    return null;
  }
}

function applyQueueItem(client: QueryClient, item: QueueItem): QueueItem | undefined {
  const previous = client.getQueryData<QueueItem[]>(keys.queue)?.find((q) => q.id === item.id);
  client.setQueryData<QueueItem[]>(keys.queue, (list) => {
    if (!list) return list;
    if (FINISHED.has(item.state)) return list.filter((q) => q.id !== item.id);
    const i = list.findIndex((q) => q.id === item.id);
    if (i === -1) return [...list, item];
    const next = list.slice();
    next[i] = item;
    return next;
  });
  return previous;
}

function applyAccount(client: QueryClient, account: AccountView) {
  client.setQueryData<AccountView[]>(keys.accounts, (list) => {
    if (!list) return list;
    const i = list.findIndex((a) => a.id === account.id);
    if (i === -1) return [...list, account];
    const next = list.slice();
    next[i] = account;
    return next;
  });
}

function applyTask(client: QueryClient, task: ScheduledTask) {
  client.setQueryData<ScheduledTask[]>(keys.tasks, (list) =>
    list?.map((t) => (t.name === task.name ? task : t)),
  );
}

/**
 * Subscribes to `/api/v1/events` for the lifetime of the authenticated shell and patches
 * the query cache from it. Mount exactly once.
 *
 * EventSource reconnects by itself after a dropped connection, but gives up for good on
 * an HTTP error (e.g. a 401 or a restarting server answering 502), so a closed stream is
 * reopened here with backoff. Anything missed while disconnected is recovered by
 * invalidating every query once the stream is back.
 */
export function useEvents(): EventStreamStatus {
  const client = useQueryClient();
  const [status, setStatus] = useState<EventStreamStatus>('connecting');

  useEffect(() => {
    let source: EventSource | null = null;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;
    let attempt = 0;
    let everOpened = false;
    let disposed = false;
    const lists = makeDebouncedInvalidator(client, 1_000);

    const connect = () => {
      source = new EventSource(`${API_BASE}/events`, { withCredentials: true });

      source.onopen = () => {
        attempt = 0;
        setStatus('open');
        // On a reconnect we may have missed events; refetch everything that is mounted.
        if (everOpened) void client.invalidateQueries({ predicate: (q) => q.queryKey[0] !== 'authState' });
        everOpened = true;
      };

      source.onerror = () => {
        if (disposed || !source) return;
        if (source.readyState === EventSource.CLOSED) {
          source = null;
          setStatus('reconnecting');
          const delay = Math.min(30_000, 1_000 * 2 ** attempt++);
          retryTimer = setTimeout(connect, delay);
          // The stream may have closed because the session expired; checking auth
          // state routes to the login page if so.
          void client.invalidateQueries({ queryKey: keys.authState });
        } else {
          setStatus('reconnecting');
        }
      };

      source.addEventListener('queue', (e) => {
        const item = parse<QueueItem>(e);
        if (!item) return;
        const previous = applyQueueItem(client, item);
        // Progress ticks arrive once a second; only a state change affects book lists.
        if (!previous || previous.state !== item.state) {
          lists.invalidate(keys.books);
          lists.invalidate(keys.wanted);
          lists.invalidate(keys.status);
        }
      });

      source.addEventListener('queueRemoved', (e) => {
        const data = parse<{ id: string }>(e);
        if (!data) return;
        client.setQueryData<QueueItem[]>(keys.queue, (list) => list?.filter((q) => q.id !== data.id));
        lists.invalidate(keys.books);
        lists.invalidate(keys.wanted);
      });

      source.addEventListener('history', (e) => {
        const item = parse<HistoryItem>(e);
        if (!item) return;
        lists.invalidate(keys.history);
        if (item.bookId) lists.invalidate(keys.book(item.bookId));
        if (item.event === 'bookAdded' || item.event === 'syncCompleted') {
          lists.invalidate(keys.books);
          lists.invalidate(keys.wanted);
          lists.invalidate(keys.status);
        }
      });

      source.addEventListener('account', (e) => {
        const account = parse<AccountView>(e);
        if (account) applyAccount(client, account);
      });

      source.addEventListener('health', (e) => {
        const items = parse<HealthItem[]>(e);
        if (items) client.setQueryData(keys.health, items);
      });

      source.addEventListener('task', (e) => {
        const task = parse<ScheduledTask>(e);
        if (task) applyTask(client, task);
      });
    };

    connect();

    return () => {
      disposed = true;
      lists.cancel();
      if (retryTimer !== null) clearTimeout(retryTimer);
      source?.close();
    };
  }, [client]);

  return status;
}
