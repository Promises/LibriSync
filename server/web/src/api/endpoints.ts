import { request, seg, API_BASE } from './client';
import type {
  AbsLibrary,
  AccountPatch,
  AccountView,
  AuthState,
  BookDetail,
  BookQuery,
  BookView,
  BulkDownloadResult,
  Credentials,
  DeviceCodeSession,
  ExternalLoginSession,
  HealthItem,
  HistoryItem,
  HistoryQuery,
  LogLevel,
  LogLine,
  Notification,
  NotificationDraft,
  NotificationTestResult,
  Page,
  PageParams,
  ProviderInfo,
  QueueItem,
  QueuePriority,
  ScheduledTask,
  Settings,
  SystemStatus,
  TaskName,
  WantedPage,
} from './types';

// One function per endpoint in server/API.md, grouped as the document groups them.

export const systemApi = {
  status: () => request<SystemStatus>('/system/status'),
  health: () => request<HealthItem[]>('/health'),
  tasks: () => request<ScheduledTask[]>('/system/tasks'),
  runTask: (name: TaskName) => request<void>(`/system/tasks/${seg(name)}/run`, { method: 'POST' }),
  log: (lines: number, level: LogLevel) =>
    request<{ lines: LogLine[] }>('/log', { query: { lines, level } }),
};

export const authApi = {
  state: () => request<AuthState>('/auth/state'),
  setup: (body: Credentials) => request<void>('/auth/setup', { method: 'POST', body }),
  login: (body: Credentials) => request<void>('/auth/login', { method: 'POST', body }),
  logout: () => request<void>('/auth/logout', { method: 'POST' }),
  changePassword: (body: { currentPassword: string; newPassword: string }) =>
    request<void>('/auth/password', { method: 'POST', body }),
};

export const providersApi = {
  list: () => request<ProviderInfo[]>('/providers'),
};

export const accountsApi = {
  list: () => request<AccountView[]>('/accounts'),
  get: (id: string) => request<AccountView>(`/accounts/${seg(id)}`),
  patch: (id: string, body: AccountPatch) =>
    request<AccountView>(`/accounts/${seg(id)}`, { method: 'PATCH', body }),
  remove: (id: string, deregister: boolean) =>
    request<void>(`/accounts/${seg(id)}`, {
      method: 'DELETE',
      query: deregister ? { deregister: true } : undefined,
    }),
  sync: (id: string) => request<void>(`/accounts/${seg(id)}/sync`, { method: 'POST' }),
  clearCooldown: (id: string) =>
    request<AccountView>(`/accounts/${seg(id)}/clear-cooldown`, { method: 'POST' }),
};

export const loginApi = {
  startDeviceCode: (locale: string) =>
    request<DeviceCodeSession>('/login/audible/device-code', { method: 'POST', body: { locale } }),
  pollDeviceCode: (sessionId: string) =>
    request<DeviceCodeSession>(`/login/audible/device-code/${seg(sessionId)}`),
  cancelDeviceCode: (sessionId: string) =>
    request<void>(`/login/audible/device-code/${seg(sessionId)}`, { method: 'DELETE' }),
  startExternal: (locale: string) =>
    request<ExternalLoginSession>('/login/audible/external', { method: 'POST', body: { locale } }),
  finishExternal: (sessionId: string, responseUrl: string) =>
    request<AccountView>(`/login/audible/external/${seg(sessionId)}`, {
      method: 'POST',
      body: { responseUrl },
    }),
  credentials: (body: { provider: 'librofm' | 'chirp'; email: string; password: string }) =>
    request<AccountView>('/login/credentials', { method: 'POST', body }),
  importAccount: (accountJson: string) =>
    request<AccountView>('/login/import', { method: 'POST', body: { accountJson } }),
};

function bookQuery(q: BookQuery) {
  return {
    page: q.page,
    pageSize: q.pageSize,
    search: q.search,
    provider: q.provider,
    account: q.account,
    status: q.status,
    sort: q.sort,
    dir: q.dir,
  };
}

export const booksApi = {
  list: (q: BookQuery) => request<Page<BookView>>('/books', { query: bookQuery(q) }),
  get: (id: string) => request<BookDetail>(`/books/${seg(id)}`),
  download: (id: string, force = false) =>
    request<QueueItem>(`/books/${seg(id)}/download`, {
      method: 'POST',
      body: force ? { force: true } : {},
    }),
  downloadMany: (ids: string[]) =>
    request<BulkDownloadResult>('/books/download', { method: 'POST', body: { ids } }),
  skip: (id: string) => request<BookView>(`/books/${seg(id)}/skip`, { method: 'POST' }),
  unskip: (id: string) => request<BookView>(`/books/${seg(id)}/unskip`, { method: 'POST' }),
  forget: (id: string) => request<BookView>(`/books/${seg(id)}/forget`, { method: 'POST' }),
  coverUrl: (id: string) => `${API_BASE}/books/${seg(id)}/cover`,
  wanted: (q: PageParams) => request<WantedPage>('/wanted', { query: q }),
};

export const queueApi = {
  list: () => request<QueueItem[]>('/queue'),
  cancel: (id: string) => request<void>(`/queue/${seg(id)}`, { method: 'DELETE' }),
  retry: (id: string) => request<QueueItem>(`/queue/${seg(id)}/retry`, { method: 'POST' }),
  move: (id: string, priority: QueuePriority) =>
    request<QueueItem>(`/queue/${seg(id)}/move`, { method: 'POST', body: { priority } }),
};

export const historyApi = {
  list: (q: HistoryQuery) =>
    request<Page<HistoryItem>>('/history', {
      query: { page: q.page, pageSize: q.pageSize, event: q.event, account: q.account, book: q.book },
    }),
};

export const settingsApi = {
  get: () => request<Settings>('/settings'),
  put: (body: Settings) => request<Settings>('/settings', { method: 'PUT', body }),
  regenerateApiKey: () =>
    request<{ apiKey: string }>('/settings/api-key/regenerate', { method: 'POST' }),
};

export const notificationsApi = {
  list: () => request<Notification[]>('/notifications'),
  create: (body: NotificationDraft) =>
    request<Notification>('/notifications', { method: 'POST', body }),
  update: (id: number, body: NotificationDraft) =>
    request<Notification>(`/notifications/${seg(id)}`, { method: 'PUT', body }),
  remove: (id: number) => request<void>(`/notifications/${seg(id)}`, { method: 'DELETE' }),
  test: (body: NotificationDraft) =>
    request<NotificationTestResult>('/notifications/test', { method: 'POST', body }),
  /** With the `id` of a saved connection, a masked `apiKey` means "use the stored key". */
  absLibraries: (url: string, apiKey: string, id?: number) =>
    request<AbsLibrary[]>('/notifications/abs/libraries', { method: 'POST', body: { url, apiKey, id } }),
};
