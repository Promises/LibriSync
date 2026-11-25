//! Shared application state.

use std::sync::{Arc, RwLock, Weak};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rust_core::storage::Database;
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::events::Hub;
use crate::executor::SharedExecutor;
use crate::logging::{LevelSetter, LogBuffer};
use crate::login;
use crate::notify::Notifier;
use crate::queue::Runtime;
use crate::scheduler::Tasks;
use crate::settings::{self, Settings};
use crate::sync;

pub type AppState = Arc<App>;

pub struct App {
    pub cfg: Config,
    /// The rust-core database (`librisync.db`). Write core tables only through it.
    pub core: Arc<Database>,
    /// `server.db`, with the core DB attached as `core` (read-only use).
    pub db: SqlitePool,
    pub hub: Hub,
    pub http: reqwest::Client,
    pub logs: LogBuffer,
    pub set_log_level: LevelSetter,
    pub started: DateTime<Utc>,
    pub logins: login::Sessions,
    pub syncs: sync::Running,
    pub token_locks: crate::accounts::TokenLocks,
    pub queue: Runtime,
    pub tasks: Tasks,
    pub notifier: Notifier,
    pub executor: SharedExecutor,
    pub shutdown: CancellationToken,
    settings: RwLock<Settings>,
    api_key: RwLock<String>,
    tools: RwLock<Option<Result<String, String>>>,
    me: Weak<App>,
}

pub struct Deps {
    pub cfg: Config,
    pub executor: SharedExecutor,
    pub logs: LogBuffer,
    pub set_log_level: LevelSetter,
}

impl App {
    /// Open both databases, load settings and the API key.
    pub async fn build(deps: Deps) -> Result<AppState> {
        let cfg = deps.cfg;
        std::fs::create_dir_all(&cfg.config_dir)
            .with_context(|| format!("creating {}", cfg.config_dir.display()))?;
        let core = Database::new(cfg.core_db_path())
            .await
            .with_context(|| format!("opening {}", cfg.core_db_path().display()))?;
        let db = crate::db::open(&cfg.server_db_path(), &cfg.core_db_path()).await?;
        let mut settings = settings::load(&db).await?;
        let api_key = settings::load_or_create_api_key(&db).await?;
        settings.media.library_dir = cfg.library_dir.display().to_string();
        let http = reqwest::Client::builder()
            .user_agent(concat!("LibriSync-Server/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(20))
            .build()
            .context("building the HTTP client")?;

        Ok(Arc::new_cyclic(|me| App {
            core: Arc::new(core),
            db,
            hub: Hub::default(),
            notifier: Notifier::new(http.clone()),
            http,
            logs: deps.logs,
            set_log_level: deps.set_log_level,
            started: crate::time::now(),
            logins: login::Sessions::default(),
            syncs: sync::Running::default(),
            token_locks: Default::default(),
            queue: Runtime::default(),
            tasks: Tasks::new(),
            executor: deps.executor,
            shutdown: CancellationToken::new(),
            settings: RwLock::new(settings),
            api_key: RwLock::new(api_key),
            tools: RwLock::new(None),
            me: me.clone(),
            cfg,
        }))
    }

    /// A strong handle to this app, for spawning background work from `&App`.
    pub fn self_arc(&self) -> Option<AppState> {
        self.me.upgrade()
    }

    pub fn settings(&self) -> Settings {
        let mut s = self.settings.read().map(|s| s.clone()).unwrap_or_default();
        s.general.api_key = self.api_key();
        s.media.library_dir = self.cfg.library_dir.display().to_string();
        s
    }

    pub fn set_settings(&self, s: Settings) {
        if let Ok(mut cur) = self.settings.write() {
            *cur = s;
        }
    }

    pub fn api_key(&self) -> String {
        self.api_key.read().map(|k| k.clone()).unwrap_or_default()
    }

    pub fn set_api_key(&self, key: String) {
        if let Ok(mut k) = self.api_key.write() {
            *k = key;
        }
    }

    /// Result of the last `ffmpeg` check (`None` = not checked yet).
    pub fn tools_status(&self) -> Option<Result<String, String>> {
        self.tools.read().ok().and_then(|t| t.clone())
    }

    pub fn set_tools_status(&self, status: Result<String, String>) {
        if let Err(e) = &status {
            tracing::warn!("ffmpeg check failed: {e}");
        }
        if let Ok(mut t) = self.tools.write() {
            *t = Some(status);
        }
    }
}
