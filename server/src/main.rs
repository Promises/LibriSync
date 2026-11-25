//! LibriSync Server: a self-hosted daemon that syncs purchased audiobook libraries
//! (Audible, Libro.fm, Chirp), downloads new purchases, back-fills the rest at a pace
//! Audible tolerates, and hands books to Audiobookshelf. See DESIGN.md and API.md.

mod accounts;
mod api;
mod auth;
mod backfill;
mod books;
mod config;
mod covers;
mod db;
mod error;
mod events;
mod executor;
mod health;
mod history;
mod logging;
mod login;
mod notify;
mod pacing;
mod queue;
mod scheduler;
mod settings;
mod state;
mod sync;
mod template;
mod time;
mod web;

#[cfg(test)]
mod test_support;

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use rust_core::pipeline::MediaTools;

use crate::config::Config;
use crate::executor::CoreExecutor;
use crate::state::{App, AppState, Deps};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Could not start the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match args.first().map(String::as_str) {
        None | Some("serve") => match runtime.block_on(serve()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("librisync-server: {e:#}");
                ExitCode::FAILURE
            }
        },
        Some("healthcheck") => runtime.block_on(healthcheck()),
        Some("--version" | "-V" | "version") => {
            println!("librisync-server {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("Unknown command '{other}'. Usage: librisync-server [serve|healthcheck|--version]");
            ExitCode::from(2)
        }
    }
}

/// `librisync-server healthcheck`: exit 0 if `/ping` answers, 1 otherwise (Docker
/// HEALTHCHECK without curl in the image).
async fn healthcheck() -> ExitCode {
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e:#}");
            return ExitCode::FAILURE;
        }
    };
    let client = match reqwest::Client::builder().timeout(Duration::from_secs(5)).build() {
        Ok(c) => c,
        Err(_) => return ExitCode::FAILURE,
    };
    match client.get(format!("{}/ping", cfg.local_url())).send().await {
        Ok(r) if r.status().is_success() => ExitCode::SUCCESS,
        Ok(r) => {
            eprintln!("unhealthy: HTTP {}", r.status());
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("unhealthy: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> Result<()> {
    let cfg = Config::from_env()?;
    // Bootstrap logging at the env level (or info); the stored setting is applied
    // once the database is open.
    let logging = logging::init(&cfg.logs_dir(), cfg.log_override.as_deref(), "info");
    logging::install_panic_hook();
    tracing::info!("LibriSync Server {} starting", env!("CARGO_PKG_VERSION"));
    tracing::info!("Config: {}  Library: {}", cfg.config_dir.display(), cfg.library_dir.display());

    let tools = MediaTools { ffmpeg: cfg.ffmpeg.clone(), ffprobe: cfg.ffprobe.clone() };
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .build()
        .context("building the download HTTP client")?;
    let executor = Arc::new(CoreExecutor::new(tools, http));

    let app = App::build(Deps {
        cfg: cfg.clone(),
        executor,
        logs: logging.buffer.clone(),
        set_log_level: logging.set_level.clone(),
    })
    .await?;
    (app.set_log_level)(app.settings().general.log_level.as_str());

    if let Err(e) = std::fs::create_dir_all(cfg.work_dir()) {
        tracing::warn!("Could not create {}: {e}", cfg.work_dir().display());
    }
    let recovered = queue::dispatch::recover(&app.db).await?;
    if recovered > 0 {
        tracing::info!("Re-queued {recovered} download(s) interrupted by the last shutdown");
    }

    start_background(&app).await;

    let addr = SocketAddr::new(cfg.bind, cfg.port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!("Listening on http://{addr}");

    let shutdown = app.shutdown.clone();
    let router = api::router(app.clone());
    let server = axum::serve(listener, router).with_graceful_shutdown(async move {
        wait_for_signal().await;
        shutdown.cancel();
    });
    server.await.context("HTTP server")?;

    tracing::info!("Shutting down");
    app.logins.cancel_all();
    app.queue.cancel_all();
    // Give workers a moment to put their items back in the queue.
    for _ in 0..50 {
        if app.queue.running_count() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

async fn start_background(app: &AppState) {
    // ffmpeg check and client profiles first, off the startup path.
    {
        let app = app.clone();
        tokio::spawn(async move {
            let status = app.executor.check_tools().await;
            if let Ok(v) = &status {
                tracing::info!("ffmpeg: {v}");
            }
            app.set_tools_status(status);
        });
    }
    tokio::spawn(queue::dispatch::run(app.clone()));
    tokio::spawn(health::monitor(app.clone()));
    scheduler::start(app).await;
}

async fn wait_for_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}
