# LibriSync Server — HTTP API (v1)

The contract between `server/src` (backend) and `server/web` (UI). JSON in and out,
`camelCase` keys, timestamps are RFC 3339 UTC strings, ids are strings unless noted.

**Auth.** Every `/api/v1/*` route except those marked *anon* requires either a session
cookie (`librisync_session`, set by `auth/login`) or the API key (`X-Api-Key` header or
`?apikey=` query). Unauthenticated → `401 {"error":"unauthorized"}`.

**Errors.** Any non-2xx carries `{"error": "<machine_code>", "message": "<human text>"}`,
plus `"field": "<dotted.path>"` on `400 invalid_input` when one field is at fault
(e.g. `"audible.dailyLicenceCap"`, `"currentPassword"`).
Machine codes include `unauthorized`, `not_found`, `invalid_input`, `conflict`,
`setup_required`, `license_throttled`, `account_cooldown`, `provider_error`, `internal`.

**Pagination.** List endpoints that page take `page` (1-based, default 1) and `pageSize`
(default 50, max 500) and return `{"total": n, "page": p, "pageSize": s, "items": [...]}`.

---

## Health and system

| Method | Path | Notes |
|---|---|---|
| GET/HEAD | `/ping` | *anon*. `{"status":"OK"}` — Docker HEALTHCHECK |
| GET | `/api/v1/system/status` | `SystemStatus` |
| GET | `/api/v1/health` | `HealthItem[]` |
| GET | `/api/v1/system/tasks` | `ScheduledTask[]` |
| POST | `/api/v1/system/tasks/{name}/run` | run now → `204` |
| GET | `/api/v1/log?lines=500&level=info` | `{ "lines": LogLine[] }`, newest last; `level` = that level and above |
| GET | `/api/v1/events` | Server-Sent Events, see below |

```ts
type SystemStatus = {
  version: string; startTime: string; configDir: string; libraryDir: string;
  ffmpegVersion: string | null; isDocker: boolean; os: string;
  accounts: number; books: number; downloaded: number; queued: number;
};
type HealthItem = { source: string; type: 'ok' | 'notice' | 'warning' | 'error'; message: string };
type ScheduledTask = {
  name: 'sync' | 'backfill' | 'clientProfiles' | 'housekeeping';
  displayName: string; intervalMinutes: number;
  lastRun: string | null; nextRun: string | null; running: boolean;
};
type LogLine = { time: string; level: 'trace'|'debug'|'info'|'warn'|'error'; target: string; message: string };
```

Health sources the backend reports: `ffmpeg` (missing/too old), `libraryDir` (missing /
not writable), `disk` (< 2 GB free), `account:<id>` (auth expired, cooldown active, last
sync failed), `notifications` (last delivery failed).

**SSE `/api/v1/events`** (cookie or `?apikey=`). Event names and `data` payloads:

- `queue` — `QueueItem` (on every state/progress change; progress at most 1/s per item)
- `queueRemoved` — `{ "id": string }`
- `history` — `HistoryItem`
- `account` — `AccountView` (sent on every change, including sync start/finish and each
  completed download, so counts and `syncing` stay live)
- `health` — `HealthItem[]`
- `task` — `ScheduledTask`

## Auth

| Method | Path | Body → Response |
|---|---|---|
| GET | `/api/v1/auth/state` | *anon* → `{ "setupRequired": bool, "authenticated": bool, "username": string\|null }` |
| POST | `/api/v1/auth/setup` | *anon, only while setupRequired* `{username, password}` → `204` + session cookie |
| POST | `/api/v1/auth/login` | *anon* `{username, password}` → `204` + session cookie; `401` on bad credentials |
| POST | `/api/v1/auth/logout` | → `204`, clears cookie |
| POST | `/api/v1/auth/password` | `{currentPassword, newPassword}` → `204`; wrong current password → `400 invalid_input` with `field: "currentPassword"` |

Passwords: minimum 8 characters.

## Providers

`GET /api/v1/providers` → `ProviderInfo[]`

```ts
type ProviderId = 'audible' | 'librofm' | 'chirp';
type ProviderInfo = {
  id: ProviderId; name: string;
  loginMethods: ('deviceCode' | 'externalBrowser' | 'credentials' | 'import')[];
  locales: { code: string; name: string }[];   // Audible only; empty otherwise
  formats: ('m4b' | 'mp3')[];
};
```

## Accounts

```ts
type AccountView = {
  id: string;                  // Accounts.account_id
  provider: ProviderId;
  name: string;                // display name
  locale: string | null;       // Audible country code
  enabled: boolean;            // included in scheduled sync
  autoDownloadNew: boolean;
  backfill: boolean;
  syncing: boolean;            // a sync is running now
  lastSync: string | null; nextSync: string | null;
  lastSyncResult: { ok: boolean; added: number; updated: number; total: number; message: string | null } | null;
  bookCount: number; downloadedCount: number;
  cooldownUntil: string | null;    // Audible licence cooldown
  cooldownReason: string | null;
  licencesLast24h: number; dailyLicenceCap: number | null;   // null for DRM-free providers
  authStatus: 'ok' | 'expired' | 'unknown';
};
```

