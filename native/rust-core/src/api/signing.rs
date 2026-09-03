//! ADP request signing — the auth scheme Audible uses for endpoints the bearer token
//! cannot reach.
//!
//! Reference: `references/AudibleApi/AudibleApi/Cryptography/Util.cs:30-63` (SignRequest,
//! CalculateSignature) and `AudibleApi/Cryptography/PrivateKey.cs:33-60` (SignMessage,
//! CreateRsaProviderFromPrivateKey).
//!
//! Most of the API answers a plain `Authorization: Bearer` header, which is what the rest
//! of this client sends. A few endpoints do not — `/1.0/customer/information` and
//! `/license/token` (activation bytes) reject it with
//! `{"message":"Request could not be authenticated"}` — and instead want the request
//! signed with the device's RSA key from registration:
//!
//! ```text
//! x-adp-token:     <adp_token>
//! x-adp-alg:       SHA256withRSA:1.0
//! x-adp-signature: base64(RSA-PKCS1v15-SHA256(data)):<date>
//!
//! data = "{METHOD}\n{path with query}\n{date}\n{body}\n{adp_token}"
//! ```
//!
//! The reference signs the *relative* URI (its HttpClient carries the base address), so
//! the signed path must exclude the scheme and host.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{DateTime, SecondsFormat, Utc};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::{Pkcs1v15Sign, RsaPrivateKey};
use sha2::{Digest, Sha256};

use crate::{LibationError, Result};

/// Value of the `x-adp-alg` header. Reference: Util.cs:37.
pub const ADP_ALG: &str = "SHA256withRSA:1.0";

/// The three headers that authenticate a signed request.
#[derive(Debug, Clone)]
pub struct AdpSignature {
    pub adp_token: String,
    pub alg: &'static str,
    pub signature: String,
}

impl AdpSignature {
    /// As `(name, value)` pairs, ready to add to a request.
    pub fn headers(&self) -> [(&'static str, String); 3] {
        [
            ("x-adp-token", self.adp_token.clone()),
            ("x-adp-alg", self.alg.to_string()),
            ("x-adp-signature", self.signature.clone()),
        ]
    }
}

/// Sign a request. `path_with_query` must be the path the server sees (`/1.0/customer/
/// information?response_groups=…`), not an absolute URL, and `body` is empty for GETs.
pub fn sign_request(
    method: &str,
    path_with_query: &str,
    body: &str,
    adp_token: &str,
    private_key: &str,
    now: DateTime<Utc>,
) -> Result<AdpSignature> {
    if adp_token.trim().is_empty() {
        return Err(LibationError::InvalidInput(
            "Account has no ADP token; sign in again to register the device".to_string(),
        ));
    }

    let key = parse_private_key(private_key)?;
    let date = rfc3339_millis(now);

    // Field order is part of the contract. Reference: Util.cs:58.
    let data = format!("{method}\n{path_with_query}\n{date}\n{body}\n{adp_token}");

    let digest = Sha256::digest(data.as_bytes());
    let signed = key
        .sign(Pkcs1v15Sign::new::<Sha256>(), &digest)
        .map_err(|e| LibationError::InternalError(format!("Failed to sign request: {e}")))?;

    Ok(AdpSignature {
        adp_token: adp_token.to_string(),
        alg: ADP_ALG,
        signature: format!("{}:{}", BASE64.encode(signed), date),
    })
}

/// The date format the signature carries, e.g. `2026-09-03T18:31:28.818Z`.
/// Reference: Util.cs:55 (`dateTime.ToRfc3339String()`).
pub fn rfc3339_millis(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Read the device key stored at registration.
///
/// Audible hands out PKCS#1 for iOS registrations and PKCS#8 for Android ones, with or
/// without PEM armour, so try each in turn rather than assuming.
/// Reference: PrivateKey.cs:42-60.
fn parse_private_key(raw: &str) -> Result<RsaPrivateKey> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(LibationError::InvalidInput(
            "Account has no device private key; sign in again to register the device".to_string(),
        ));
    }

