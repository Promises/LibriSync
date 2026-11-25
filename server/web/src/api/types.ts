// Types for the LibriSync Server HTTP API (v1), transcribed by hand from server/API.md.
// Keep this file a faithful mirror of that document: when the contract changes, change it
// here first and let the compiler find every consumer.

/** RFC 3339 UTC timestamp. */
export type Timestamp = string;

export type Page<T> = {
  total: number;
  page: number;
  pageSize: number;
  items: T[];
};

export type PageParams = {
  page?: number;
  pageSize?: number;
};

// ---- Health and system -------------------------------------------------------------

export type SystemStatus = {
  version: string;
  startTime: Timestamp;
  configDir: string;
  libraryDir: string;
  ffmpegVersion: string | null;
  isDocker: boolean;
  os: string;
  accounts: number;
  books: number;
  downloaded: number;
  queued: number;
};

export type HealthType = 'ok' | 'notice' | 'warning' | 'error';

export type HealthItem = {
  source: string;
  type: HealthType;
  message: string;
};

export type TaskName = 'sync' | 'backfill' | 'clientProfiles' | 'housekeeping';

export type ScheduledTask = {
  name: TaskName;
  displayName: string;
  intervalMinutes: number;
  lastRun: Timestamp | null;
  nextRun: Timestamp | null;
  running: boolean;
};

export type LogLevel = 'trace' | 'debug' | 'info' | 'warn' | 'error';

export type LogLine = {
  time: Timestamp;
  level: LogLevel;
  target: string;
  message: string;
};

// ---- Auth --------------------------------------------------------------------------

export type AuthState = {
  setupRequired: boolean;
  authenticated: boolean;
  username: string | null;
};

export type Credentials = { username: string; password: string };

// ---- Providers ---------------------------------------------------------------------

export type ProviderId = 'audible' | 'librofm' | 'chirp';

export type LoginMethod = 'deviceCode' | 'externalBrowser' | 'credentials';

export type OutputFormat = 'm4b' | 'mp3';

export type ProviderInfo = {
  id: ProviderId;
  name: string;
  loginMethods: LoginMethod[];
  /** Audible only; empty otherwise. */
  locales: { code: string; name: string }[];
  formats: OutputFormat[];
};

// ---- Accounts ----------------------------------------------------------------------

export type SyncResult = {
  ok: boolean;
  added: number;
  updated: number;
  total: number;
  message: string | null;
};

export type AccountView = {
  id: string;
  provider: ProviderId;
  name: string;
  locale: string | null;
  enabled: boolean;
  autoDownloadNew: boolean;
  backfill: boolean;
  /** A sync is running now. */
  syncing: boolean;
  lastSync: Timestamp | null;
  nextSync: Timestamp | null;
  lastSyncResult: SyncResult | null;
  bookCount: number;
  downloadedCount: number;
  cooldownUntil: Timestamp | null;
  cooldownReason: string | null;
  licencesLast24h: number;
  /** null for DRM-free providers. */
  dailyLicenceCap: number | null;
  authStatus: 'ok' | 'expired' | 'unknown';
};

export type AccountPatch = Partial<Pick<AccountView, 'enabled' | 'autoDownloadNew' | 'backfill'>>;

export type DeviceCodeSession = {
  sessionId: string;
  status: 'pending' | 'registered' | 'expired' | 'error';
  publicCode: string;
  verificationUrl: string;
  expiresAt: Timestamp;
  account: AccountView | null;
  message: string | null;
};

export type ExternalLoginSession = { sessionId: string; url: string };

// ---- Books -------------------------------------------------------------------------

export type BookStatus =
  | 'downloaded'
  | 'queued'
  | 'downloading'
  | 'wanted'
  | 'cooldown'
  | 'failed'
  | 'skipped';

export type BookView = {
  id: string;
  provider: ProviderId;
  accounts: string[];
  title: string;
  subtitle: string | null;
  authors: string[];
  narrators: string[];
  series: string | null;
  seriesSequence: string | null;
  coverUrl: string | null;
  lengthMinutes: number;
  releaseDate: string | null;
  purchaseDate: string | null;
  addedAt: Timestamp;
  status: BookStatus;
  /** Relative to libraryDir. */
  outputPath: string | null;
  format: OutputFormat | null;
  downloadedAt: Timestamp | null;
  lastError: string | null;
  backoffUntil: Timestamp | null;
};

export type BookDetail = BookView & {
  description: string;
  history: HistoryItem[];
};

export type BookSort = 'title' | 'author' | 'added' | 'purchased' | 'released';
export type SortDir = 'asc' | 'desc';

export type BookQuery = PageParams & {
  search?: string;
  provider?: ProviderId;
  account?: string;
  status?: BookStatus[];
  sort?: BookSort;
  dir?: SortDir;
};

export type BulkDownloadResult = {
  queued: number;
  skipped: { id: string; reason: string }[];
};

export type WantedPage = Page<BookView> & {
  estimate: { perDay: number; daysRemaining: number | null };
};

// ---- Queue -------------------------------------------------------------------------

export type QueueState =
  | 'queued'
  | 'waiting'
  | 'planning'
  | 'running'
  | 'completed'
  | 'failed'
  | 'cancelled';

export type QueuePriority = 'manual' | 'new' | 'backfill';

