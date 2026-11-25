import { keepPreviousData, useQuery } from '@tanstack/react-query';
import {
  accountsApi,
  authApi,
  booksApi,
  historyApi,
  notificationsApi,
  providersApi,
  queueApi,
  settingsApi,
  systemApi,
} from './endpoints';
import type { BookQuery, HistoryQuery, LogLevel, PageParams } from './types';

/** Every cache key the app uses, so invalidation and SSE patching agree on them. */
export const keys = {
  authState: ['authState'] as const,
  providers: ['providers'] as const,
  accounts: ['accounts'] as const,
  books: ['books'] as const,
  bookList: (q: BookQuery) => ['books', 'list', q] as const,
  book: (id: string) => ['books', 'detail', id] as const,
  wanted: ['wanted'] as const,
  wantedList: (q: PageParams) => ['wanted', q] as const,
  queue: ['queue'] as const,
  history: ['history'] as const,
  historyList: (q: HistoryQuery) => ['history', q] as const,
  settings: ['settings'] as const,
  notifications: ['notifications'] as const,
  status: ['system', 'status'] as const,
  health: ['system', 'health'] as const,
  tasks: ['system', 'tasks'] as const,
  log: (lines: number, level: LogLevel) => ['system', 'log', lines, level] as const,
};

export const useAuthState = () =>
  useQuery({ queryKey: keys.authState, queryFn: authApi.state, staleTime: Infinity });

export const useProviders = () =>
  useQuery({ queryKey: keys.providers, queryFn: providersApi.list, staleTime: Infinity });

export const useAccounts = () => useQuery({ queryKey: keys.accounts, queryFn: accountsApi.list });

export const useBooks = (q: BookQuery) =>
  useQuery({
    queryKey: keys.bookList(q),
    queryFn: () => booksApi.list(q),
    placeholderData: keepPreviousData,
  });

export const useBook = (id: string | null) =>
  useQuery({
    queryKey: keys.book(id ?? ''),
    queryFn: () => booksApi.get(id as string),
    enabled: id !== null,
  });

export const useWanted = (q: PageParams) =>
  useQuery({
    queryKey: keys.wantedList(q),
    queryFn: () => booksApi.wanted(q),
    placeholderData: keepPreviousData,
  });

/** Kept fresh by SSE; the slow poll only covers a dropped event stream. */
export const useQueue = () =>
  useQuery({ queryKey: keys.queue, queryFn: queueApi.list, refetchInterval: 60_000 });

export const useHistory = (q: HistoryQuery) =>
  useQuery({
    queryKey: keys.historyList(q),
    queryFn: () => historyApi.list(q),
    placeholderData: keepPreviousData,
  });

export const useSettings = () =>
  useQuery({ queryKey: keys.settings, queryFn: settingsApi.get, staleTime: Infinity });

export const useNotifications = () =>
  useQuery({ queryKey: keys.notifications, queryFn: notificationsApi.list });

export const useSystemStatus = () =>
  useQuery({ queryKey: keys.status, queryFn: systemApi.status });

export const useHealth = () =>
  useQuery({ queryKey: keys.health, queryFn: systemApi.health, refetchInterval: 5 * 60_000 });

export const useTasks = () => useQuery({ queryKey: keys.tasks, queryFn: systemApi.tasks });

export const useLog = (lines: number, level: LogLevel, autoRefresh: boolean) =>
  useQuery({
    queryKey: keys.log(lines, level),
    queryFn: () => systemApi.log(lines, level),
    refetchInterval: autoRefresh ? 5_000 : false,
  });
