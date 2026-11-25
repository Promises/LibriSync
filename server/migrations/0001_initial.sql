-- LibriSync Server state (server.db). The core DB (librisync.db) is attached as `core`
-- on every connection; table names here must not collide case-insensitively with the
-- core's (Books, Accounts, DownloadTasks, ...).
-- Timestamps are fixed-width RFC 3339 UTC strings ("2026-11-25T21:51:06.123Z").

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- The cookie carries a random token; only its SHA-256 is stored.
CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

-- Per-account server state; core.Accounts holds the credentials.
CREATE TABLE account_state (
    account_id TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 1,
    auto_download_new INTEGER NOT NULL DEFAULT 1,
    backfill INTEGER NOT NULL DEFAULT 1,
    -- Set by the first fully successful sync; books found before it are back-fill.
    baseline_at TEXT,
    last_sync TEXT,
    next_sync TEXT,
    last_sync_ok INTEGER,
    last_sync_added INTEGER,
    last_sync_updated INTEGER,
    last_sync_total INTEGER,
    last_sync_message TEXT,
    auth_status TEXT NOT NULL DEFAULT 'unknown',
    -- Audible licence throttle cooldown.
    cooldown_until TEXT,
    cooldown_level INTEGER NOT NULL DEFAULT 0,
    cooldown_reason TEXT,
    -- Earliest next back-fill licence request (random spacing).
    next_licence_at TEXT
);

-- Every Audible licence request, granted or not (the rolling daily cap counts these).
CREATE TABLE licence_requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL,
    book_id TEXT NOT NULL,
    time TEXT NOT NULL,
    outcome TEXT NOT NULL DEFAULT 'pending'
);
CREATE INDEX idx_licence_requests_account_time ON licence_requests(account_id, time);

-- Per-title licence denial backoff. Rows outlive `until` so the level keeps doubling.
CREATE TABLE title_backoff (
    book_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    level INTEGER NOT NULL,
    until TEXT NOT NULL,
    reason TEXT
);

CREATE TABLE skips (
    book_id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL
);

CREATE TABLE queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    book_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    title TEXT NOT NULL,
    authors TEXT NOT NULL DEFAULT '[]',
    priority INTEGER NOT NULL,
    state TEXT NOT NULL,
    force INTEGER NOT NULL DEFAULT 0,
    waiting_reason TEXT,
    not_before TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT
);
-- A book has at most one live (or failed) queue row; completed/cancelled rows are history.
CREATE UNIQUE INDEX idx_queue_book_slot ON queue(book_id)
    WHERE state IN ('queued', 'waiting', 'planning', 'running', 'failed');
CREATE INDEX idx_queue_state ON queue(state, priority, created_at);
CREATE INDEX idx_queue_account ON queue(account_id, state);

-- Books this server has imported into the library.
CREATE TABLE downloads (
    book_id TEXT PRIMARY KEY,
    account_id TEXT,
    provider TEXT NOT NULL,
    output_path TEXT NOT NULL,
    format TEXT NOT NULL,
    total_bytes INTEGER,
    audio_files INTEGER,
    downloaded_at TEXT NOT NULL
);

CREATE TABLE history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    time TEXT NOT NULL,
    event TEXT NOT NULL,
    account_id TEXT,
    book_id TEXT,
    title TEXT,
    message TEXT NOT NULL,
    data TEXT
);
CREATE INDEX idx_history_time ON history(time);
CREATE INDEX idx_history_book ON history(book_id, time);
CREATE INDEX idx_history_account ON history(account_id, time);

CREATE TABLE notifications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    events TEXT,
    config TEXT NOT NULL,
    last_ok INTEGER,
    last_time TEXT,
    last_message TEXT
);

CREATE TABLE task_runs (
    name TEXT PRIMARY KEY,
    last_run TEXT
);