export type QueueStage =
  | 'downloading'
  | 'decrypting'
  | 'extracting'
  | 'joining'
  | 'validating'
  | 'encoding'
  | 'finalizing';

export type QueueItem = {
  id: string;
  bookId: string;
  accountId: string;
  provider: ProviderId;
  title: string;
  authors: string[];
  priority: QueuePriority;
  state: QueueState;
  waitingReason: string | null;
  notBefore: Timestamp | null;
  stage: QueueStage | null;
  part: number | null;
  parts: number | null;
  bytesDone: number | null;
  bytesTotal: number | null;
  bytesPerSec: number | null;
  /** 0..1 overall estimate. */
  progress: number | null;
  attempts: number;
  error: string | null;
  createdAt: Timestamp;
  startedAt: Timestamp | null;
  finishedAt: Timestamp | null;
};

// ---- History -----------------------------------------------------------------------

export type HistoryEvent =
  | 'bookAdded'
  | 'grab'
  | 'download'
  | 'downloadFailed'
  | 'licenceDenied'
  | 'licenceThrottled'
  | 'syncCompleted'
  | 'syncFailed'
  | 'accountAdded'
  | 'accountRemoved'
  | 'authExpired';

export const HISTORY_EVENTS: readonly HistoryEvent[] = [
  'bookAdded',
  'grab',
  'download',
  'downloadFailed',
  'licenceDenied',
  'licenceThrottled',
  'syncCompleted',
  'syncFailed',
  'accountAdded',
  'accountRemoved',
  'authExpired',
];

export type HistoryItem = {
  id: number;
  time: Timestamp;
  event: HistoryEvent;
  accountId: string | null;
  bookId: string | null;
  title: string | null;
  message: string;
  data: Record<string, unknown> | null;
};

export type HistoryQuery = PageParams & {
  event?: HistoryEvent[];
  account?: string;
  book?: string;
};

// ---- Settings ----------------------------------------------------------------------

export type ServerLogLevel = 'debug' | 'info' | 'warn';
export type BackfillOrder = 'newest_first' | 'oldest_first';

export type Settings = {
  general: {
    instanceName: string;
    /** Read-only. */
    apiKey: string;
    logLevel: ServerLogLevel;
  };
  media: {
    /** Read-only (env). */
    libraryDir: string;
    folderTemplate: string;
    noSeriesFolderTemplate: string;
    format: OutputFormat;
    writeSidecars: boolean;
  };
  sync: {
    /** 1..168 */
    intervalHours: number;
    autoDownloadNew: boolean;
  };
  backfill: {
    enabled: boolean;
    order: BackfillOrder;
    /** "HH:MM" local; both or neither. */
    windowStart: string | null;
    windowEnd: string | null;
  };
  audible: {
    /** 1..200 */
    dailyLicenceCap: number;
    spacingMinMinutes: number;
    spacingMaxMinutes: number;
    /** Base, doubles; default 24. */
    throttleCooldownHours: number;
  };
  downloads: {
    /** 1..4 */
    concurrency: number;
  };
};

// ---- Notifications -----------------------------------------------------------------

export type NotificationKind = 'webhook' | 'apprise' | 'audiobookshelf';

export type WebhookConfig = {
  url: string;
  method: 'POST' | 'PUT';
  headers: { key: string; value: string }[];
  username?: string;
  password?: string;
};

export type AppriseConfig = {
  serverUrl: string;
  configKey?: string;
  statelessUrls?: string;
  tags?: string;
};

export type AbsConfig = {
  url: string;
  apiKey: string;
  libraryId: string;
  pathPrefix?: string;
};

/** History events plus health changes. ('test' is sent only by the test endpoint.) */
export type NotificationEvent = HistoryEvent | 'health' | 'healthRestored';

export const NOTIFICATION_EVENTS: readonly NotificationEvent[] = [...HISTORY_EVENTS, 'health', 'healthRestored'];

type NotificationBase = {
  id: number;
  name: string;
  enabled: boolean;
  lastResult: { ok: boolean; time: Timestamp; message: string | null } | null;
};

// API.md types `config` as an untagged union; the UI narrows it by `kind`, which is
// the only thing that determines its shape.
export type Notification =
  | (NotificationBase & { kind: 'webhook'; events: NotificationEvent[] | null; config: WebhookConfig })
  | (NotificationBase & { kind: 'apprise'; events: NotificationEvent[] | null; config: AppriseConfig })
  | (NotificationBase & { kind: 'audiobookshelf'; events: null; config: AbsConfig });

/** What the UI sends when creating; the server assigns `id` and `lastResult`. */
type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;
export type NotificationDraft = DistributiveOmit<Notification, 'id' | 'lastResult'> & {
  id?: number;
};

export type NotificationTestResult = { ok: boolean; message: string };

export type AbsLibrary = { id: string; name: string; folders: string[] };

/** The value the server returns in place of a stored secret; sending it back keeps it. */
export const SECRET_MASK = '********';

// ---- Errors ------------------------------------------------------------------------

export type ApiErrorCode =
  | 'unauthorized'
  | 'forbidden'
  | 'not_found'
  | 'invalid_input'
  | 'conflict'
  | 'setup_required'
  | 'license_throttled'
  | 'account_cooldown'
  | 'provider_error'
  | 'internal';
