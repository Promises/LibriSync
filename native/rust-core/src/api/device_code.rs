// LibriSync - Audible Library Sync for Mobile
// Copyright (C) 2025 Henning Berge
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//! Amazon device-code ("code pair") sign-in for Audible, and building the stored
//! [`Account`] from a device registration.
//!
//! The user opens `https://www.amazon.{tld}/code`, signs in there (2FA, CAPTCHA and CVF
//! all happen on Amazon's page) and enters a short public code; meanwhile we poll
//! `/auth/register` with the code pair. No password, PKCE or redirect URL is involved,
//! which makes it the right flow for a headless server.
//!
//! Reference: mkb79's device-code implementation
//! (https://gist.github.com/mkb79/d3ca499c2a7516933199914be3039d2d, mkb79/Audible#906,
//! draft PR #914) — `device_code_registration`, `retry_delay`, `amazon_error`,
//! `registration_credentials`. Does not work for pre-Amazon Audible accounts — those use
//! the external-browser flow (`generate_authorization_url` + `exchange_authorization_code`).
//!
//! [`account_from_registration`] is a port of the account construction in
//! `src/screens/LoginScreen.tsx` (the app's OAuth callback handler).

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::auth::{
    parse_registration_response, registration_request_body, AccessToken, Account, Identity,
    Locale, OAuthConfig, RegistrationAuth, RegistrationResponse,
};
use crate::{LibationError, Result};

/// The User-Agent the Audible for iPhone app's sign-in web view sends. WebKit has frozen
/// the OS token at 18_7 since iOS 26, so it does not match the registered OS version.
const IPHONE_WEBVIEW_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) \
     AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";

/// How long one code pair stays valid, requested from Amazon (seconds, as a string).
const CODE_DURATION_SECONDS: &str = "600";
/// Length of the public code the user types.
const CODE_LENGTH: &str = "5";
/// Upper bound for the exponential backoff after 429/5xx/network errors.
const MAX_BACKOFF_SECONDS: u64 = 60;
/// Per-request timeout, as in the reference.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);

/// A code pair issued by `POST https://api.amazon.{tld}/auth/create/codepair`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodePair {
    /// The short code the user types at `verification_url`.
    pub public_code: String,
    /// Secret half; never shown to the user.
    pub private_code: String,
    /// Seconds the pair stays valid (Amazon issues 600).
    pub expires_in: u64,
    /// Seconds to wait between registration polls.
    pub polling_interval: u64,
    /// `https://www.amazon.{tld}/code`
    pub verification_url: String,
}

/// Result of one registration poll.
#[derive(Debug, Clone)]
pub enum CodePairPoll {
    /// The user has not confirmed the code yet (Amazon answers 401 `Unauthorized`).
    Pending,
    /// Amazon is rate-limiting the polls (429) or had a transient 5xx/network error;
    /// wait this many seconds (exponential, capped at 60) before polling again.
    Backoff(u64),
    /// Confirmed: the device is registered.
    Registered(Box<RegistrationResponse>),
}

/// Request a code pair for `device_serial` (from [`crate::api::auth::generate_device_serial`]).
///
/// Keep `device_serial` for the polls: the registration is bound to it.
pub async fn create_code_pair(locale: &Locale, device_serial: &str) -> Result<CodePair> {
    create_code_pair_at(&amazon_api_base(locale), locale, device_serial).await
}

/// Poll `/auth/register` once with the code pair, registering with the current device
/// profile (`DeviceProfile::default()`). `attempt` is the number of consecutive
/// `Backoff` results so far (for the exponential wait). Any error other than
/// pending/backoff is returned as `Err` and is fatal for this pair.
///
/// The caller owns the clock: stop polling once `pair.expires_in` has elapsed since
/// [`create_code_pair`] was *called* (Amazon's validity starts before its answer).
pub async fn poll_code_pair_registration(
    locale: &Locale,
    device_serial: &str,
    pair: &CodePair,
    attempt: u32,
) -> Result<CodePairPoll> {
    poll_code_pair_registration_at(&amazon_api_base(locale), locale, device_serial, pair, attempt)
        .await
}

/// Build the [`Account`] the app stores after sign-in (the Rust equivalent of
/// `src/screens/LoginScreen.tsx`'s account construction), ready for
/// `storage::accounts` to save. Works for both device-code and external-browser
/// registrations.
///
/// `device_serial` is used only if the registration's `device_info` carries no serial
/// (the app always takes Amazon's echo of it).
///
/// Serialise the result with `serde_json::to_string` and pass it to
/// `storage::accounts::save_account(pool, &account.account_id, &json)`.
pub fn account_from_registration(
    registration: &RegistrationResponse,
    locale: &Locale,
    device_serial: &str,
) -> Account {
    account_from_registration_at(registration, locale, device_serial, Utc::now())
}

