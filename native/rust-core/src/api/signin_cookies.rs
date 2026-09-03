//! The cookies Amazon's sign-in page expects from a real Audible app.
//!
//! Reference: `AudibleApi/Authorization/RegistrationOptions.cs:67-200` (GetSignInCookies,
//! create_frc_cookie, create_map_md_cookie) and `AudibleApi/Cryptography/FrcEncoder.cs`.
//!
//! The official app seeds three cookies on `/ap` before the login page loads. We have
//! never sent any of them, and neither did upstream until late 2025. Since 2026-09-02
//! Audible has denied download licences to tokens minted by third-party registrations
//! while serving the official app, and the registration handshake is the only place the
//! two still differ — so this is the one remaining lever we control.
//!
//! Nothing here needs a secret from Amazon: `frc` is keyed entirely on a device serial
//! the client invents, which is what makes it reproducible at all.

use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use flate2::write::GzEncoder;
use flate2::Compression;
use hmac::{Hmac, Mac};
use std::io::Write;

use crate::api::auth::DeviceProfile;
use crate::{LibationError, Result};

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

/// PBKDF2 salt for the AES key. Reference: FrcEncoder.cs:86.
const AES_SALT: &[u8] = b"AES/CBC/PKCS7Padding";
/// PBKDF2 salt for the HMAC key. Reference: FrcEncoder.cs:87.
const HMAC_SALT: &[u8] = b"HmacSHA256";
/// Iterations and key length are fixed by Amazon's scheme. Reference: FrcEncoder.cs:93.
const PBKDF2_ROUNDS: u32 = 1000;
const KEY_LEN: usize = 16;

/// MAP library version the Android app reports. Reference: Resources.cs:33.
const MAP_VERSION: &str = "MAPAndroidLib-1.3.40908.0";

/// The three cookies, ready to set on the sign-in domain.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SignInCookies {
    /// Encrypted device fingerprint, set on `/ap`.
    pub frc: String,
    /// Base64 app identity blob, set on `/ap`.
    pub map_md: String,
    /// Always empty, set on `/`. Amazon just expects it to exist.
    pub sid: String,
}

/// Build the sign-in cookies for a device serial and the app identity we register as.
///
/// `language` is the locale's language tag (e.g. "en-US"); `timezone_offset` is the
/// local UTC offset formatted as ±HH:MM.
pub fn build(
    device_serial: &str,
    profile: &DeviceProfile,
    language: &str,
    timezone_offset: &str,
) -> Result<SignInCookies> {
    if device_serial.trim().is_empty() {
        return Err(LibationError::InvalidInput(
            "A device serial is required to build sign-in cookies".to_string(),
        ));
    }

    // Reference: RegistrationOptions.cs:165-178. Field order matches upstream; Amazon
    // only reads the JSON, but keeping the order makes the two directly comparable.
    let device_info = serde_json::json!({
        "ApplicationName": "com.audible.application",
        "ApplicationVersion": profile.app_version,
        "DeviceOSVersion": profile.os_version,
        "DeviceName": format!("{}/{}/{}", profile.product, profile.manufacturer, profile.model),
        "ScreenWidthPixels": "1080",
        "ThirdPartyDeviceId": device_serial,
        "FirstPartyDeviceId": device_serial,
        "ScreenHeightPixels": "2400",
        "DeviceLanguage": language,
        "TimeZone": timezone_offset,
        "Carrier": "T-Mobile",
        "IpAddress": "0.0.0.0",
    });

    Ok(SignInCookies {
        frc: encode_frc(device_serial, &device_info.to_string())?,
        map_md: map_md(profile),
        sid: String::new(),
    })
}

/// `base64([0x00] || hmac_sig[..8] || iv[16] || aes_cbc(gzip(json)))`.
/// Reference: FrcEncoder.cs:15-29.
fn encode_frc(device_serial: &str, json: &str) -> Result<String> {
    let mut gzip = GzEncoder::new(Vec::new(), Compression::best());
    gzip.write_all(json.as_bytes())
        .map_err(|e| LibationError::InternalError(format!("frc gzip failed: {e}")))?;
    let compressed = gzip
        .finish()
        .map_err(|e| LibationError::InternalError(format!("frc gzip failed: {e}")))?;

    let iv: [u8; 16] = rand::random();
    let aes_key = derive_key(device_serial, AES_SALT);
    let ciphertext = Aes128CbcEnc::new(&aes_key.into(), &iv.into())
        .encrypt_padded_vec_mut::<Pkcs7>(&compressed);

    let signature = sign(device_serial, &iv, &ciphertext);

    // Leading byte is a version marker and stays zero.
    let mut out = Vec::with_capacity(1 + signature.len() + iv.len() + ciphertext.len());
    out.push(0);
    out.extend_from_slice(&signature);
    out.extend_from_slice(&iv);
    out.extend_from_slice(&ciphertext);

    Ok(BASE64.encode(out))
}

