//! Multi-provider abstraction.
//!
//! LibriSync supports several audiobook sources (Audible, LibriVox, Libro.fm, …).
//! Rather than branching on the provider throughout the codebase, each provider
//! implements the [`Provider`] trait and is reached through the [`AnyProvider`]
//! enum-dispatch registry keyed by [`ProviderId`].
//!
//! The keystone output is a typed [`DownloadPlan`]: the core produces a plan of
//! typed [`DownloadPart`]s per book, and the (Kotlin) download engine executes it
//! without knowing which provider it came from. Adding a DRM-free provider then
//! needs no engine changes — see the plan in `greedy-orbiting-cerf.md`.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::api::library::SyncStats;
use crate::storage::Database;
use crate::{LibationError, Result};

pub mod audible;
pub mod http;
pub mod librofm;

/// Stable identifier for a provider. Serializes to a lowercase string that also
/// doubles as the `Books.source` / `Accounts.provider` column value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Audible,
    Librivox,
    Librofm,
}

impl ProviderId {
    /// The canonical string form, matching the DB `source`/`provider` values.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderId::Audible => "audible",
            ProviderId::Librivox => "librivox",
            ProviderId::Librofm => "librofm",
        }
    }

    /// Parse a provider id from its string form. Unknown ids are an error
    /// (callers default to `audible` at the DB layer, not here).
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "audible" => Ok(ProviderId::Audible),
            "librivox" => Ok(ProviderId::Librivox),
            "librofm" => Ok(ProviderId::Librofm),
            other => Err(LibationError::InvalidInput(format!(
                "Unknown provider id: {other}"
            ))),
        }
    }
}

/// HTTP headers to send when fetching a download part (auth, user-agent, …).
pub type Headers = BTreeMap<String, String>;

/// Opaque, per-provider credential blob persisted in `Accounts.identity_json`.
/// Audible = the existing `Identity`/`Account` JSON; Libro.fm = `{access_token,…}`;
/// LibriVox = null (no account).
pub type CredentialBlob = serde_json::Value;

/// Raw login fields collected by the UI (e.g. `{username, password}`), passed
/// opaquely to a provider's [`Provider::login`].
pub type LoginInput = serde_json::Value;

/// Per-download options from the caller, e.g. the Libro.fm "download format"
/// setting (`{"format":"parts"|"m4b"}`). Opaque JSON like [`LoginInput`] so a new
/// provider can take its own settings without touching the bridge.
pub type PlanOptions = serde_json::Value;

/// One chapter marker for a downloaded book (used when the source supplies them,
/// e.g. Libro.fm `tracks[]`). Times are milliseconds from the start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanChapter {
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// A single downloadable unit and how to turn it into a finished audio file.
/// The download engine dispatches on `kind`; a new DRM-free provider only ever
/// produces `Plain`/`Zip`, so it needs no engine changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DownloadPart {
    /// A plain audio file — download and copy as-is (LibriVox mp3, Libro.fm
    /// packaged m4b).
    Plain {
        url: String,
        #[serde(default)]
        headers: Headers,
        filename: String,
    },
    /// An AAXC-encrypted file — download, then FFmpeg-decrypt with `key`/`iv`
    /// (Audible). `key`/`iv` are hex strings.
    Aaxc {
        url: String,
        #[serde(default)]
        headers: Headers,
        key: String,
        iv: String,
        filename: String,
    },
    /// A ZIP archive of audio files — download and extract into the book folder
    /// (LibriVox zip, Libro.fm manifest parts).
    Zip {
        url: String,
        #[serde(default)]
        headers: Headers,
    },
}

/// A provider-agnostic download plan for one book. The engine downloads each
/// part, applies its per-`kind` post-processing (decrypt/unzip), then copies to
/// the user's chosen directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadPlan {
    pub parts: Vec<DownloadPart>,
    /// Whether to embed metadata/cover during conversion (Audible = true;
    /// DRM-free providers usually false — files already carry tags).
    #[serde(default)]
    pub embed_metadata: bool,
    /// Chapter markers, when the provider supplies them.
    #[serde(default)]
    pub chapters: Vec<PlanChapter>,
}

