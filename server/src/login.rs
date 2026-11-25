//! Sign-in flows that need server-side state between requests: Audible device code
//! ("code pair") and Audible external browser (PKCE). Sessions live in memory for 15
//! minutes; nothing secret about them is ever logged or returned.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration as StdDuration, Instant};

use chrono::Duration;
use rust_core::api::auth::{
    exchange_authorization_code, generate_authorization_url, generate_device_serial, parse_authorization_callback,
    Locale, OAuthState, PkceChallenge,
};
use rust_core::api::device_code::{self, CodePair, CodePairPoll};
use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::accounts::{self, AccountView};
use crate::error::ApiError;
use crate::state::AppState;
use crate::time;

pub const SESSION_TTL: StdDuration = StdDuration::from_secs(15 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStatus {
    Pending,
    Registered,
    Expired,
    Error,
}

/// `DeviceCodeSession` in API.md.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCodeSession {
    pub session_id: String,
    pub status: DeviceStatus,
    pub public_code: String,
    pub verification_url: String,
    pub expires_at: String,
    pub account: Option<AccountView>,
    pub message: Option<String>,
}

struct DeviceState {
    view: DeviceCodeSession,
    cancel: CancellationToken,
    created: Instant,
}

struct ExternalState {
    locale: Locale,
    serial: String,
    verifier: String,
    created: Instant,
}

#[derive(Default)]
pub struct Sessions {
    device: Mutex<HashMap<String, DeviceState>>,
    external: Mutex<HashMap<String, ExternalState>>,
}

impl Sessions {
    fn prune(&self) {
        if let Ok(mut m) = self.device.lock() {
            m.retain(|_, s| {
                let keep = s.created.elapsed() < SESSION_TTL;
                if !keep {
                    s.cancel.cancel();
                }
                keep
            });
        }
        if let Ok(mut m) = self.external.lock() {
            m.retain(|_, s| s.created.elapsed() < SESSION_TTL);
        }
    }

    pub fn device_view(&self, id: &str) -> Option<DeviceCodeSession> {
        self.prune();
        self.device.lock().ok()?.get(id).map(|s| s.view.clone())
    }

    fn update_device(&self, id: &str, f: impl FnOnce(&mut DeviceCodeSession)) {
        if let Ok(mut m) = self.device.lock() {
            if let Some(s) = m.get_mut(id) {
                f(&mut s.view);
            }
        }
    }

    pub fn cancel_device(&self, id: &str) -> bool {
        let Ok(mut m) = self.device.lock() else { return false };
        match m.remove(id) {
            Some(s) => {
                s.cancel.cancel();
                true
            }
            None => false,
        }
    }

    pub fn cancel_all(&self) {
        if let Ok(mut m) = self.device.lock() {
            for s in m.values() {
                s.cancel.cancel();
            }
            m.clear();
        }
    }
}

pub fn locale(code: &str) -> Result<Locale, ApiError> {
    Locale::from_country_code(code).ok_or_else(|| ApiError::invalid_field("locale", format!("unknown Audible locale '{code}'")))
}

/// Run `fut` on its own task so a panic inside rust-core becomes an error here.
async fn isolate<T: Send + 'static>(fut: impl Future<Output = T> + Send + 'static) -> Result<T, String> {
    tokio::spawn(fut).await.map_err(|e| {
        if e.is_panic() {
            "sign-in is not available in this build (internal error)".to_string()
        } else {
            "cancelled".to_string()
        }
    })
}

/// The stored-account JSON for an Audible registration, built by rust-core.
async fn account_json(
    reg: rust_core::api::auth::RegistrationResponse,
    locale: Locale,
    serial: String,
) -> Result<(String, Value), String> {
    let account = isolate(async move { device_code::account_from_registration(&reg, &locale, &serial) }).await?;
    let id = account.account_id.clone();
    let mut json = serde_json::to_value(&account).map_err(|e| e.to_string())?;
    json["provider"] = Value::from("audible");
    Ok((id, json))
}

pub async fn start_device_code(app: &AppState, locale_code: &str) -> Result<DeviceCodeSession, ApiError> {
    let locale = locale(locale_code)?;
    let serial = generate_device_serial();
    let pair: CodePair = {
        let (l, s) = (locale.clone(), serial.clone());
        isolate(async move { device_code::create_code_pair(&l, &s).await })
            .await
            .map_err(ApiError::provider)?
            .map_err(|e| ApiError::provider(format!("Amazon refused to issue a sign-in code: {e}")))?
    };

    let id = uuid::Uuid::new_v4().to_string();
    let view = DeviceCodeSession {
        session_id: id.clone(),
        status: DeviceStatus::Pending,
        public_code: pair.public_code.clone(),
        verification_url: pair.verification_url.clone(),
        expires_at: time::fmt(time::now() + Duration::seconds(pair.expires_in as i64)),
        account: None,
        message: None,
    };
    let cancel = app.shutdown.child_token();
    app.logins.prune();
    if let Ok(mut m) = app.logins.device.lock() {
        m.insert(id.clone(), DeviceState { view: view.clone(), cancel: cancel.clone(), created: Instant::now() });
    }
    tokio::spawn(poll_device_code(app.clone(), id, locale, serial, pair, cancel));
    Ok(view)
}