    if trimmed.contains("-----BEGIN") {
        if let Ok(key) = RsaPrivateKey::from_pkcs1_pem(trimmed) {
            return Ok(key);
        }
        if let Ok(key) = RsaPrivateKey::from_pkcs8_pem(trimmed) {
            return Ok(key);
        }
    }

    // Bare base64 DER: strip any armour and whitespace, then try both encodings.
    let body: String = trimmed
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<String>()
        .replace(['\r', '\n', ' ', '\t'], "")
        .replace("\\n", "");

    let der = BASE64.decode(body.as_bytes()).map_err(|e| {
        LibationError::InvalidInput(format!("Device private key is not valid base64: {e}"))
    })?;

    RsaPrivateKey::from_pkcs1_der(&der)
        .or_else(|_| RsaPrivateKey::from_pkcs8_der(&der))
        .map_err(|e| {
            LibationError::InvalidInput(format!(
                "Device private key is neither PKCS#1 nor PKCS#8: {e}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::pkcs8::EncodePrivateKey;
    use rsa::pkcs1v15::VerifyingKey;
    use rsa::signature::Verifier;
    use rsa::RsaPublicKey;

    fn test_key() -> RsaPrivateKey {
        // Small key: these tests exercise encoding and the signed data string, not
        // cryptographic strength.
        let mut rng = rand::thread_rng();
        RsaPrivateKey::new(&mut rng, 1024).unwrap()
    }

    #[test]
    fn signature_verifies_over_the_documented_data_string() {
        let key = test_key();
        let pem = key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap().to_string();
        let now = Utc.with_ymd_and_hms(2026, 9, 3, 18, 31, 28).unwrap();

        let signed = sign_request(
            "GET",
            "/1.0/customer/information?response_groups=migration_details",
            "",
            "{enc:token}",
            &pem,
            now,
        )
        .unwrap();

        // base64 has no colons, so the first one separates signature from date — the date
        // itself contains colons, which is why this must not split from the right.
        let (sig_b64, date) = signed.signature.split_once(':').unwrap();
        assert_eq!(date, "2026-09-03T18:31:28.000Z");
        assert_eq!(signed.alg, "SHA256withRSA:1.0");

        let expected = format!(
            "GET\n/1.0/customer/information?response_groups=migration_details\n{date}\n\n{{enc:token}}"
        );
        let verifying = VerifyingKey::<Sha256>::new(RsaPublicKey::from(&key));
        let signature = rsa::pkcs1v15::Signature::try_from(BASE64.decode(sig_b64).unwrap().as_slice())
            .unwrap();
        verifying
            .verify(expected.as_bytes(), &signature)
            .expect("signature must cover method, path, date, body and adp token");
    }

    #[test]
    fn accepts_the_key_encodings_audible_hands_out() {
        let key = test_key();
        let now = Utc.with_ymd_and_hms(2026, 9, 3, 18, 31, 28).unwrap();

        let pkcs8_pem = key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap().to_string();
        let pkcs1_pem = key.to_pkcs1_pem(rsa::pkcs8::LineEnding::LF).unwrap().to_string();
        // Android registrations arrive as bare base64 DER, no armour.
        let bare_pkcs8 = BASE64.encode(key.to_pkcs8_der().unwrap().as_bytes());
        let bare_pkcs1 = BASE64.encode(key.to_pkcs1_der().unwrap().as_bytes());

        for encoding in [pkcs8_pem, pkcs1_pem, bare_pkcs8, bare_pkcs1] {
            sign_request("GET", "/license/token", "", "adp", &encoding, now)
                .expect("every encoding Audible returns must be readable");
        }
    }

    #[test]
    fn refuses_to_sign_without_credentials() {
        let key = test_key();
        let pem = key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap().to_string();
        let now = Utc::now();

        assert!(sign_request("GET", "/x", "", "", &pem, now).is_err());
        assert!(sign_request("GET", "/x", "", "adp", "", now).is_err());
        assert!(sign_request("GET", "/x", "", "adp", "not a key", now).is_err());
    }
}
