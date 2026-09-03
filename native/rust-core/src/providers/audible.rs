//! Audible provider — a thin adapter over the existing `AudibleClient` / license
//! pipeline. All AAXC/OAuth/Widevine specifics stay in `api::*`; this only maps
//! them onto the [`Provider`] abstraction. Nothing calls it until the generic
//! bridge (Phase 2) routes Audible through the registry.

use async_trait::async_trait;

use crate::api::auth::Account;
use crate::api::client::AudibleClient;
use crate::api::content::{flatten_chapters, DownloadQuality};
use crate::api::license::DownloadLicense;
use crate::api::library::SyncStats;
use crate::storage::Database;
use crate::{LibationError, Result};

use super::{
    CredentialBlob, DownloadPart, DownloadPlan, LoginInput, PlanChapter, PlanOptions, Provider,
    ProviderId,
};

/// Audible provider. Stateless — it rebuilds an [`AudibleClient`] from the stored
/// credential blob per call, exactly as the current JNI entry points do.
pub struct AudibleProvider;

impl AudibleProvider {
    /// Refresh the access token if it's expired/expiring, then return the account.
    /// Threshold 30 min — same as the previous inline JNI behaviour.
    async fn valid_account(db: &Database, creds: &CredentialBlob) -> Result<Account> {
        let account_json = serde_json::to_string(creds)
            .map_err(|e| LibationError::InvalidInput(format!("Invalid Audible account: {e}")))?;
        let refreshed = crate::api::auth::ensure_valid_token(db.pool(), &account_json, 30).await?;
        serde_json::from_str(&refreshed)
            .map_err(|e| LibationError::InvalidInput(format!("Invalid Audible account: {e}")))
    }

    /// Chapter markers for the plan, used by the engine when the user asked for
    /// per-chapter output. The license response carries `chapter_info` for most
    /// titles; when it doesn't, fall back to the content metadata endpoint.
    /// Chapters are optional — a book without them still downloads as one file.
    async fn plan_chapters(
        client: &AudibleClient,
        asin: &str,
        license: &DownloadLicense,
    ) -> Vec<PlanChapter> {
        let info = match license.content_metadata.chapter_info.clone() {
            Some(info) => Some(info),
            None => client
                .get_content_metadata(asin)
                .await
                .ok()
                .and_then(|m| m.chapter_info),
        };

        let Some(info) = info else {
            return Vec::new();
        };

        flatten_chapters(info.chapters, Some(": "))
            .into_iter()
            .filter(|c| c.length_ms > 0)
            .map(|c| {
                let start = c.start_offset_ms.max(0);
                PlanChapter {
                    title: c.title,
                    start_ms: start as u64,
                    end_ms: (start + c.length_ms) as u64,
                }
            })
            .collect()
    }
}

#[async_trait]
impl Provider for AudibleProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Audible
    }

    async fn login(&self, _input: LoginInput) -> Result<CredentialBlob> {
        // Audible authenticates via the dedicated OAuth WebView flow, not the
        // generic credential login. Explicit error so the generic path never
        // silently mishandles Audible.
        Err(LibationError::InvalidInput(
            "Audible uses the OAuth sign-in flow, not generic login".to_string(),
        ))
    }

    async fn refresh(&self, _creds: &CredentialBlob) -> Result<Option<CredentialBlob>> {
        // Token refresh is handled inside the sync/download client today; no
        // separate provider-layer refresh needed yet.
        Ok(None)
    }

    async fn sync_library_page(
        &self,
        db: &Database,
        creds: &CredentialBlob,
        page: i32,
    ) -> Result<SyncStats> {
        // Just-in-time token refresh before the API call (matches the behaviour
        // the JNI sync path had inline).
        let account = Self::valid_account(db, creds).await?;
        let mut client = AudibleClient::new(account.clone())?;
        client.sync_library_page(db, &account, page).await
    }

    async fn download_plan(
        &self,
        db: &Database,
        creds: &CredentialBlob,
        item_ref: &str,
        _options: &PlanOptions,
    ) -> Result<DownloadPlan> {
        let account = Self::valid_account(db, creds).await?;
        let client = AudibleClient::new(account)?;
        let license = client
            .build_download_license(item_ref, DownloadQuality::High, false)
            .await?;

        // Filename extension from the URL, falling back by DRM type (mirrors the
        // logic in the existing nativeGetDownloadLicense).
        let default_ext = if license.drm_type.is_encrypted() {
            "aaxc"
        } else {
            "mp3"
        };
        let ext = license
            .download_url
            .split('?')
            .next()
            .and_then(|p| p.rsplit('/').next())
            .and_then(|n| n.rsplit_once('.').map(|(_, e)| e))
            .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or(default_ext)
            .to_ascii_lowercase();
        let filename = format!("{item_ref}.{ext}");

        // Only encrypted (AAXC) downloads become one big M4B that the engine may
        // need to split; a plain MP3 asset (podcast episode) is already one file.
        let chapters = if license.drm_type.is_encrypted() {
            Self::plan_chapters(&client, item_ref, &license).await
        } else {
            Vec::new()
        };

        let part = if license.drm_type.is_encrypted() {
            let keys = license
                .decryption_keys
                .as_ref()
                .filter(|k| !k.is_empty())
                .ok_or_else(|| {
                    LibationError::InvalidInput("No decryption keys in Audible license".into())
                })?;
            let kd = &keys[0];
            // 4 bytes = activation bytes from the legacy AAX fallback (no IV); 16 = AAXC.
            if kd.key_part_1.len() == 4 {
                return Ok(DownloadPlan {
                    parts: vec![DownloadPart::Aax {
                        url: license.download_url,
                        headers: Default::default(),
                        activation_bytes: hex::encode(&kd.key_part_1),
                        filename: format!("{item_ref}.aax"),
                    }],
                    embed_metadata: true,
                    chapters,
                });
            }
            if kd.key_part_1.len() != 16 {
                return Err(LibationError::InvalidInput(
                    "Unsupported Audible key format (expected AAXC key or activation bytes)".into(),
                ));
            }
            let iv = kd.key_part_2.as_ref().ok_or_else(|| {
                LibationError::InvalidInput("No IV in Audible AAXC keys".into())
            })?;
            DownloadPart::Aaxc {
                url: license.download_url,
                headers: Default::default(),
                key: hex::encode(&kd.key_part_1),
                iv: hex::encode(iv),
                filename,
            }
        } else {
            DownloadPart::Plain {
                url: license.download_url,
                headers: Default::default(),
                filename,
            }
        };

        Ok(DownloadPlan {
            parts: vec![part],
            embed_metadata: true,
            chapters,
        })
    }
}