/// Poll Amazon until the user confirms the code, the pair expires, or the session is
/// cancelled. Honours Amazon's polling interval and its backoff requests.
async fn poll_device_code(app: AppState, id: String, locale: Locale, serial: String, pair: CodePair, cancel: CancellationToken) {
    let deadline = Instant::now() + StdDuration::from_secs(pair.expires_in);
    let interval = StdDuration::from_secs(pair.polling_interval.max(1));
    let mut wait = interval;
    let mut backoffs: u32 = 0;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(wait) => {}
        }
        if Instant::now() >= deadline {
            app.logins.update_device(&id, |v| {
                v.status = DeviceStatus::Expired;
                v.message = Some("The code expired before it was confirmed".into());
            });
            return;
        }
        let poll = {
            let (l, s, p) = (locale.clone(), serial.clone(), pair.clone());
            isolate(async move { device_code::poll_code_pair_registration(&l, &s, &p, backoffs).await }).await
        };
        match poll {
            Ok(Ok(CodePairPoll::Pending)) => {
                backoffs = 0;
                wait = interval;
            }
            Ok(Ok(CodePairPoll::Backoff(secs))) => {
                backoffs += 1;
                wait = StdDuration::from_secs(secs.max(1));
            }
            Ok(Ok(CodePairPoll::Registered(reg))) => {
                let result = match account_json(*reg, locale.clone(), serial.clone()).await {
                    Ok((account_id, json)) => accounts::add(&app, &account_id, &json).await.map_err(|e| format!("{e:#}")),
                    Err(e) => Err(e),
                };
                app.logins.update_device(&id, |v| match result {
                    Ok(view) => {
                        v.status = DeviceStatus::Registered;
                        v.account = Some(view);
                    }
                    Err(e) => {
                        v.status = DeviceStatus::Error;
                        v.message = Some(format!("Signed in, but saving the account failed: {e}"));
                    }
                });
                return;
            }
            Ok(Err(e)) => {
                tracing::warn!("Device-code sign-in failed: {e}");
                app.logins.update_device(&id, |v| {
                    v.status = DeviceStatus::Error;
                    v.message = Some(e.to_string());
                });
                return;
            }
            Err(e) => {
                app.logins.update_device(&id, |v| {
                    v.status = DeviceStatus::Error;
                    v.message = Some(e);
                });
                return;
            }
        }
    }
}

pub fn start_external(app: &AppState, locale_code: &str) -> Result<(String, String), ApiError> {
    let locale = locale(locale_code)?;
    let serial = generate_device_serial();
    let pkce = PkceChallenge::generate().map_err(|e| ApiError::internal(e.to_string()))?;
    let url = generate_authorization_url(&locale, &serial, &pkce, &OAuthState::generate())
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let id = uuid::Uuid::new_v4().to_string();
    app.logins.prune();
    if let Ok(mut m) = app.logins.external.lock() {
        m.insert(id.clone(), ExternalState { locale, serial, verifier: pkce.verifier, created: Instant::now() });
    }
    Ok((id, url))
}

/// The Amazon domain a locale signs in on (mirrors `generate_authorization_url`).
fn amazon_domain(country: &str) -> &'static str {
    match country {
        "uk" => "amazon.co.uk",
        "de" => "amazon.de",
        "fr" => "amazon.fr",
        "ca" => "amazon.ca",
        "au" => "amazon.com.au",
        "it" => "amazon.it",
        "es" => "amazon.es",
        "in" => "amazon.in",
        "jp" => "amazon.co.jp",
        "br" => "amazon.com.br",
        _ => "amazon.com",
    }
}

fn host_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// The pasted URL must be the `/ap/maplanding` page Amazon redirected to, over https,
/// on the locale's Amazon (or Audible) domain — never an arbitrary URL.
pub fn validate_maplanding(url: &str, locale: &Locale) -> Result<(), ApiError> {
    let bad = || {
        ApiError::invalid_field(
            "responseUrl",
            "paste the full address of the page Amazon showed after signing in (it contains /ap/maplanding)",
        )
    };
    let parsed = url::Url::parse(url.trim()).map_err(|_| bad())?;
    if parsed.scheme() != "https" {
        return Err(bad());
    }
    let host = parsed.host_str().ok_or_else(bad)?.to_ascii_lowercase();
    let ok_host = host_matches(&host, amazon_domain(&locale.country_code)) || host_matches(&host, &locale.domain);
    if !ok_host || parsed.path() != "/ap/maplanding" {
        return Err(bad());
    }
    Ok(())
}

