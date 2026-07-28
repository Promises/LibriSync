//! Audible provider — a thin adapter over the existing `AudibleClient` / license
//! pipeline. All AAXC/OAuth/Widevine specifics stay in `api::*`; this only maps
//! them onto the [`Provider`] abstraction. Nothing calls it until the generic
//! bridge (Phase 2) routes Audible through the registry.

use async_trait::async_trait;

use crate::api::auth::Account;
use crate::api::client::AudibleClient;
use crate::api::content::DownloadQuality;
use crate::api::library::SyncStats;
use crate::storage::Database;
use crate::{LibationError, Result};

use super::{CredentialBlob, DownloadPart, DownloadPlan, LoginInput, Provider, ProviderId};

/// Audible provider. Stateless — it rebuilds an [`AudibleClient`] from the stored
/// credential blob per call, exactly as the current JNI entry points do.
pub struct AudibleProvider;

impl AudibleProvider {
    /// Deserialize the stored credential blob back into an `Account`.
    fn account_from(creds: &CredentialBlob) -> Result<Account> {
        serde_json::from_value(creds.clone())
            .map_err(|e| LibationError::InvalidInput(format!("Invalid Audible account: {e}")))
    }

    /// Refresh the access token if it's expired/expiring, then return the account.
    /// Threshold 30 min — same as the previous inline JNI behaviour.
    async fn valid_account(db: &Database, creds: &CredentialBlob) -> Result<Account> {
        let account_json = serde_json::to_string(creds)
            .map_err(|e| LibationError::InvalidInput(format!("Invalid Audible account: {e}")))?;
        let refreshed = crate::api::auth::ensure_valid_token(db.pool(), &account_json, 30).await?;
        serde_json::from_str(&refreshed)
            .map_err(|e| LibationError::InvalidInput(format!("Invalid Audible account: {e}")))
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

        let part = if license.drm_type.is_encrypted() {
            let keys = license
                .decryption_keys
                .as_ref()
                .filter(|k| !k.is_empty())
                .ok_or_else(|| {
                    LibationError::InvalidInput("No decryption keys in Audible license".into())
                })?;
            let kd = &keys[0];
            if kd.key_part_1.len() != 16 {
                return Err(LibationError::InvalidInput(
                    "Unsupported Audible key format (AAXC only)".into(),
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
            chapters: Vec::new(),
        })
    }
}