// ============================================================================
// Internals (base URL injectable so tests can drive a local server)
// ============================================================================

fn amazon_api_base(locale: &Locale) -> String {
    format!("https://api.{}", locale.amazon_domain())
}

/// The headers every device-code request carries. The auth-domain header names the
/// real Amazon API host even when `api_base` points elsewhere (tests).
fn request_headers(locale: &Locale) -> [(&'static str, String); 3] {
    [
        ("Accept", "application/json".to_string()),
        ("User-Agent", IPHONE_WEBVIEW_USER_AGENT.to_string()),
        (
            "x-amzn-identity-auth-domain",
            format!("api.{}", locale.amazon_domain()),
        ),
    ]
}

/// The status, `Retry-After` and body of one Amazon response.
#[derive(Debug)]
struct RawResponse {
    status: u16,
    retry_after: Option<f64>,
    body: String,
}

/// POST `body` as JSON with the device-code headers. A transport failure (connect,
/// timeout, reading the body) is the `Err` case.
async fn post_json(
    url: &str,
    locale: &Locale,
    body: &Value,
) -> std::result::Result<RawResponse, reqwest::Error> {
    // A fresh client per request, as the reference does, so no cookie carries over.
    let client = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()?;
    let mut request = client.post(url).json(body);
    for (name, value) in request_headers(locale) {
        request = request.header(name, value);
    }
    let response = request.send().await?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite());
    let body = response.text().await?;
    Ok(RawResponse {
        status,
        retry_after,
        body,
    })
}

/// `POST /auth/create/codepair` body.
fn code_pair_request_body(device_serial: &str) -> Value {
    serde_json::json!({
        "code_data": {
            "device_serial": device_serial,
            "device_type": OAuthConfig::default().device_type,
            "domain": "Device"
        },
        "code_properties": {
            "code_duration": CODE_DURATION_SECONDS,
            "code_length": CODE_LENGTH
        }
    })
}

async fn create_code_pair_at(
    api_base: &str,
    locale: &Locale,
    device_serial: &str,
) -> Result<CodePair> {
    let url = format!("{}/auth/create/codepair", api_base);
    let response = post_json(&url, locale, &code_pair_request_body(device_serial))
        .await
        .map_err(|e| LibationError::NetworkError {
            message: format!("Device code request failed: {}", e),
            is_transient: true,
        })?;
    parse_code_pair_response(&response, locale)
}

/// Map a `/auth/create/codepair` response to a [`CodePair`].
fn parse_code_pair_response(response: &RawResponse, locale: &Locale) -> Result<CodePair> {
    let data = json_object(&response.body);
    let (error_code, _) = amazon_error(&data);
    if response.status != 200 || error_code.is_some() {
        let message = format!(
            "Amazon refused to create a device code ({})",
            describe(response.status, &data)
        );
        // An overloaded endpoint is worth retrying; anything else is a refusal.
        if response.status == 429 || (500..=599).contains(&response.status) {
            return Err(LibationError::ApiRequestFailed {
                message,
                status_code: Some(response.status),
                endpoint: Some("/auth/create/codepair".to_string()),
            });
        }
        return Err(LibationError::AuthenticationFailed {
            message,
            account_id: None,
        });
    }

    let non_empty = |key: &str| -> Result<String> {
        data.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| LibationError::InvalidApiResponse {
                message: "Amazon's device code response lacks the code".to_string(),
                response_body: None,
            })
    };
    let public_code = non_empty("public_code")?;
    let private_code = non_empty("private_code")?;
    let expires_in = positive_number(data.get("expires_in"), "expires_in")?;
    let polling_interval = positive_number(
        data.get("polling_interval_in_seconds"),
        "polling_interval_in_seconds",
    )?;

    Ok(CodePair {
        public_code,
        private_code,
        expires_in: expires_in.floor() as u64,
        polling_interval: (polling_interval.ceil() as u64).max(1),
        verification_url: format!("https://www.{}/code", locale.amazon_domain()),
    })
}