pub async fn complete_external(app: &AppState, id: &str, response_url: &str) -> Result<AccountView, ApiError> {
    app.logins.prune();
    let (locale, serial, verifier) = {
        let m = app.logins.external.lock().map_err(|_| ApiError::internal("login sessions unavailable"))?;
        let s = m.get(id).ok_or_else(|| ApiError::not_found("Sign-in session not found or expired"))?;
        (s.locale.clone(), s.serial.clone(), s.verifier.clone())
    };
    validate_maplanding(response_url, &locale)?;
    let code = parse_authorization_callback(response_url.trim())
        .map_err(|e| ApiError::provider_rejected(format!("Could not read the sign-in response: {e}")))?;
    let pkce = PkceChallenge { verifier, challenge: String::new(), method: "S256".into() };
    let reg = exchange_authorization_code(&locale, &code, &serial, &pkce)
        .await
        .map_err(|e| ApiError::provider(format!("Amazon refused the sign-in: {e}")))?;
    let (account_id, json) = account_json(reg, locale, serial).await.map_err(ApiError::internal)?;
    let view = accounts::add(app, &account_id, &json).await?;
    if let Ok(mut m) = app.logins.external.lock() {
        m.remove(id);
    }
    Ok(view)
}

/// Validate an app-exported Audible `Account` JSON and normalise it for storage.
pub fn parse_import(account_json: &str) -> Result<(String, Value), ApiError> {
    let bad = |m: &str| ApiError::invalid_field("accountJson", m.to_string());
    let mut v: Value = serde_json::from_str(account_json).map_err(|_| bad("is not valid JSON"))?;
    let obj = v.as_object_mut().ok_or_else(|| bad("must be a JSON object"))?;
    match obj.get("provider").and_then(Value::as_str) {
        None | Some("audible") => {}
        Some(_) => return Err(bad("only Audible accounts can be imported")),
    }
    obj.insert("provider".into(), Value::from("audible"));
    // The app omits these for some accounts; rust-core's `Account` requires them.
    obj.entry("decrypt_key").or_insert_with(|| Value::from(""));
    obj.entry("library_scan").or_insert(Value::Bool(true));

    let account: rust_core::api::auth::Account =
        serde_json::from_value(v.clone()).map_err(|e| bad(&format!("is not an Audible account: {e}")))?;
    if account.account_id.trim().is_empty() {
        return Err(bad("has no account_id"));
    }
    let identity = account.identity.as_ref().ok_or_else(|| bad("has no identity (sign in with the app first)"))?;
    if identity.refresh_token.trim().is_empty() {
        return Err(bad("has no refresh token"));
    }
    if identity.adp_token.trim().is_empty() || identity.device_private_key.trim().is_empty() {
        return Err(bad("lacks the device credentials downloads need; sign in again in the app and re-export"));
    }
    if v.get("account_name").and_then(Value::as_str).is_none_or(|n| n.trim().is_empty()) {
        v["account_name"] = Value::from(account.account_id.clone());
    }
    Ok((account.account_id, v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maplanding_urls_are_checked_strictly() {
        let us = Locale::from_country_code("us").unwrap();
        let de = Locale::from_country_code("de").unwrap();
        let ok = "https://www.amazon.com/ap/maplanding?openid.oa2.authorization_code=abc";
        assert!(validate_maplanding(ok, &us).is_ok());
        assert!(validate_maplanding("https://www.amazon.de/ap/maplanding?x=1", &de).is_ok());

        for bad in [
            "http://www.amazon.com/ap/maplanding?x=1",
            "https://www.amazon.com.evil.example/ap/maplanding?x=1",
            "https://evilamazon.com/ap/maplanding?x=1",
            "https://www.amazon.com/ap/signin?x=1",
            "https://www.amazon.de/ap/maplanding?x=1",
            "not a url",
        ] {
            assert!(validate_maplanding(bad, &us).is_err(), "{bad}");
        }
    }

    #[test]
    fn import_requires_a_usable_audible_identity() {
        assert!(parse_import("[]").is_err());
        assert!(parse_import(r#"{"provider":"chirp","account_id":"x"}"#).is_err());
        assert!(parse_import(r#"{"account_id":"x","account_name":"X"}"#).is_err());
    }
}
