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
//! draft PR #914). Does not work for pre-Amazon Audible accounts — those use the
//! external-browser flow (`generate_authorization_url` + `exchange_authorization_code`).

use serde::{Deserialize, Serialize};

use crate::api::auth::{Account, Locale, RegistrationResponse};
use crate::Result;

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
pub async fn create_code_pair(locale: &Locale, device_serial: &str) -> Result<CodePair> {
    let _ = (locale, device_serial);
    todo!("core: create_code_pair")
}

/// Poll `/auth/register` once with the code pair, registering with the current device
/// profile (`DeviceProfile::default()`). `attempt` is the number of consecutive
/// `Backoff` results so far (for the exponential wait). Any error other than
/// pending/backoff is returned as `Err` and is fatal for this pair.
pub async fn poll_code_pair_registration(
    locale: &Locale,
    device_serial: &str,
    pair: &CodePair,
    attempt: u32,
) -> Result<CodePairPoll> {
    let _ = (locale, device_serial, pair, attempt);
    todo!("core: poll_code_pair_registration")
}

/// Build the [`Account`] the app stores after sign-in (the Rust equivalent of
/// `src/screens/LoginScreen.tsx`'s account construction), ready for
/// `storage::accounts` to save. Works for both device-code and external-browser
/// registrations.
pub fn account_from_registration(
    registration: &RegistrationResponse,
    locale: &Locale,
    device_serial: &str,
) -> Account {
    let _ = (registration, locale, device_serial);
    todo!("core: account_from_registration")
}