async fn poll_code_pair_registration_at(
    api_base: &str,
    locale: &Locale,
    device_serial: &str,
    pair: &CodePair,
    attempt: u32,
) -> Result<CodePairPoll> {
    let auth = RegistrationAuth::CodePair {
        public_code: &pair.public_code,
        private_code: &pair.private_code,
    };
    let body = registration_request_body(locale.amazon_domain(), device_serial, &auth);
    let url = format!("{}/auth/register", api_base);
    match post_json(&url, locale, &body).await {
        // Never log the error: the request carries the private code.
        Err(_) => Ok(CodePairPoll::Backoff(backoff_seconds(
            pair.polling_interval,
            attempt,
            None,
        ))),
        Ok(response) => classify_poll_response(&response, &auth, device_serial, pair, attempt),
    }
}

/// Map one `/auth/register` response to pending / backoff / registered / fatal.
fn classify_poll_response(
    response: &RawResponse,
    auth: &RegistrationAuth<'_>,
    device_serial: &str,
    pair: &CodePair,
    attempt: u32,
) -> Result<CodePairPoll> {
    if response.status == 429 || (500..=599).contains(&response.status) {
        return Ok(CodePairPoll::Backoff(backoff_seconds(
            pair.polling_interval,
            attempt,
            response.retry_after,
        )));
    }

    let data = json_object(&response.body);
    let has_success = data
        .get("response")
        .and_then(Value::as_object)
        .is_some_and(|r| r.contains_key("success"));
    if response.status == 200 && has_success {
        let registration = parse_registration_response(&response.body, auth)?;
        // The reference refuses a registration echoing another device's serial.
        let returned = &registration.device_info.device_serial_number;
        if !returned.is_empty() && returned != device_serial {
            return Err(LibationError::AuthenticationFailed {
                message: "The registration belongs to another device".to_string(),
                account_id: None,
            });
        }
        return Ok(CodePairPoll::Registered(Box::new(registration)));
    }

    let (code, _) = amazon_error(&data);
    if response.status == 401
        && code
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case("unauthorized"))
    {
        return Ok(CodePairPoll::Pending);
    }

    Err(LibationError::AuthenticationFailed {
        message: format!(
            "Amazon rejected the device code ({})",
            describe(response.status, &data)
        ),
        account_id: None,
    })
}

/// Wait before the next poll after an overload or a network error: the polling
/// interval doubled per consecutive failure, capped at 60 s. A longer numeric
/// `Retry-After` from Amazon wins, and it is never shorter than the interval.
///
/// Reference: the gist's `retry_delay` (`failures` there is `attempt + 1` here).
fn backoff_seconds(interval: u64, attempt: u32, retry_after: Option<f64>) -> u64 {
    let interval = interval.max(1);
    let failures = attempt.saturating_add(1).min(4);
    let mut delay = interval.saturating_mul(1u64 << failures).min(MAX_BACKOFF_SECONDS);
    if let Some(retry_after) = retry_after.filter(|r| r.is_finite() && *r > 0.0) {
        delay = delay.max(retry_after.ceil() as u64);
    }
    delay.max(interval)
}

fn json_object(body: &str) -> Value {
    serde_json::from_str::<Value>(body)
        .ok()
        .filter(Value::is_object)
        .unwrap_or(Value::Null)
}

