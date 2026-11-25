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

/// Save a signed-in Audible account. When it replaces an existing registration of the
/// same account (signing in again), the old device is deregistered afterwards, best
/// effort: it would otherwise linger in the customer's Amazon device list.
async fn save_audible_account(app: &AppState, account_id: &str, json: &Value) -> anyhow::Result<AccountView> {
    let previous = accounts::load_json(app, account_id).await.ok().flatten();
    let view = accounts::add(app, account_id, json).await?;
    if let Some(old) = previous.and_then(|p| device_to_deregister(&p, json)) {
        tokio::spawn(async move {
            match rust_core::api::auth::deregister_device(&old).await {
                Ok(()) => tracing::info!("Deregistered the previous device of a re-signed-in Audible account"),
                Err(e) => tracing::warn!("Could not deregister the previous device of a re-signed-in Audible account: {e}"),
            }
        });
    }
    Ok(view)
}

/// The previous registration to deregister: an Audible account with a different
/// device serial than the new one. The same serial would deregister the new device.
fn device_to_deregister(previous: &Value, new: &Value) -> Option<rust_core::api::auth::Account> {
    if accounts::provider_of(previous).ok()? != rust_core::providers::ProviderId::Audible {
        return None;
    }
    let serial = |v: &Value| {
        v.pointer("/identity/device_serial_number")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let old_serial = serial(previous)?;
    if Some(&old_serial) == serial(new).as_ref() {
        return None;
    }
    serde_json::from_value(previous.clone()).ok()
}

pub async fn start_device_code(app: &AppState, locale_code: &str) -> Result<DeviceCodeSession, ApiError> {
    let locale = locale(locale_code)?;
    let serial = generate_device_serial();
    // The pair's lifetime counts from when Amazon issued it, so measure from just
    // before asking: never later than Amazon's own deadline.
    let requested = Instant::now();
    let requested_at = time::now();
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
        expires_at: time::fmt(requested_at + Duration::seconds(pair.expires_in as i64)),
        account: None,
        message: None,
    };
    let cancel = app.shutdown.child_token();
    app.logins.prune();
    if let Ok(mut m) = app.logins.device.lock() {
        m.insert(id.clone(), DeviceState { view: view.clone(), cancel: cancel.clone(), created: Instant::now() });
    }
    let deadline = requested + StdDuration::from_secs(pair.expires_in);
    tokio::spawn(poll_device_code(app.clone(), id, locale, serial, pair, deadline, cancel));
    Ok(view)
}

/// Poll Amazon until the user confirms the code, the pair expires, or the session is
/// cancelled. Honours Amazon's polling interval and its backoff requests.
async fn poll_device_code(
    app: AppState,
    id: String,
    locale: Locale,
    serial: String,
    pair: CodePair,
    deadline: Instant,
    cancel: CancellationToken,
) {
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
        // Cancelled while the poll was in flight: never add the account. If Amazon
        // registered the device in that moment, undo the registration (best effort).
        if cancel.is_cancelled() {
            if let Ok(Ok(CodePairPoll::Registered(reg))) = poll {
                if let Ok((_, json)) = account_json(*reg, locale.clone(), serial.clone()).await {
                    if let Ok(account) = serde_json::from_value::<rust_core::api::auth::Account>(json) {
                        if let Err(e) = rust_core::api::auth::deregister_device(&account).await {
                            tracing::warn!("Could not deregister a device registered after its sign-in was cancelled: {e}");
                        }
                    }
                }
            }
            return;
        }
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
                    Ok((account_id, json)) => {
                        save_audible_account(&app, &account_id, &json).await.map_err(|e| format!("{e:#}"))
                    }
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
    let view = save_audible_account(app, &account_id, &json).await?;
    if let Ok(mut m) = app.logins.external.lock() {
        m.remove(id);
    }
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_replaced_audible_device_is_deregistered() {
        use rust_core::api::auth::{AccessToken, Account, Identity};
        let account = |serial: &str, provider: &str| {
            let mut a = Account::new("amzn1.account.X:us".into()).unwrap();
            let token = AccessToken { token: "t".into(), expires_at: chrono::Utc::now() };
            let locale = Locale::from_country_code("us").unwrap();
            let mut identity = Identity::new(token, "r".into(), "k".into(), "adp".into(), locale);
            identity.device_serial_number = serial.into();
            a.identity = Some(identity);
            let mut v = serde_json::to_value(&a).unwrap();
            v["provider"] = Value::from(provider);
            v
        };
        let old = device_to_deregister(&account("A", "audible"), &account("B", "audible")).expect("a replaced device");
        assert_eq!(old.identity.unwrap().device_serial_number, "A", "the old device, not the new one");
        // The same serial is the device just registered: never deregister it.
        assert!(device_to_deregister(&account("A", "audible"), &account("A", "audible")).is_none());
        assert!(device_to_deregister(&account("A", "chirp"), &account("B", "chirp")).is_none());
        assert!(device_to_deregister(&account("", "audible"), &account("B", "audible")).is_none());
    }

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
}