/// First 8 bytes of HMAC-SHA256 over `iv || ciphertext`. Reference: FrcEncoder.cs:72-79.
fn sign(device_serial: &str, iv: &[u8], ciphertext: &[u8]) -> [u8; 8] {
    let key = derive_key(device_serial, HMAC_SALT);
    let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(&key)
        .expect("HMAC accepts keys of any length");
    mac.update(iv);
    mac.update(ciphertext);
    let full = mac.finalize().into_bytes();
    let mut out = [0u8; 8];
    out.copy_from_slice(&full[..8]);
    out
}

/// PBKDF2-SHA1 of the serial. Reference: FrcEncoder.cs:93.
fn derive_key(device_serial: &str, salt: &[u8]) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(device_serial.as_bytes(), salt, PBKDF2_ROUNDS, &mut key);
    key
}

/// Base64 of the app identity blob. Reference: RegistrationOptions.cs:116-146.
///
/// `SHA-256` is the APK signing-certificate digest; upstream sends null and so do we —
/// asserting the official app's certificate digest would be a claim we cannot make.
fn map_md(profile: &DeviceProfile) -> String {
    let map_md = serde_json::json!({
        "device_registration_data": { "software_version": profile.software_version },
        "app_identifier": {
            "package": "com.audible.application",
            "SHA-256": serde_json::Value::Null,
            "app_version": profile.app_version,
            "app_version_name": "26.34.07",
            "app_sms_hash": serde_json::Value::Null,
            "map_version": MAP_VERSION,
        },
        "app_info": {
            "auto_pv": 0,
            "auto_pv_with_smsretriever": 1,
            "smartlock_supported": 0,
            "permission_runtime_grant": 2,
        }
    });
    BASE64.encode(map_md.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockDecryptMut;
    use flate2::read::GzDecoder;
    use std::io::Read;

    type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

    /// Decode our own cookie the way upstream's `FrcEncoder.Decode` would, which is the
    /// only way to prove the layout without a sample from Amazon.
    fn decode_frc(device_serial: &str, encoded: &str) -> String {
        let bytes = BASE64.decode(encoded).expect("valid base64");
        let (signature, rest) = (&bytes[1..9], &bytes[9..]);
        let (iv, ciphertext) = rest.split_at(16);

        assert_eq!(signature, sign(device_serial, iv, ciphertext), "signature must verify");

        let key = derive_key(device_serial, AES_SALT);
        let compressed = Aes128CbcDec::new(&key.into(), iv.into())
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
            .expect("padding must be valid");

        let mut json = String::new();
        GzDecoder::new(&compressed[..])
            .read_to_string(&mut json)
            .expect("payload must be gzip");
        json
    }

    #[test]
    fn frc_round_trips_and_carries_the_device_fingerprint() {
        let serial = "7BB588D5C0FE4A1E9E1C2D3F4A5B6C7D";
        let cookies = build(&serial, &DeviceProfile::default(), "en-US", "+02:00").unwrap();

        let json: serde_json::Value =
            serde_json::from_str(&decode_frc(serial, &cookies.frc)).unwrap();
        assert_eq!(json["ThirdPartyDeviceId"], serial);
        assert_eq!(json["FirstPartyDeviceId"], serial);
        assert_eq!(json["ApplicationName"], "com.audible.application");
        assert_eq!(json["DeviceLanguage"], "en-US");
        assert_eq!(json["TimeZone"], "+02:00");

        // A different serial must not verify: the keys are derived from it.
        let other = build("0000000000000000", &DeviceProfile::default(), "en-US", "+02:00").unwrap();
        assert_ne!(other.frc, cookies.frc);
    }

    #[test]
    fn map_md_is_the_shape_amazon_expects() {
        let cookies = build("ABC123", &DeviceProfile::default(), "en-US", "+00:00").unwrap();
        let decoded = String::from_utf8(BASE64.decode(&cookies.map_md).unwrap()).unwrap();
        let json: serde_json::Value = serde_json::from_str(&decoded).unwrap();

        assert_eq!(json["app_identifier"]["package"], "com.audible.application");
        assert!(json["app_identifier"]["SHA-256"].is_null());
        assert_eq!(json["app_identifier"]["map_version"], MAP_VERSION);
        assert_eq!(json["app_info"]["permission_runtime_grant"], 2);
        assert!(!json["device_registration_data"]["software_version"]
            .as_str()
            .unwrap()
            .is_empty());

        // `sid` exists but is deliberately empty.
        assert!(cookies.sid.is_empty());
    }

    #[test]
    fn a_serial_is_required() {
        assert!(build("  ", &DeviceProfile::default(), "en-US", "+00:00").is_err());
    }
}