| Method | Path | Body → Response |
|---|---|---|
| GET | `/api/v1/accounts` | → `AccountView[]` |
| GET | `/api/v1/accounts/{id}` | → `AccountView` |
| PATCH | `/api/v1/accounts/{id}` | `{enabled?, autoDownloadNew?, backfill?}` → `AccountView` |
| DELETE | `/api/v1/accounts/{id}` | `?deregister=true` also deregisters the Audible device → `204`. Books stay on disk. |
| POST | `/api/v1/accounts/{id}/sync` | start a sync now → `202` |
| POST | `/api/v1/accounts/{id}/clear-cooldown` | → `AccountView` |

`{id}` is URL-encoded (Audible ids contain `:`).

### Sign-in flows

All login sessions live in memory for 15 minutes.

| Method | Path | Body → Response |
|---|---|---|
| POST | `/api/v1/login/audible/device-code` | `{locale}` → `DeviceCodeSession` |
| GET | `/api/v1/login/audible/device-code/{sessionId}` | → `DeviceCodeSession` (poll every 3–5 s) |
| DELETE | `/api/v1/login/audible/device-code/{sessionId}` | cancel → `204` |
| POST | `/api/v1/login/audible/external` | `{locale}` → `{ sessionId, url }` |
| POST | `/api/v1/login/audible/external/{sessionId}` | `{ responseUrl }` → `AccountView` |
| POST | `/api/v1/login/credentials` | `{provider: 'librofm'\|'chirp', email, password}` → `AccountView` |
| POST | `/api/v1/login/import` | `{ accountJson: string }` (an app-exported Audible `Account`) → `AccountView` |

```ts
type DeviceCodeSession = {
  sessionId: string;
  status: 'pending' | 'registered' | 'expired' | 'error';
  publicCode: string;            // show big: "ABC12"
  verificationUrl: string;       // https://www.amazon.{tld}/code
  expiresAt: string;
  account: AccountView | null;   // when registered
  message: string | null;        // when error
};
```

A new account triggers an immediate first sync (its baseline).

**Re-authentication.** Signing in again to an account that already exists (same
`account_id`) replaces its credentials in place and keeps its settings, history and
download records; the response is the updated `AccountView`. That is how an account with
`authStatus: 'expired'` is repaired — the UI offers "Sign in again" with the same flow.

## Books

```ts
type BookStatus = 'downloaded' | 'queued' | 'downloading' | 'wanted' | 'cooldown' | 'failed' | 'skipped';
type BookView = {
  id: string;                 // Books.audible_product_id (asin / chirp_<id> / isbn)
  provider: ProviderId;
  accounts: string[];         // owning account ids
  title: string; subtitle: string | null;
  authors: string[]; narrators: string[];
  series: string | null; seriesSequence: string | null;
  coverUrl: string | null;
  lengthMinutes: number;
  releaseDate: string | null; purchaseDate: string | null; addedAt: string;
  status: BookStatus;
  outputPath: string | null;  // relative to libraryDir
  format: 'm4b' | 'mp3' | null;
  downloadedAt: string | null;
  lastError: string | null;
  backoffUntil: string | null;
};
```

| Method | Path | Notes |
|---|---|---|
| GET | `/api/v1/books` | paged; filters `search`, `provider`, `account`, `status` (comma list); `sort` = `title`\|`author`\|`added`\|`purchased`\|`released` with `dir` = `asc`\|`desc` (default `added desc`) |
| GET | `/api/v1/books/{id}` | → `BookView` + `{ description: string, history: HistoryItem[] }` |
| POST | `/api/v1/books/{id}/download` | `{force?: boolean}` → `QueueItem` (manual priority). `409 account_cooldown` if the account is cooling down and not forced |
| POST | `/api/v1/books/download` | `{ids: string[]}` → `{ queued: number, skipped: { id, reason }[] }`, manual priority |

Manual items skip the Audible spacing and title backoff but **still count against and
wait for the daily licence cap** — otherwise a bulk "download selected" would walk straight
into the throttle. `force: true` (single book only) bypasses the cap and an account
cooldown. To fetch a whole backlog, enable back-fill.
| POST | `/api/v1/books/{id}/skip` | mark as not wanted (excluded from back-fill) → `BookView` |
| POST | `/api/v1/books/{id}/unskip` | → `BookView` |
| POST | `/api/v1/books/{id}/forget` | forget the download record (files untouched) → `BookView` |
| GET | `/api/v1/books/{id}/cover` | proxies + caches the cover image (so the UI works offline from the store CDN) |

`GET /api/v1/wanted` = `/books?status=wanted` with the same paging, plus
`{ "estimate": { "perDay": number, "daysRemaining": number | null } }` at top level for
the Audible back-fill.

## Queue (Activity)

