//! The seam between the queue and the outside world: getting a provider's download
//! plan (for Audible, the licence request) and running the download pipeline.
//!
//! The queue only talks to [`Executor`], so its pacing and state transitions are
//! tested with a fake; [`CoreExecutor`] is the real thing on top of rust-core.

use std::sync::Arc;

use async_trait::async_trait;
use rust_core::pipeline::{self, FailureKind, MediaTools, PipelineError, PipelineJob, PipelineOutput, ProgressFn};
use rust_core::providers::{AnyProvider, DownloadPart, DownloadPlan, ProviderId};
use rust_core::storage::Database;
use rust_core::LibationError;
use tokio_util::sync::CancellationToken;

/// Why a plan could not be produced, as far as pacing is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanFailureKind {
    /// Audible `CustomerThrottled`: cool the whole account down.
    Throttled,
    /// Any other licence refusal: back this title off.
    Denied,
    /// The stored credential no longer works; the user must sign in again.
    AuthExpired,
    /// A network error or a 5xx / 429 from the store: the service is unavailable.
    /// For Audible this holds the whole customer briefly (DESIGN.md).
    Unavailable,
    /// Anything else (unexpected response): ordinary retry.
    Other,
}

#[derive(Debug, Clone)]
pub struct PlanFailure {
    pub kind: PlanFailureKind,
    pub message: String,
}

#[async_trait]
pub trait Executor: Send + Sync + 'static {
    /// The provider's download plan for `book_id`. `account` is the stored account JSON.
    async fn plan(
        &self,
        core: &Database,
        provider: ProviderId,
        account: &serde_json::Value,
        book_id: &str,
        options: &serde_json::Value,
    ) -> Result<DownloadPlan, PlanFailure>;

    /// Execute a plan into its destination folder.
    async fn run(
        &self,
        job: PipelineJob,
        on_progress: ProgressFn,
        cancel: CancellationToken,
    ) -> Result<PipelineOutput, PipelineError>;

    /// `ffmpeg` version string, or why it is unusable.
    async fn check_tools(&self) -> Result<String, String>;
}

/// Map a core error to the pacing class by its stable machine code.
pub fn classify(err: &LibationError) -> PlanFailureKind {
    match err.error_code() {
        "license_throttled" => PlanFailureKind::Throttled,
        "license_denied" => PlanFailureKind::Denied,
        "auth_expired" => PlanFailureKind::AuthExpired,
        "network" => PlanFailureKind::Unavailable,
        _ => PlanFailureKind::Other,
    }
}

/// rust-core's plan + pipeline, run with the system `ffmpeg`.
pub struct CoreExecutor {
    tools: MediaTools,
    http: reqwest::Client,
}

impl CoreExecutor {
    pub fn new(tools: MediaTools, http: reqwest::Client) -> Self {
        Self { tools, http }
    }
}

/// Audible's plan leaves part headers empty; its CDN expects the download User-Agent
/// the app sends (from the published client profiles).
fn add_audible_download_headers(plan: &mut DownloadPlan) {
    let ua = rust_core::client_profiles::current().audible_download_user_agent;
    for part in &mut plan.parts {
        let headers = match part {
            DownloadPart::Plain { headers, .. }
            | DownloadPart::Aaxc { headers, .. }
            | DownloadPart::Aax { headers, .. }
            | DownloadPart::Zip { headers, .. } => headers,
        };
        let has_ua = headers.keys().any(|k| k.eq_ignore_ascii_case("user-agent"));
        if !has_ua {
            headers.insert("User-Agent".to_string(), ua.clone());
        }
    }
}

#[async_trait]
impl Executor for CoreExecutor {
    async fn plan(
        &self,
        core: &Database,
        provider: ProviderId,
        account: &serde_json::Value,
        book_id: &str,
        options: &serde_json::Value,
    ) -> Result<DownloadPlan, PlanFailure> {
        let p = AnyProvider::get(provider).map_err(|e| PlanFailure {
            kind: PlanFailureKind::Other,
            message: e.to_string(),
        })?;
        match p.as_provider().download_plan(core, account, book_id, options).await {
            Ok(mut plan) => {
                if provider == ProviderId::Audible {
                    add_audible_download_headers(&mut plan);
                }
                Ok(plan)
            }
            Err(e) => Err(PlanFailure { kind: classify(&e), message: e.to_string() }),
        }
    }

    async fn run(
        &self,
        job: PipelineJob,
        on_progress: ProgressFn,
        cancel: CancellationToken,
    ) -> Result<PipelineOutput, PipelineError> {
        let tools = self.tools.clone();
        let http = self.http.clone();
        // A separate task so a panic inside the pipeline fails this book, not the worker.
        // Reported as `InvalidPlan`: the book fails without a retry (a crash on this
        // input would recur, and each retry re-plans, spending a licence), and — unlike
        // `Io` — it doesn't pause the whole queue as a disk or folder problem would.
        match tokio::spawn(async move { pipeline::run(job, &tools, &http, on_progress, cancel).await }).await {
            Ok(result) => result,
            Err(join) => Err(PipelineError::new(
                FailureKind::InvalidPlan,
                format!("The download pipeline crashed: {}", panic_text(join)),
            )),
        }
    }

    async fn check_tools(&self) -> Result<String, String> {
        let tools = self.tools.clone();
        match tokio::spawn(async move { tools.check().await }).await {
            Ok(Ok(version)) => Ok(version),
            Ok(Err(e)) => Err(e.message),
            Err(join) => Err(format!("ffmpeg check crashed: {}", panic_text(join))),
        }
    }
}

fn panic_text(e: tokio::task::JoinError) -> String {
    if e.is_cancelled() {
        return "cancelled".into();
    }
    match e.try_into_panic() {
        Ok(payload) => payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into()),
        Err(_) => "unknown".into(),
    }
}

pub type SharedExecutor = Arc<dyn Executor>;