/// The error code and message Amazon sent, if any. Errors come either top-level
/// (`{"error": "...", "error_description": "..."}`) or nested
/// (`{"response": {"error": {"code": "...", "message": "..."}}}`).
///
/// Reference: the gist's `amazon_error`.
fn amazon_error(data: &Value) -> (Option<String>, Option<String>) {
    let error = data
        .get("error")
        .or_else(|| data.get("response").and_then(|r| r.get("error")));
    match error {
        Some(Value::String(code)) => (
            Some(code.clone()),
            data.get("error_description")
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
        Some(Value::Object(obj)) => {
            let code = ["code", "error_code", "error", "type"]
                .iter()
                .find_map(|k| obj.get(*k).and_then(Value::as_str))
                .map(str::to_string);
            let message = obj.get("message").and_then(Value::as_str).map(str::to_string);
            (code, message)
        }
        _ => (None, None),
    }
}

/// `HTTP 400, InvalidValue, The code has expired` — status plus Amazon's own words.
fn describe(status: u16, data: &Value) -> String {
    let (code, message) = amazon_error(data);
    [Some(format!("HTTP {}", status)), code, message]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ")
}

/// A strictly positive number that Amazon may send as a JSON number or a string.
fn positive_number(value: Option<&Value>, field: &str) -> Result<f64> {
    let number = match value {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    number
        .filter(|n| n.is_finite() && *n > 0.0)
        .ok_or_else(|| LibationError::InvalidApiResponse {
            message: format!("Amazon's device code response has an invalid {}", field),
            response_body: None,
        })
}

/// [`account_from_registration`] with an injectable clock.
fn account_from_registration_at(
    registration: &RegistrationResponse,
    locale: &Locale,
    device_serial: &str,
    now: DateTime<Utc>,
) -> Account {
    // The app builds its own locale object: the same code, name and domain as
    // `Locale::from_country_code`, but `with_username` is `code !== 'jp'`
    // (LoginScreen.tsx), which differs from the Rust table for de and br. Nothing
    // reads it, but keep the stored JSON identical to what the app writes.
    let locale = Locale {
        country_code: locale.country_code.clone(),
        name: locale.name.clone(),
        domain: locale.domain.clone(),
        with_username: locale.country_code != "jp",
    };

    // `expires_in` arrives as a string ("3600"). JS `Date` holds milliseconds, so the
    // app's expiry has millisecond precision. An unparsable value makes the token
    // already expired, so the first use refreshes it instead of trusting a guess.
    let expires_in_seconds = registration
        .bearer
        .expires_in
        .trim()
        .parse::<i64>()
        .unwrap_or(0);
    let expires_at = DateTime::<Utc>::from_timestamp_millis(
        now.timestamp_millis() + expires_in_seconds.saturating_mul(1000),
    )
    .unwrap_or(now);

    // Website cookies array → name/value map (a repeated name keeps the last value).
    let cookies: HashMap<String, String> = registration
        .website_cookies
        .iter()
        .map(|c| (c.name.clone(), c.value.clone()))
        .collect();

    let customer = &registration.customer_info;
    let customer_name = [customer.name.trim(), customer.given_name.trim()]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("Audible Account");
    let region_code = locale.country_code.to_uppercase();

    let device_serial_number = if registration.device_info.device_serial_number.is_empty() {
        device_serial.to_string()
    } else {
        registration.device_info.device_serial_number.clone()
    };

    Account {
        account_id: format!("{}:{}", customer.user_id, locale.country_code),
        account_name: format!("{} ({})", customer_name, region_code),
        library_scan: true,
        // Activation bytes are not fetched at sign-in; must be present (not null) or
        // `storage::accounts::get_account` returns JSON that won't deserialise.
        decrypt_key: String::new(),
        identity: Some(Identity {
            access_token: AccessToken {
                token: registration.bearer.access_token.clone(),
                expires_at,
            },
            refresh_token: registration.bearer.refresh_token.clone(),
            device_private_key: registration.mac_dms.device_private_key.clone(),
            adp_token: registration.mac_dms.adp_token.clone(),
            cookies,
            device_serial_number,
            device_type: registration.device_info.device_type.clone(),
            device_name: registration.device_info.device_name.clone(),
            amazon_account_id: customer.user_id.clone(),
            store_authentication_cookie: registration.store_authentication_cookie.cookie.clone(),
            locale,
            customer_info: customer.clone(),
        }),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::Database;
    use std::sync::{Arc, Mutex};

    const SERIAL: &str = "abcdefghijABCDEFGHIJ0123456789abcdefghij";

    fn pair() -> CodePair {
        CodePair {
            public_code: "ABC12".to_string(),
            private_code: "private-half".to_string(),
            expires_in: 600,
            polling_interval: 5,
            verification_url: "https://www.amazon.com/code".to_string(),
        }
    }

    fn raw(status: u16, body: &str) -> RawResponse {
        RawResponse {
            status,
            retry_after: None,
            body: body.to_string(),
        }
    }

    /// A synthetic `/auth/register` success, shaped like Amazon's (no real values).
    fn registration_json(serial: &str, with_store_cookie: bool) -> String {
        let mut tokens = serde_json::json!({
            "bearer": {"access_token": "Atna|fake", "refresh_token": "Atnr|fake", "expires_in": "3600"},
            "mac_dms": {"device_private_key": "fake-key", "adp_token": "{enc:fake}"},
            "website_cookies": [
                {"Name": "session-id", "Value": "000-0000000-0000000", "Domain": ".amazon.com",
                 "Path": "/", "Expires": "1 Jan 2050 00:00:00 GMT", "Secure": "true", "HttpOnly": "false"},
                {"Name": "at-main", "Value": "Atza|fake", "Domain": ".amazon.com",
                 "Path": "/", "Expires": "1 Jan 2050 00:00:00 GMT", "Secure": "true", "HttpOnly": "true"}
            ],
            "website_cookies_ttl": 31536000
        });
        if with_store_cookie {
            tokens["store_authentication_cookie"] = serde_json::json!({"cookie": "fake-store-cookie"});
        }
        serde_json::json!({
            "request_id": "00000000-0000-0000-0000-000000000000",
            "response": {"success": {
                "customer_id": "amzn1.account.FAKE",
                "tokens": tokens,
                "extensions": {
                    "device_info": {"device_name": "Test's Audible for iPhone",
                                    "device_serial_number": serial,
                                    "device_type": "A2CZJZGLK2JJVM"},
                    "customer_info": {"account_pool": "Amazon", "user_id": "amzn1.account.FAKE",
                                      "home_region": "NA", "name": " Test User ", "given_name": "Test"}
                }
            }}
        })
        .to_string()
    }

    fn code_pair_auth(pair: &CodePair) -> RegistrationAuth<'_> {
        RegistrationAuth::CodePair {
            public_code: &pair.public_code,
            private_code: &pair.private_code,
        }
    }

    // ---------- request bodies ----------

    #[test]
    fn code_pair_request_body_matches_the_reference() {
        assert_eq!(
            code_pair_request_body(SERIAL),
            serde_json::json!({
                "code_data": {"device_serial": SERIAL, "device_type": "A2CZJZGLK2JJVM", "domain": "Device"},
                "code_properties": {"code_duration": "600", "code_length": "5"}
            })
        );
    }

    #[test]
    fn registration_body_for_a_code_pair_matches_the_reference() {
        let pair = pair();
        let body = registration_request_body("amazon.co.uk", SERIAL, &code_pair_auth(&pair));
        assert_eq!(
            body,
            serde_json::json!({
                "requested_token_type": ["bearer", "mac_dms", "website_cookies"],
                "cookies": {"website_cookies": [], "domain": ".amazon.co.uk"},
                "registration_data": {
                    "domain": "Device", "app_version": "4.60", "device_serial": SERIAL,
                    "device_type": "A2CZJZGLK2JJVM",
                    "device_name": "%FIRST_NAME%%FIRST_NAME_POSSESSIVE_STRING%%DUPE_STRATEGY_1ST%Audible for iPhone",
                    "os_version": "26.2", "software_version": "46000856",
                    "device_model": "iPhone", "app_name": "Audible"
                },
                "auth_data": {
                    "code_pair": {"public_code": "ABC12", "private_code": "private-half"},
                    "use_global_authentication": "true"
                },
                "requested_extensions": ["device_info", "customer_info"]
            })
        );
    }

    #[test]
    fn headers_name_the_amazon_auth_domain_of_the_marketplace() {
        let headers = request_headers(&Locale::jp());
        assert!(headers.contains(&("x-amzn-identity-auth-domain", "api.amazon.co.jp".to_string())));
        assert!(headers.contains(&("Accept", "application/json".to_string())));
        assert!(headers
            .iter()
            .any(|(k, v)| *k == "User-Agent" && v.contains("iPhone OS 18_7")));
    }

    // ---------- code pair response ----------

    #[test]
    fn parses_a_code_pair_and_builds_the_verification_url() {
        let body = r#"{"public_code":"XY9Z1","private_code":"secret","expires_in":600,"polling_interval_in_seconds":"5"}"#;
        let pair = parse_code_pair_response(&raw(200, body), &Locale::de()).unwrap();
        assert_eq!(pair.public_code, "XY9Z1");
        assert_eq!(pair.private_code, "secret");
        assert_eq!(pair.expires_in, 600);
        assert_eq!(pair.polling_interval, 5);
        assert_eq!(pair.verification_url, "https://www.amazon.de/code");
    }

    #[test]
    fn a_code_pair_refusal_carries_amazons_words() {
        let body = r#"{"response":{"error":{"code":"InvalidValue","message":"bad device type"}}}"#;
        let err = parse_code_pair_response(&raw(400, body), &Locale::us()).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("HTTP 400, InvalidValue, bad device type"), "{text}");
    }

    #[test]
    fn a_code_pair_without_codes_is_an_invalid_response() {
        let body = r#"{"public_code":"","private_code":"x","expires_in":600,"polling_interval_in_seconds":5}"#;
        assert!(matches!(
            parse_code_pair_response(&raw(200, body), &Locale::us()),
            Err(LibationError::InvalidApiResponse { .. })
        ));
        let body = r#"{"public_code":"A","private_code":"x","expires_in":0,"polling_interval_in_seconds":5}"#;
        assert!(parse_code_pair_response(&raw(200, body), &Locale::us()).is_err());
    }

    // ---------- poll classification ----------

    #[test]
    fn unconfirmed_code_is_pending() {
        let pair = pair();
        let body = r#"{"response":{"error":{"code":"Unauthorized","message":"not yet"}}}"#;
        let poll =
            classify_poll_response(&raw(401, body), &code_pair_auth(&pair), SERIAL, &pair, 0)
                .unwrap();
        assert!(matches!(poll, CodePairPoll::Pending));
    }

    #[test]
    fn a_401_with_another_code_is_fatal() {
        let pair = pair();
        let body = r#"{"response":{"error":{"code":"InvalidToken","message":"nope"}}}"#;
        let err = classify_poll_response(&raw(401, body), &code_pair_auth(&pair), SERIAL, &pair, 0)
            .unwrap_err();
        assert!(err.to_string().contains("InvalidToken"));
    }

    #[test]
    fn overload_backs_off_exponentially_and_caps_at_60() {
        let pair = pair();
        let auth = code_pair_auth(&pair);
        let backoff = |status, attempt| {
            match classify_poll_response(&raw(status, "{}"), &auth, SERIAL, &pair, attempt).unwrap()
            {
                CodePairPoll::Backoff(s) => s,
                other => panic!("expected Backoff, got {other:?}"),
            }
        };
        assert_eq!(backoff(429, 0), 10);
        assert_eq!(backoff(503, 1), 20);
        assert_eq!(backoff(500, 2), 40);
        assert_eq!(backoff(500, 3), 60);
        assert_eq!(backoff(500, 40), 60);
    }

    #[test]
    fn backoff_honours_a_longer_retry_after_and_never_undercuts_the_interval() {
        assert_eq!(backoff_seconds(5, 0, Some(90.0)), 90);
        assert_eq!(backoff_seconds(5, 0, Some(1.0)), 10);
        assert_eq!(backoff_seconds(120, 0, None), 120);
        assert_eq!(backoff_seconds(0, 0, None), 2);
    }

    #[test]
    fn confirmed_code_is_registered_without_a_store_cookie() {
        let pair = pair();
        let body = registration_json(SERIAL, false);
        match classify_poll_response(&raw(200, &body), &code_pair_auth(&pair), SERIAL, &pair, 0)
            .unwrap()
        {
            CodePairPoll::Registered(reg) => {
                assert_eq!(reg.bearer.access_token, "Atna|fake");
                assert_eq!(reg.device_info.device_serial_number, SERIAL);
                assert_eq!(reg.store_authentication_cookie.cookie, "");
            }
            other => panic!("expected Registered, got {other:?}"),
        }
    }

    #[test]
    fn a_registration_for_another_serial_is_refused() {
        let pair = pair();
        let body = registration_json("someone-elses-serial", false);
        assert!(
            classify_poll_response(&raw(200, &body), &code_pair_auth(&pair), SERIAL, &pair, 0)
                .is_err()
        );
    }

    #[test]
    fn an_expired_code_is_fatal_with_the_server_message() {
        let pair = pair();
        let body = r#"{"response":{"error":{"code":"InvalidValue","message":"The code pair has expired"}}}"#;
        let err = classify_poll_response(&raw(400, body), &code_pair_auth(&pair), SERIAL, &pair, 0)
            .unwrap_err();
        assert!(err.to_string().contains("The code pair has expired"), "{err}");
        assert_eq!(err.error_code(), "auth_expired");
    }

    #[test]
    fn the_authorization_code_flow_still_requires_the_store_cookie() {
        let auth = RegistrationAuth::AuthorizationCode {
            client_id_hex: "00",
            authorization_code: "code",
            code_verifier: "verifier",
        };
        assert!(parse_registration_response(&registration_json(SERIAL, false), &auth).is_err());
        let reg = parse_registration_response(&registration_json(SERIAL, true), &auth).unwrap();
        assert_eq!(reg.store_authentication_cookie.cookie, "fake-store-cookie");
    }

    // ---------- the HTTP layer, against a local server ----------

    /// Serve one canned response per connection on 127.0.0.1; record every raw request.
    async fn serve(
        status: u16,
        extra_headers: &'static str,
        body: String,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let body_len = text[..end]
                            .lines()
                            .filter_map(|l| l.split_once(':'))
                            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                            .unwrap_or(0);
                        if buf.len() >= end + 4 + body_len {
                            break;
                        }
                    }
                }
                log.lock().unwrap().push(String::from_utf8_lossy(&buf).to_string());
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (url, seen)
    }

    fn request_body(request: &str) -> Value {
        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn create_code_pair_posts_the_reference_request() {
        let (base, seen) = serve(
            200,
            "",
            r#"{"public_code":"ABC12","private_code":"p","expires_in":600,"polling_interval_in_seconds":5}"#
                .to_string(),
        )
        .await;
        let pair = create_code_pair_at(&base, &Locale::fr(), SERIAL).await.unwrap();
        assert_eq!(pair.public_code, "ABC12");
        assert_eq!(pair.verification_url, "https://www.amazon.fr/code");

        let requests = seen.lock().unwrap();
        let request = &requests[0];
        assert!(request.starts_with("POST /auth/create/codepair "), "{request}");
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("x-amzn-identity-auth-domain: api.amazon.fr"));
        assert!(lower.contains("accept: application/json"));
        assert!(lower.contains("user-agent: mozilla/5.0 (iphone; cpu iphone os 18_7"));
        assert_eq!(request_body(request), code_pair_request_body(SERIAL));
    }

    #[tokio::test]
    async fn poll_maps_each_server_answer() {
        let pair = pair();
        let locale = Locale::us();

        let (base, seen) = serve(
            401,
            "",
            r#"{"response":{"error":{"code":"Unauthorized"}}}"#.to_string(),
        )
        .await;
        let poll = poll_code_pair_registration_at(&base, &locale, SERIAL, &pair, 0)
            .await
            .unwrap();
        assert!(matches!(poll, CodePairPoll::Pending));
        let request = seen.lock().unwrap()[0].clone();
        assert!(request.starts_with("POST /auth/register "), "{request}");
        assert_eq!(
            request_body(&request)["auth_data"]["code_pair"]["public_code"],
            "ABC12"
        );

        let (base, _) = serve(429, "Retry-After: 30\r\n", "{}".to_string()).await;
        let poll = poll_code_pair_registration_at(&base, &locale, SERIAL, &pair, 0)
            .await
            .unwrap();
        assert!(matches!(poll, CodePairPoll::Backoff(30)), "{poll:?}");

        let (base, _) = serve(200, "", registration_json(SERIAL, false)).await;
        let poll = poll_code_pair_registration_at(&base, &locale, SERIAL, &pair, 0)
            .await
            .unwrap();
        assert!(matches!(poll, CodePairPoll::Registered(_)));

        let (base, _) = serve(
            400,
            "",
            r#"{"response":{"error":{"code":"InvalidValue","message":"expired"}}}"#.to_string(),
        )
        .await;
        assert!(poll_code_pair_registration_at(&base, &locale, SERIAL, &pair, 0)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_network_failure_backs_off() {
        // Bind then drop a listener, so the port refuses connections.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let poll = poll_code_pair_registration_at(&base, &Locale::us(), SERIAL, &pair(), 2)
            .await
            .unwrap();
        assert!(matches!(poll, CodePairPoll::Backoff(40)), "{poll:?}");
    }

    // ---------- the stored account ----------

    /// What `LoginScreen.tsx` stores for the same registration (its `Account` literal),
    /// written out by hand so a drift on either side shows up here.
    fn app_account_json(now: DateTime<Utc>) -> Value {
        let expires_at = DateTime::<Utc>::from_timestamp_millis(now.timestamp_millis() + 3_600_000)
            .unwrap()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        let locale = serde_json::json!({
            "country_code": "de", "name": "Germany", "domain": "audible.de", "with_username": true
        });
        serde_json::json!({
            "account_id": "amzn1.account.FAKE:de",
            "account_name": "Test User (DE)",
            "library_scan": true,
            "decrypt_key": "",
            "locale": locale,
            "identity": {
                "access_token": {"token": "Atna|fake", "expires_at": expires_at},
                "refresh_token": "Atnr|fake",
                "device_private_key": "fake-key",
                "adp_token": "{enc:fake}",
                "cookies": {"session-id": "000-0000000-0000000", "at-main": "Atza|fake"},
                "device_serial_number": SERIAL,
                "device_type": "A2CZJZGLK2JJVM",
                "device_name": "Test's Audible for iPhone",
                "amazon_account_id": "amzn1.account.FAKE",
                "store_authentication_cookie": "fake-store-cookie",
                "locale": locale,
                "customer_info": {"account_pool": "Amazon", "user_id": "amzn1.account.FAKE",
                                  "home_region": "NA", "name": " Test User ", "given_name": "Test"}
            }
        })
    }

    fn registration() -> RegistrationResponse {
        let auth = RegistrationAuth::AuthorizationCode {
            client_id_hex: "00",
            authorization_code: "code",
            code_verifier: "verifier",
        };
        parse_registration_response(&registration_json(SERIAL, true), &auth).unwrap()
    }

    #[test]
    fn account_matches_what_the_app_builds() {
        let now = Utc::now();
        let account = account_from_registration_at(&registration(), &Locale::de(), SERIAL, now);
        let ours = serde_json::to_value(&account).unwrap();
        let app = app_account_json(now);

        for key in ["account_id", "account_name", "library_scan", "decrypt_key"] {
            assert_eq!(ours[key], app[key], "{key}");
        }
        // Identity: identical except that the expiry is compared as an instant (chrono
        // and JS print the same millisecond differently when it is .000).
        let mut ours_identity = ours["identity"].clone();
        let mut app_identity = app["identity"].clone();
        let expiry = |v: &Value| {
            v["access_token"]["expires_at"]
                .as_str()
                .unwrap()
                .parse::<DateTime<Utc>>()
                .unwrap()
        };
        assert_eq!(expiry(&ours_identity), expiry(&app_identity));
        ours_identity["access_token"]["expires_at"] = Value::Null;
        app_identity["access_token"]["expires_at"] = Value::Null;
        assert_eq!(ours_identity, app_identity);
    }

    #[test]
    fn account_name_falls_back_like_the_app() {
        let mut reg = registration();
        reg.customer_info.name = "  ".to_string();
        let account = account_from_registration(&reg, &Locale::us(), SERIAL);
        assert_eq!(account.account_name, "Test (US)");
        reg.customer_info.given_name = String::new();
        let account = account_from_registration(&reg, &Locale::us(), SERIAL);
        assert_eq!(account.account_name, "Audible Account (US)");
    }

    #[test]
    fn an_unparsable_expiry_makes_the_token_expired() {
        let mut reg = registration();
        reg.bearer.expires_in = "soon".to_string();
        let now = Utc::now();
        let account = account_from_registration_at(&reg, &Locale::us(), SERIAL, now);
        assert!(account.identity.unwrap().access_token.expires_at <= now);
    }

    #[tokio::test]
    async fn account_round_trips_through_storage_and_token_check() {
        use crate::api::auth::ensure_valid_token;
        use crate::storage::accounts::{get_account, save_account};

        let db = Database::new_in_memory().await.unwrap();
        let pool = db.pool();
        let now = Utc::now();
        let account = account_from_registration_at(&registration(), &Locale::de(), SERIAL, now);
        let json = serde_json::to_string(&account).unwrap();
        save_account(pool, &account.account_id, &json).await.unwrap();

        // The app's own JSON for the same registration lands in an identical row.
        let app = app_account_json(now);
        save_account(pool, "app-copy", &app.to_string()).await.unwrap();
        type Row = (String, String, String, Option<String>, String);
        let row = |id: &'static str| async move {
            sqlx::query_as::<_, Row>(
                "SELECT account_name, locale_code, identity_json, decrypt_key, COALESCE(provider,'audible') FROM Accounts WHERE account_id = ?",
            )
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
        };
        let (ours, app_row) = (row("amzn1.account.FAKE:de").await, row("app-copy").await);
        assert_eq!(ours.0, app_row.0);
        assert_eq!(ours.1, "de");
        assert_eq!(ours.1, app_row.1);
        assert_eq!(ours.3.as_deref(), Some(""));
        assert_eq!(ours.3, app_row.3);
        assert_eq!(ours.4, "audible");
        assert_eq!(ours.4, app_row.4);

        // Read back exactly as the app and the providers do.
        let stored = get_account(pool, &account.account_id).await.unwrap().unwrap();
        let parsed: Account = serde_json::from_str(&stored).unwrap();
        assert_eq!(
            serde_json::to_value(&parsed).unwrap(),
            serde_json::to_value(&account).unwrap()
        );
        let stored_value: Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(stored_value["provider"], "audible");
        assert_eq!(stored_value["locale"]["country_code"], "de");

        // Fresh token (expires in an hour), so this is the no-network path.
        let checked = ensure_valid_token(pool, &stored, 30).await.unwrap();
        let checked: Account = serde_json::from_str(&checked).unwrap();
        assert_eq!(checked.account_id, "amzn1.account.FAKE:de");
        let identity = checked.identity.unwrap();
        assert_eq!(identity.adp_token, "{enc:fake}");
        assert_eq!(identity.locale.country_code, "de");
    }
}