/// The behaviour every audiobook provider implements. Not object-safe (async
/// methods) — dispatched through [`AnyProvider`], not `dyn Provider`.
#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;

    /// Turn raw login fields into a persisted [`CredentialBlob`]. Providers that
    /// authenticate through a dedicated flow (Audible OAuth) return an error here.
    async fn login(&self, input: LoginInput) -> Result<CredentialBlob>;

    /// Refresh an expiring credential. `Ok(None)` = nothing to refresh.
    async fn refresh(&self, creds: &CredentialBlob) -> Result<Option<CredentialBlob>>;

    /// Sync one page of the owned library into the DB (books tagged with this
    /// provider's `source`). Reuses the existing [`SyncStats`] (carries `has_more`).
    async fn sync_library_page(
        &self,
        db: &Database,
        creds: &CredentialBlob,
        page: i32,
    ) -> Result<SyncStats>;

    /// Produce the typed [`DownloadPlan`] for one owned book (`item_ref` = the
    /// provider's item id: asin / isbn / librivox id). `options` carries any
    /// provider-specific download settings; providers without any ignore it.
    async fn download_plan(
        &self,
        db: &Database,
        creds: &CredentialBlob,
        item_ref: &str,
        options: &PlanOptions,
    ) -> Result<DownloadPlan>;
}

/// Enum-dispatch registry over the compile-time set of providers. This is the
/// single place that maps a [`ProviderId`] to a concrete implementation.
pub enum AnyProvider {
    Audible(audible::AudibleProvider),
    Librofm(librofm::LibrofmProvider),
    // Librivox is folded in during Phase 5 cleanup.
}

impl AnyProvider {
    /// Resolve a provider by id. Returns an error for providers not yet wired
    /// into the registry.
    pub fn get(id: ProviderId) -> Result<Self> {
        match id {
            ProviderId::Audible => Ok(AnyProvider::Audible(audible::AudibleProvider)),
            ProviderId::Librofm => Ok(AnyProvider::Librofm(librofm::LibrofmProvider)),
            other => Err(LibationError::InvalidInput(format!(
                "Provider not yet implemented: {}",
                other.as_str()
            ))),
        }
    }

    /// Borrow the inner provider as the trait, so callers can invoke trait
    /// methods without matching on the variant.
    pub fn as_provider(&self) -> &dyn Provider {
        match self {
            AnyProvider::Audible(p) => p,
            AnyProvider::Librofm(p) => p,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_id_string_roundtrip() {
        for id in [ProviderId::Audible, ProviderId::Librivox, ProviderId::Librofm] {
            assert_eq!(ProviderId::from_str(id.as_str()).unwrap(), id);
        }
        assert!(ProviderId::from_str("nope").is_err());
    }

    #[test]
    fn provider_id_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&ProviderId::Librofm).unwrap(),
            "\"librofm\""
        );
    }

    #[test]
    fn download_part_tagged_serde() {
        let part = DownloadPart::Aaxc {
            url: "https://x/y.aaxc".into(),
            headers: Headers::new(),
            key: "abcd".into(),
            iv: "ef01".into(),
            filename: "B01.aaxc".into(),
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["kind"], "aaxc");
        let round: DownloadPart = serde_json::from_value(json).unwrap();
        matches!(round, DownloadPart::Aaxc { .. });
    }

    #[test]
    fn registry_resolves_implemented_and_rejects_unimplemented() {
        assert!(AnyProvider::get(ProviderId::Audible).is_ok());
        assert!(AnyProvider::get(ProviderId::Librofm).is_ok());
        // Librivox is folded in during Phase 5 cleanup.
        assert!(AnyProvider::get(ProviderId::Librivox).is_err());
        assert_eq!(
            AnyProvider::get(ProviderId::Librofm)
                .unwrap()
                .as_provider()
                .id(),
            ProviderId::Librofm
        );
    }
}
