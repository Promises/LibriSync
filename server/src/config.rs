//! Process configuration from the environment (see DESIGN.md "Runtime layout").

use std::net::IpAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub config_dir: PathBuf,
    pub library_dir: PathBuf,
    pub port: u16,
    pub bind: IpAddr,
    /// `LIBRISYNC_LOG` when explicitly set; it then wins over the UI's log level.
    pub log_override: Option<String>,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    /// Public URL of this instance for notification payloads (`applicationUrl`).
    pub application_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let port = match var("LIBRISYNC_PORT") {
            Some(p) => p.parse().with_context(|| format!("LIBRISYNC_PORT is not a port: {p}"))?,
            None => 8383,
        };
        let bind = match var("LIBRISYNC_BIND") {
            Some(b) => b.parse().with_context(|| format!("LIBRISYNC_BIND is not an IP address: {b}"))?,
            None => IpAddr::from([0, 0, 0, 0]),
        };
        Ok(Self {
            config_dir: PathBuf::from(var("LIBRISYNC_CONFIG_DIR").unwrap_or_else(|| "/config".into())),
            library_dir: PathBuf::from(
                var("LIBRISYNC_LIBRARY_DIR").unwrap_or_else(|| "/audiobooks".into()),
            ),
            port,
            bind,
            log_override: var("LIBRISYNC_LOG"),
            ffmpeg: PathBuf::from(var("FFMPEG_PATH").unwrap_or_else(|| "ffmpeg".into())),
            ffprobe: PathBuf::from(var("FFPROBE_PATH").unwrap_or_else(|| "ffprobe".into())),
            application_url: var("LIBRISYNC_APPLICATION_URL").unwrap_or_default(),
        })
    }

    pub fn core_db_path(&self) -> PathBuf {
        self.config_dir.join("librisync.db")
    }
    pub fn server_db_path(&self) -> PathBuf {
        self.config_dir.join("server.db")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.config_dir.join("logs")
    }
    pub fn work_dir(&self) -> PathBuf {
        self.config_dir.join("work")
    }
    pub fn cover_cache_dir(&self) -> PathBuf {
        self.config_dir.join("cache").join("covers")
    }

    /// The address the healthcheck subcommand should reach: loopback when bound to all
    /// interfaces, the bind address otherwise.
    pub fn local_url(&self) -> String {
        let host = if self.bind.is_unspecified() {
            match self.bind {
                IpAddr::V4(_) => "127.0.0.1".to_string(),
                IpAddr::V6(_) => "[::1]".to_string(),
            }
        } else {
            match self.bind {
                IpAddr::V4(a) => a.to_string(),
                IpAddr::V6(a) => format!("[{a}]"),
            }
        };
        format!("http://{host}:{}", self.port)
    }
}