```ts
type QueueState = 'queued' | 'waiting' | 'planning' | 'running' | 'completed' | 'failed' | 'cancelled';
type QueueItem = {
  id: string; bookId: string; accountId: string; provider: ProviderId;
  title: string; authors: string[];
  priority: 'manual' | 'new' | 'backfill';
  state: QueueState;
  waitingReason: string | null;   // "Licence budget: next slot 14:20", "Account cooldown until …"
  notBefore: string | null;
  stage: 'downloading' | 'decrypting' | 'extracting' | 'joining' | 'validating' | 'encoding' | 'finalizing' | null;
  part: number | null; parts: number | null;
  bytesDone: number | null; bytesTotal: number | null; bytesPerSec: number | null;
  progress: number | null;        // 0..1 overall estimate
  attempts: number;
  error: string | null;
  createdAt: string; startedAt: string | null; finishedAt: string | null;
};
```

| Method | Path | Notes |
|---|---|---|
| GET | `/api/v1/queue` | active items (not completed/cancelled), ordered as the worker would take them |
| DELETE | `/api/v1/queue/{id}` | cancel (running items stop; partial files kept for a retry) → `204` |
| POST | `/api/v1/queue/{id}/retry` | failed/cancelled → queued → `QueueItem` |
| POST | `/api/v1/queue/{id}/move` | `{priority}` → `QueueItem` |

## History

```ts
type HistoryEvent = 'bookAdded' | 'grab' | 'download' | 'downloadFailed' | 'licenceDenied'
  | 'licenceThrottled' | 'syncCompleted' | 'syncFailed' | 'accountAdded' | 'accountRemoved' | 'authExpired';
type HistoryItem = {
  id: number; time: string; event: HistoryEvent;
  accountId: string | null; bookId: string | null; title: string | null;
  message: string; data: Record<string, unknown> | null;
};
```

`GET /api/v1/history` — paged, filters `event` (comma list), `account`, `book`. Newest first.

## Settings

`GET /api/v1/settings` → `Settings`; `PUT /api/v1/settings` with the full object →
`Settings` (validated; `400 invalid_input` names the field).

```ts
type Settings = {
  general: { instanceName: string; apiKey: string /* read-only */; logLevel: 'debug'|'info'|'warn' };
  media: {
    libraryDir: string;                 // read-only (env)
    folderTemplate: string;             // "{author}/{series}/{seriesSequence} - {title}"
    noSeriesFolderTemplate: string;     // "{author}/{title}"
    format: 'm4b' | 'mp3';
    writeSidecars: boolean;
  };
  sync: { intervalHours: number /* 1..168 */; autoDownloadNew: boolean };
  backfill: {
    enabled: boolean;
    order: 'newest_first' | 'oldest_first';
    windowStart: string | null; windowEnd: string | null;   // "HH:MM" local, both or neither
  };
  audible: {
    dailyLicenceCap: number;            // 1..200
    spacingMinMinutes: number; spacingMaxMinutes: number;
    throttleCooldownHours: number;      // base, doubles; default 24
  };
  downloads: { concurrency: number /* 1..4 */ };
};
```

`POST /api/v1/settings/api-key/regenerate` → `{ apiKey }`.

## Notifications (Connect)

```ts
type NotificationKind = 'webhook' | 'apprise' | 'audiobookshelf';
type Notification = {
  id: number; kind: NotificationKind; name: string; enabled: boolean;
  events: NotificationEvent[] | null;   // null for audiobookshelf (fires on 'download')
  config: WebhookConfig | AppriseConfig | AbsConfig;
  lastResult: { ok: boolean; time: string; message: string | null } | null;
};
type NotificationEvent = HistoryEvent | 'health' | 'healthRestored';   // 'test' is sent only by the test endpoint
type WebhookConfig = { url: string; method: 'POST' | 'PUT'; headers: { key: string; value: string }[]; username?: string; password?: string };
type AppriseConfig = { serverUrl: string; configKey?: string; statelessUrls?: string; tags?: string };
type AbsConfig = { url: string; apiKey: string; libraryId: string; pathPrefix?: string };
```

| Method | Path | Notes |
|---|---|---|
| GET | `/api/v1/notifications` | → `Notification[]` (secrets returned masked as `"********"`; sending the mask back keeps the stored value) |
| POST | `/api/v1/notifications` | create → `Notification` |
| PUT | `/api/v1/notifications/{id}` | → `Notification` |
| DELETE | `/api/v1/notifications/{id}` | → `204` |
| POST | `/api/v1/notifications/test` | body = a `Notification` (unsaved allowed) → `{ ok, message }` |
| POST | `/api/v1/notifications/abs/libraries` | `{url, apiKey, id?}` → `{ id, name, folders: string[] }[]`. With `id` of a saved Audiobookshelf connection, a masked `apiKey` (`"********"`) means "use the stored key" |

Webhook payload: `{ eventType, instanceName, applicationUrl, time, account?: {id, provider, name}, book?: {id, title, authors, series, seriesSequence, outputPath}, message }`.
