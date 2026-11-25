import { Link } from 'react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Clock, Inbox, RotateCw, X } from 'lucide-react';
import { queueApi } from '../../api/endpoints';
import { keys, useAccounts, useQueue } from '../../api/queries';
import type { QueueItem, QueuePriority } from '../../api/types';
import { Badge } from '../../components/Badge';
import { Button } from '../../components/Button';
import { useConfirm } from '../../components/Confirm';
import { EmptyState, ErrorState, SkeletonRows } from '../../components/Feedback';
import { Select } from '../../components/Form';
import { ProgressBar } from '../../components/Progress';
import { ProviderIcon } from '../../components/ProviderIcon';
import { useNow } from '../../hooks/useNow';
import { formatBytes, formatDuration, formatPercent, formatRelative, formatSpeed, joinNames } from '../../lib/format';
import { PRIORITY, QUEUE_STAGE, QUEUE_STATE } from '../../lib/meta';
import styles from './QueueTab.module.css';

/** Seconds left for the bytes of the current part, when the server reports enough to tell. */
function etaSeconds(item: QueueItem): number | null {
  if (!item.bytesPerSec || item.bytesTotal === null || item.bytesDone === null) return null;
  const remaining = item.bytesTotal - item.bytesDone;
  return remaining > 0 ? remaining / item.bytesPerSec : null;
}

export function QueueTab() {
  const queue = useQueue();
  const accounts = useAccounts();
  const client = useQueryClient();
  const confirm = useConfirm();
  const now = useNow();

  const replaceItem = (item: QueueItem) =>
    client.setQueryData<QueueItem[]>(keys.queue, (list) => list?.map((q) => (q.id === item.id ? item : q)));

  const cancel = useMutation({
    mutationFn: queueApi.cancel,
    meta: { errorTitle: 'Could not cancel' },
    onSuccess: (_r, id) => {
      client.setQueryData<QueueItem[]>(keys.queue, (list) => list?.filter((q) => q.id !== id));
      void client.invalidateQueries({ queryKey: keys.books });
    },
  });
  const retry = useMutation({
    mutationFn: queueApi.retry,
    meta: { errorTitle: 'Could not retry' },
    onSuccess: replaceItem,
  });
  const move = useMutation({
    mutationFn: ({ id, priority }: { id: string; priority: QueuePriority }) => queueApi.move(id, priority),
    meta: { errorTitle: 'Could not change priority' },
    onSuccess: () => void client.invalidateQueries({ queryKey: keys.queue }),
  });

  const onCancel = async (item: QueueItem) => {
    const running = item.state === 'running' || item.state === 'planning';
    const ok = await confirm({
      title: 'Cancel download?',
      message: (
        <p>
          {running
            ? `${item.title} will stop. Partial files are kept, so a retry can resume.`
            : `${item.title} will be removed from the queue.`}
        </p>
      ),
      confirmLabel: 'Cancel download',
      tone: 'danger',
    });
    if (ok) cancel.mutate(item.id);
  };

  if (queue.isPending) return <SkeletonRows rows={4} label="Loading queue" />;
  if (queue.isError) return <ErrorState error={queue.error} onRetry={() => void queue.refetch()} />;
  if (queue.data.length === 0) {
    return (
      <EmptyState icon={<Inbox size={24} />} title="Nothing in the queue">
        New purchases are queued after each sync, and back-fill adds older books at a slow pace. You can also pick
        books to download from the <Link to="/library">Library</Link> or <Link to="/wanted">Wanted</Link>.
      </EmptyState>
    );
  }

  const accountName = (id: string) => accounts.data?.find((a) => a.id === id)?.name ?? id;

  return (
    <ol className={styles.list}>
      {queue.data.map((item) => {
        const state = QUEUE_STATE[item.state];
        const running = item.state === 'running' || item.state === 'planning';
        const eta = etaSeconds(item);
        const canRetry = item.state === 'failed' || item.state === 'cancelled';
        return (
          <li key={item.id} className={styles.item}>
            <div className={styles.head}>
              <ProviderIcon provider={item.provider} />
              <div className={styles.titles}>
                <Link to={`/library?book=${encodeURIComponent(item.bookId)}`} className={styles.title}>
                  {item.title}
                </Link>
                <p className={styles.sub}>
                  {joinNames(item.authors, 2)} · {accountName(item.accountId)}
                </p>
              </div>
              <div className={styles.badges}>
                <Badge tone={state.tone} icon={<state.Icon />}>
                  {state.label}
                </Badge>
                <Badge tone={PRIORITY[item.priority].tone} title={PRIORITY[item.priority].description}>
                  {PRIORITY[item.priority].label}
                </Badge>
              </div>
            </div>

            {running && (
              <div className={styles.progress}>
                <ProgressBar value={item.progress} label={`Progress of ${item.title}`} />
                <p className={styles.stats}>
                  <span>
                    {item.stage ? QUEUE_STAGE[item.stage] : 'Requesting licence'}
                    {item.parts && item.parts > 1 ? ` · part ${item.part ?? 1} of ${item.parts}` : ''}
                  </span>
                  {item.progress !== null && <span>{formatPercent(item.progress)}</span>}
                  {item.bytesTotal !== null && (
                    <span>
                      {formatBytes(item.bytesDone)} / {formatBytes(item.bytesTotal)}
                    </span>
                  )}
                  {item.bytesPerSec !== null && <span>{formatSpeed(item.bytesPerSec)}</span>}
                  {eta !== null && (
                    <span>
                      {formatDuration(eta)} left{item.parts && item.parts > 1 ? ' in this part' : ''}
                    </span>
                  )}
                </p>
              </div>
            )}

            {item.state === 'waiting' && (
              <p className={styles.waiting}>
                <Clock size={14} aria-hidden />
                <span>
                  {item.waitingReason ?? 'Waiting'}
                  {item.notBefore && new Date(item.notBefore).getTime() > now && (
                    <> · starts {formatRelative(item.notBefore, now)}</>
                  )}
                </span>
              </p>
            )}

            {item.error && (
              <p className={styles.error}>
                {item.error}
                {item.attempts > 1 ? ` (attempt ${item.attempts})` : ''}
              </p>
            )}

            <div className={styles.actions}>
              <span className={styles.meta}>Added {formatRelative(item.createdAt, now)}</span>
              <Select
                aria-label={`Priority of ${item.title}`}
                value={item.priority}
                onChange={(e) => move.mutate({ id: item.id, priority: e.target.value as QueuePriority })}
                disabled={running || move.isPending}
                className={styles.priority}
              >
                {(Object.keys(PRIORITY) as QueuePriority[]).map((p) => (
                  <option key={p} value={p}>
                    Priority: {PRIORITY[p].label}
                  </option>
                ))}
              </Select>
              {canRetry && (
                <Button size="sm" icon={<RotateCw size={14} />} onClick={() => retry.mutate(item.id)} loading={retry.isPending && retry.variables === item.id}>
                  Retry
                </Button>
              )}
              <Button size="sm" variant="ghost" icon={<X size={14} />} onClick={() => void onCancel(item)}>
                Cancel
              </Button>
            </div>
          </li>
        );
      })}
    </ol>
  );
}
