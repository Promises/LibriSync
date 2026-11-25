//! Test fixtures: an [`App`] on temporary databases with a scripted executor.

use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rust_core::pipeline::{OutputFormat, PipelineError, PipelineJob, PipelineOutput, ProgressFn};
use rust_core::providers::{DownloadPlan, ProviderId};
use rust_core::storage::Database;
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::executor::{Executor, PlanFailure};
use crate::logging::LogBuffer;
use crate::state::{App, AppState, Deps};

/// Plans and runs from scripts; records every plan request.
#[derive(Default)]
pub struct FakeExecutor {
    pub plans: Mutex<VecDeque<Result<DownloadPlan, PlanFailure>>>,
    pub runs: Mutex<VecDeque<Result<(), PipelineError>>>,
    pub plan_calls: Mutex<Vec<String>>,
}

pub fn empty_plan() -> DownloadPlan {
    DownloadPlan { parts: Vec::new(), embed_metadata: false, chapters: Vec::new(), join_parts: false }
}

#[async_trait]
impl Executor for FakeExecutor {
    async fn plan(
        &self,
        _core: &Database,
        _provider: ProviderId,
        _account: &serde_json::Value,
        book_id: &str,
        _options: &serde_json::Value,
    ) -> Result<DownloadPlan, PlanFailure> {
        if let Ok(mut c) = self.plan_calls.lock() {
            c.push(book_id.to_string());
        }
        self.plans.lock().ok().and_then(|mut p| p.pop_front()).unwrap_or_else(|| Ok(empty_plan()))
    }

    async fn run(&self, job: PipelineJob, _p: ProgressFn, _c: CancellationToken) -> Result<PipelineOutput, PipelineError> {
        let next = self.runs.lock().ok().and_then(|mut r| r.pop_front()).unwrap_or(Ok(()));
        next.map(|_| PipelineOutput {
            audio_files: vec![job.dest_dir.join(format!("{}.m4b", job.file_stem))],
            dest_dir: job.dest_dir,
            total_bytes: 1234,
            duration_ms: Some(60_000),
            chapter_count: 1,
            format: OutputFormat::M4b,
        })
    }

    async fn check_tools(&self) -> Result<String, String> {
        Ok("ffmpeg test".into())
    }
}

pub async fn test_app_with(executor: Arc<FakeExecutor>) -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).expect("library dir");
    let cfg = Config {
        config_dir: dir.path().join("config"),
        library_dir: library,
        port: 0,
        bind: IpAddr::from([127, 0, 0, 1]),
        log_override: None,
        ffmpeg: "ffmpeg".into(),
        ffprobe: "ffprobe".into(),
        application_url: String::new(),
    };
    let app = App::build(Deps {
        cfg,
        executor,
        logs: LogBuffer::default(),
        set_log_level: Arc::new(|_: &str| {}),
    })
    .await
    .expect("app");
    (app, dir)
}

pub async fn test_app() -> (AppState, tempfile::TempDir) {
    test_app_with(Arc::new(FakeExecutor::default())).await
}
