// LibriSync - Audible Library Sync for Mobile
// Copyright (C) 2025 Henning Berge
//
// This program is a Rust port of Libation (https://github.com/rmcrackan/Libation)
// Original work Copyright (C) Libation contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! License and download voucher management
//!
//! # Reference C# Sources
//! - **External: `AudibleApi/Api.cs`** - GetDownloadLicenseAsync(asin, quality, chapterTitles, drmType, requestSpatial, aacCodec, spatialCodec)
//! - **`FileLiberator/AudioDecodable.cs`** - License request and voucher handling
//! - **`FileLiberator/DownloadOptions.cs`** - License information structures (LicenseInfo)
//! - **`FileLiberator/DownloadOptions.Factory.cs`** - ChooseContent() for AAX vs Widevine selection (lines 57-112)
//! - **`AaxDecrypter/AudiobookDownloadBase.cs`** - Download URL resolution
//! - **`AudibleUtilities/Widevine/Cdm.Api.cs`** - Widevine license requests (for AAXC)
//!
//! # License Request Flow
//!
//! ## AAX/AAXC Flow (Audible DRM)
//! 1. Request license with quality tier (Normal, High, Extreme)
//! 2. Receive ContentLicense with:
//!    - Voucher (contains key/IV for AAX or AAXC)
//!    - ContentMetadata (chapter info, codec, content reference)
//!    - ContentUrl (download URL)
//! 3. DRM type detection:
//!    - AAX: Key length is 4 bytes (activation bytes)
//!    - AAXC: Key length is 16 bytes + 16 bytes (key pairs)
//! 4. Use voucher to access CDN download URL
//! 5. Download encrypted file
//!
//! Reference: DownloadOptions.cs:69-76 - DRM type detection based on key length
//!
//! ## Widevine Flow (MPEG-DASH)
//! Reference: DownloadOptions.Factory.cs:68-112
//!
//! 1. Request Widevine license with codec preferences
//! 2. Receive ContentLicense with:
//!    - LicenseResponse (MPEG-DASH manifest URL)
//!    - ContentMetadata
//! 3. Download MPEG-DASH manifest (MPD file)
//! 4. Extract content URI from manifest
//! 5. Generate Widevine license challenge
//! 6. Exchange challenge for license keys via WidevineDrmLicense endpoint
//! 7. Parse license to extract decryption keys
//! 8. Use keys to decrypt DASH segments
//!
//! # API Endpoints
//!
//! ## License Request
//! **POST** `/1.0/content/{asin}/licenserequest`
//!
//! Request body (JSON):
//! ```json
//! {
//!   "quality": "Extreme",
//!   "consumption_type": "Download",
//!   "drm_type": "Mpeg",
//!   "chapter_titles_type": "Tree",
//!   "request_spatial": false,
//!   "aac_codec": "AAC_LC",
//!   "spatial_codec": "EC_3"
//! }
//! ```
//!
//! Response: ContentLicense with voucher/keys
//!
//! ## Widevine License Exchange
//! **POST** `/1.0/content/{asin}/licenseRequest`
//!
//! Request body: Widevine license challenge (binary)
//! Response: Widevine license response (binary)
//!
//! Reference: DownloadOptions.Factory.cs:100 - api.WidevineDrmLicense()

use crate::api::client::AudibleClient;
use crate::api::content::{ChapterTitlesType, Codec, ContentMetadata, DownloadQuality, DrmType};
use crate::error::{LibationError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ============================================================================
// LICENSE REQUEST STRUCTURES
// ============================================================================

/// License request parameters
/// Reference: DownloadOptions.Factory.cs:64-84 - api.GetDownloadLicenseAsync() parameters
///
/// C# method signature:
/// ```csharp
/// Task<ContentLicense> GetDownloadLicenseAsync(
///     string asin,
///     DownloadQuality quality,
///     ChapterTitlesType chapterTitlesType,
///     DrmType drmType,
///     bool requestSpatial,
///     Codecs aacCodecChoice,
///     Codecs spatialCodecChoice
/// )
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct LicenseRequest {
    /// Media features the client can play. Audible expects this nested object; a flat
    /// body is accepted but yields a license without `content_metadata.content_url`.
    /// Reference: AudibleApi/Api.Download.cs:150-160 - GetDownloadLicenseAsync
    #[serde(rename = "supported_media_features")]
    pub supported_media_features: SupportedMediaFeatures,

    /// Request spatial (Dolby Atmos) audio.
    #[serde(rename = "spatial")]
    pub spatial: bool,

    /// Consumption type (Download vs Streaming)
    /// Always "Download" for offline use
    #[serde(rename = "consumption_type")]
    pub consumption_type: ConsumptionType,

    /// Always "Audible".
    #[serde(rename = "tenant_id")]
    pub tenant_id: String,

    /// Download quality (Normal, High, Extreme)
    #[serde(rename = "quality")]
    pub quality: DownloadQuality,

    /// Response groups. `content_reference` and `chapter_info` are what carry the
    /// codec/size and the chapter markers; without them the license comes back thin.
    #[serde(rename = "response_groups")]
    pub response_groups: String,
}

/// The `supported_media_features` object of a license request.
/// Reference: AudibleApi/Api.Download.cs:152-160
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupportedMediaFeatures {
    /// DRM schemes the client accepts, e.g. `["Adrm", "Mpeg"]`. Audible picks one and
    /// reports it back as the license's `drm_type`.
    #[serde(rename = "drm_types")]
    pub drm_types: Vec<DrmType>,

    /// Audio codecs, as the MIME-style names the licence endpoint expects — these are
    /// *not* the names Audible uses when reporting a codec back.
    /// Reference: AudibleApi/Api.Download.cs:25-30 - class Codecs
    #[serde(rename = "codecs")]
    pub codecs: Vec<String>,

    /// Chapter titles type (Flat or Tree)
    #[serde(rename = "chapter_titles_type")]
    pub chapter_titles_type: ChapterTitlesType,

    #[serde(rename = "previews")]
    pub previews: bool,

    #[serde(rename = "catalog_samples")]
    pub catalog_samples: bool,
}

/// Consumption type for license request
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsumptionType {
    /// Download for offline playback
    #[serde(rename = "Download")]
    Download,

    /// Streaming playback
    #[serde(rename = "Streaming")]
    Streaming,
}

/// Codec identifiers accepted by the license endpoint.
/// Reference: AudibleApi/Api.Download.cs:25-30
pub mod request_codecs {
    pub const AAC_LC: &str = "mp4a.40.2";
    pub const XHE_AAC: &str = "mp4a.40.42";
    pub const EC_3: &str = "ec+3";
    pub const AC_4: &str = "ac-4";
}

impl LicenseRequest {
    /// Response groups Audible needs in order to return the download URL, chapters and
    /// codec details. Reference: AudibleApi/Api.Download.cs:175.
    pub const RESPONSE_GROUPS: &'static str =
        "last_position_heard,pdf_url,content_reference,chapter_info";

    /// A download request for `drm_type`, always offering `Mpeg` as the fallback scheme
    /// the way the reference client does (podcast episodes are plain MP3).
    pub fn download(quality: DownloadQuality, drm_type: DrmType) -> Self {
        Self {
            supported_media_features: SupportedMediaFeatures {
                // Deliberately NOT offering Widevine or xHE-AAC, unlike the official app.
                // The metadata endpoint reports `mp4a.40.2` for these titles, but offering
                // xHE makes Audible answer in xHE/Widevine terms (`M4A_XHE`, a DASH
                // manifest instead of an offline URL) — a shape this client cannot consume.
                // Ask only for what we can actually decrypt.
                drm_types: vec![drm_type, DrmType::Mpeg],
                codecs: vec![request_codecs::AAC_LC.to_string()],
                chapter_titles_type: ChapterTitlesType::Tree,
                previews: false,
                catalog_samples: false,
            },
            spatial: false,
            consumption_type: ConsumptionType::Download,
            tenant_id: "Audible".to_string(),
            quality,
            response_groups: Self::RESPONSE_GROUPS.to_string(),
        }
    }
}

impl Default for LicenseRequest {
    fn default() -> Self {
        Self::download(DownloadQuality::High, DrmType::Adrm)
    }
}

// ============================================================================
// LICENSE RESPONSE STRUCTURES
// ============================================================================

/// Voucher with decryption key and IV
/// Reference: AudibleApi.Common.VoucherDtoV10, DownloadOptions.Factory.cs:53-54
///
/// C# properties:
/// - Key (string) - Base64 encoded decryption key
/// - Iv (string) - Base64 encoded initialization vector
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Voucher {
    /// Decryption key (Base64 encoded)
    /// - AAX: 4 bytes (activation bytes)
    /// - AAXC: 16 bytes (key part 1)
    #[serde(rename = "key")]
    pub key: String,

    /// Initialization vector (Base64 encoded)
    /// - AAX: None
    /// - AAXC: 16 bytes (key part 2)
    #[serde(rename = "iv", skip_serializing_if = "Option::is_none")]
    pub iv: Option<String>,
}

/// Content license response
/// Reference: AudibleApi.Common.ContentLicense, DownloadOptions.Factory.cs:42-55
///
/// C# properties:
/// - DrmType (DrmType) - Actual DRM type returned
/// - ContentMetadata (ContentMetadata) - Chapter info, codec, content reference
/// - Voucher (VoucherDtoV10) - Decryption keys for Adrm
/// - LicenseResponse (string) - MPEG-DASH manifest URL for Widevine
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContentLicense {
    /// Actual DRM type provided by API
    /// May differ from requested type
    /// Reference: DownloadOptions.Factory.cs:86 - contentLic.DrmType check
    #[serde(rename = "drm_type")]
    pub drm_type: DrmType,

    /// Content metadata with chapters and codec info
    #[serde(rename = "content_metadata")]
    pub content_metadata: ContentMetadata,

    /// Voucher with decryption keys (for Adrm/AAX/AAXC)
    /// Reference: DownloadOptions.Factory.cs:46-50 - ToKeys(license.Voucher)
    #[serde(rename = "voucher", skip_serializing_if = "Option::is_none")]
    pub voucher: Option<Voucher>,

    /// MPEG-DASH manifest URL (for Widevine)
    /// Reference: DownloadOptions.Factory.cs:90 - contentLic.LicenseResponse
    #[serde(rename = "license_response", skip_serializing_if = "Option::is_none")]
    pub license_response: Option<String>,
}

/// Download license with all necessary information
/// Higher-level structure combining ContentLicense with decryption keys
///
/// Reference: DownloadOptions.Factory.cs:41-55 - LicenseInfo private class
pub struct DownloadLicense {
    /// DRM type
    pub drm_type: DrmType,

    /// Content metadata
    pub content_metadata: ContentMetadata,

    /// Decryption keys (parsed from voucher)
    /// Reference: DownloadOptions.cs:19 - KeyData[]?
    pub decryption_keys: Option<Vec<KeyData>>,

    /// Download URL (extracted from content_metadata or DASH manifest)
    pub download_url: String,
}

/// Key data for decryption
/// Reference: AaxDecrypter/KeyData.cs, DownloadOptions.Factory.cs:53-54
///
/// C# constructor:
/// ```csharp
/// new KeyData(voucher.Key, voucher.Iv)
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyData {
    /// Decryption key part 1
    /// - AAX: 4 bytes (activation bytes)
    /// - AAXC: 16 bytes
    #[serde(rename = "key_part_1")]
    pub key_part_1: Vec<u8>,

    /// Decryption key part 2 (optional)
    /// - AAX: None
    /// - AAXC: 16 bytes
    #[serde(rename = "key_part_2", skip_serializing_if = "Option::is_none")]
    pub key_part_2: Option<Vec<u8>>,
}

impl KeyData {
    /// Create KeyData from hex-encoded key and IV
    ///
    /// # Reference
    /// For AAXC files, the decrypted voucher contains hex-encoded keys
    /// Reference: ContentLicenseDtoV10.cs voucher JSON format
    ///
    /// # Arguments
    /// * `key_hex` - Hex-encoded key (32 hex chars = 16 bytes)
    /// * `iv_hex` - Optional hex-encoded IV (32 hex chars = 16 bytes)
    ///
    /// # Returns
    /// KeyData with decoded bytes
    pub fn from_hex(key_hex: &str, iv_hex: Option<&str>) -> Result<Self> {
        let key_bytes = hex::decode(key_hex)
            .map_err(|e| LibationError::InvalidInput(format!("Invalid hex key: {}", e)))?;

        let iv_bytes = if let Some(iv_str) = iv_hex {
            Some(
                hex::decode(iv_str)
                    .map_err(|e| LibationError::InvalidInput(format!("Invalid hex IV: {}", e)))?,
            )
        } else {
            None
        };

        Ok(Self {
            key_part_1: key_bytes,
            key_part_2: iv_bytes,
        })
    }

    /// Create KeyData from Base64 encoded key and IV
    ///
    /// # Reference
    /// C# code: DownloadOptions.Factory.cs:53-54
    /// ```csharp
    /// private static KeyData[]? ToKeys(VoucherDtoV10? voucher)
    ///     => voucher is null ? null : [new KeyData(voucher.Key, voucher.Iv)];
    /// ```
    pub fn from_base64(key: &str, iv: Option<&str>) -> Result<Self> {
        use base64::{engine::general_purpose, Engine as _};

        let key_bytes = general_purpose::STANDARD
            .decode(key)
            .map_err(|e| LibationError::InvalidInput(format!("Invalid base64 key: {}", e)))?;

        let iv_bytes =
            if let Some(iv_str) = iv {
                Some(general_purpose::STANDARD.decode(iv_str).map_err(|e| {
                    LibationError::InvalidInput(format!("Invalid base64 IV: {}", e))
                })?)
            } else {
                None
            };

        Ok(Self {
            key_part_1: key_bytes,
            key_part_2: iv_bytes,
        })
    }

    /// Decrypt license_response to extract voucher with activation bytes
    ///
    /// # Reference
    /// C# code: AudibleApi.Common/ContentLicenseDtoV10.cs:19-47 - DecryptLicenseResponse()
    ///
    /// For AAXC files, the license_response field contains an AES-encrypted voucher.
    /// The decryption key and IV are derived from SHA256 hash of:
    /// - device_type + device_serial + amazon_account_id + asin
    ///
    /// AAXC scheme described in:
    /// https://patchwork.ffmpeg.org/project/ffmpeg/patch/17559601585196510@sas2-2fa759678732.qloud-c.yandex.net/
    ///
    /// # Arguments
    /// * `license_response_b64` - Base64-encoded encrypted license response
    /// * `device_type` - Device type (e.g., "A2CZJZGLK2JJVM")
    /// * `device_serial` - Device serial number
    /// * `account_id` - Amazon account ID
    /// * `asin` - Book ASIN
    ///
    /// # Returns
    /// KeyData with decryption keys (4 bytes for AAX, 16+16 bytes for AAXC)
    ///
    /// # Errors
    /// - `InvalidInput` - Invalid base64 or decryption failed
    pub fn from_license_response(
        license_response_b64: &str,
        device_type: &str,
        device_serial: &str,
        account_id: &str,
        asin: &str,
    ) -> Result<Self> {
        use aes::Aes128;
        use base64::{engine::general_purpose, Engine as _};
        use cbc::{
            cipher::{BlockDecryptMut, KeyIvInit},
            Decryptor,
        };
        use sha2::{Digest, Sha256};

        // Decode base64 ciphertext
        // Reference: ContentLicenseDtoV10.cs:38
        let ciphertext = general_purpose::STANDARD
            .decode(license_response_b64)
            .map_err(|e| {
                LibationError::InvalidInput(format!("Invalid base64 license_response: {}", e))
            })?;

        // Derive key and IV from SHA256 hash
        // Reference: ContentLicenseDtoV10.cs:24-36
        // C# uses 16-byte key + 16-byte IV (AES-128-CBC)
        let key_components = format!("{}{}{}{}", device_type, device_serial, account_id, asin);
        let hash = Sha256::digest(key_components.as_bytes());

        // Key = first 16 bytes, IV = last 16 bytes
        let key: [u8; 16] = hash[0..16].try_into().unwrap();
        let iv: [u8; 16] = hash[16..32].try_into().unwrap();

        // Decrypt using AES-128-CBC (no padding)
        // Reference: ContentLicenseDtoV10.cs:40-43 - uses Aes.Create() with 16-byte key
        type Aes128CbcDec = Decryptor<Aes128>;

        let cipher = Aes128CbcDec::new_from_slices(&key, &iv).map_err(|e| {
            LibationError::InvalidInput(format!("Failed to create cipher: {:?}", e))
        })?;

        // Decrypt in place
        let mut buffer = ciphertext.clone();
        let plaintext = cipher
            .decrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut buffer)
            .map_err(|e| {
                LibationError::InvalidInput(format!("Failed to decrypt license_response: {:?}", e))
            })?;

        // Remove null bytes and parse as ASCII
        // Reference: ContentLicenseDtoV10.cs:44
        let plaintext_no_nulls: Vec<u8> =
            plaintext.iter().copied().take_while(|&b| b != 0).collect();

        let json_str = String::from_utf8(plaintext_no_nulls).map_err(|e| {
            LibationError::InvalidInput(format!("Decrypted license is not valid UTF-8: {}", e))
        })?;

        // Never log json_str: it contains the AES key/iv for the audiobook.

        // Parse JSON to get Voucher
        // Reference: ContentLicenseDtoV10.cs:46 - VoucherDtoV10.FromJson(plainText)
        let voucher: Voucher = serde_json::from_str(&json_str).map_err(|e| {
            // Do not include json_str in the error: it contains the key/iv.
            LibationError::InvalidInput(format!("Failed to parse decrypted voucher JSON: {}", e))
        })?;

        // Convert voucher to KeyData
        // Check if key is hex (32 chars) or base64 (24 chars)
        if voucher.key.len() == 32 {
            // Hex-encoded (AAXC format from decrypted license_response)
            Self::from_hex(&voucher.key, voucher.iv.as_deref())
        } else {
            // Base64-encoded (from structured voucher field)
            Self::from_base64(&voucher.key, voucher.iv.as_deref())
        }
    }

    /// Determine file type based on key lengths
    ///
    /// # Reference
    /// C# code: DownloadOptions.cs:69-72
    /// ```csharp
    /// InputType
    /// = licInfo.DrmType is AudibleApi.Common.DrmType.Widevine ? AAXClean.FileType.Dash
    /// : licInfo.DrmType is AudibleApi.Common.DrmType.Adrm && licInfo.DecryptionKeys?.Length == 1 && licInfo.DecryptionKeys[0].KeyPart1.Length == 4 && licInfo.DecryptionKeys[0].KeyPart2 is null ? AAXClean.FileType.Aax
    /// : licInfo.DrmType is AudibleApi.Common.DrmType.Adrm && licInfo.DecryptionKeys?.Length == 1 && licInfo.DecryptionKeys[0].KeyPart1.Length == 16 && licInfo.DecryptionKeys[0].KeyPart2?.Length == 16 ? AAXClean.FileType.Aaxc
    /// : null;
    /// ```
    pub fn file_type(&self, drm_type: DrmType) -> FileType {
        match drm_type {
            DrmType::Widevine => FileType::Dash,
            DrmType::Mpeg => FileType::Mp3,
            DrmType::Adrm => {
                // AAX: 4-byte key, no IV
                if self.key_part_1.len() == 4 && self.key_part_2.is_none() {
                    FileType::Aax
                }
                // AAXC: 16-byte key + 16-byte IV
                else if self.key_part_1.len() == 16
                    && self.key_part_2.as_ref().map(|iv| iv.len()) == Some(16)
                {
                    FileType::Aaxc
                } else {
                    FileType::Unknown
                }
            }
            DrmType::None => FileType::Mp3,
        }
    }
}

/// File type based on DRM and key structure
/// Reference: AAXClean.FileType (external library), DownloadOptions.cs:39
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// Legacy AAX format (4-byte activation bytes)
    Aax,

    /// Current AAXC format (16-byte key pairs)
    Aaxc,

    /// MPEG-DASH format (Widevine)
    Dash,

    /// Unencrypted MP3
    Mp3,

    /// Unknown format
    Unknown,
}

// ============================================================================
// API FUNCTIONS
// ============================================================================

impl AudibleClient {
    fn download_license_request(quality: DownloadQuality, drm_type: DrmType) -> LicenseRequest {
        LicenseRequest::download(quality, drm_type)
    }

    fn should_retry_license_as_mpeg(error: &LibationError) -> bool {
        let message = error.to_string();
        message.contains("Sable(AssetInfos)")
            || message.contains("Unable to retrieve asset details")
            || message.contains("non_audio asset")
    }

    /// Request download license for an audiobook
    ///
    /// # Reference
    /// C# method: `Api.GetDownloadLicenseAsync(asin, quality, chapterTitles, drmType, ...)`
    /// Location: DownloadOptions.Factory.cs:57-112 - ChooseContent()
    ///
    /// # Endpoint
    /// `POST /1.0/content/{asin}/licenserequest`
    ///
    /// # Arguments
    /// * `asin` - Audible product ID
    /// * `request` - License request parameters (quality, DRM type, codecs)
    ///
    /// # Returns
    /// Content license with voucher/keys and metadata
    ///
    /// # Errors
    /// - `ApiRequestFailed` - API request failed
    /// - `InvalidApiResponse` - Response parsing failed
    /// - `MissingOfflineUrl` - License doesn't contain offline download URL
    ///
    /// # Example
    /// ```rust,no_run
    /// # use rust_core::api::client::AudibleClient;
    /// # use rust_core::api::auth::Account;
    /// # use rust_core::api::license::LicenseRequest;
    /// # async fn example() -> rust_core::error::Result<()> {
    /// let account = Account::new("user@example.com".to_string())?;
    /// let client = AudibleClient::new(account)?;
    /// let request = LicenseRequest::default();
    /// let license = client.get_download_license("B002V5D7B0", &request).await?;
    /// println!("DRM type: {:?}", license.drm_type);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_download_license(
        &self,
        asin: &str,
        request: &LicenseRequest,
    ) -> Result<ContentLicense> {
        let endpoint = format!("/1.0/content/{}/licenserequest", asin);

        // post_once, not post: a licence request must never be retried automatically.
        // Audible counts every one of them, and retrying is how an account gets throttled.
        let response: serde_json::Value = self.post_once(&endpoint, request).await?;

        // The API may wrap in "content_license" or return directly
        let license_json = response.get("content_license").unwrap_or(&response);

        // Audible reports refusals *inside* a 200 response, so the status has to be read
        // before the typed parse. Deserializing straight away turns "your download was
        // denied" into "missing field `content_url`", which is unactionable for a user.
        // Reference: AudibleApi/Api.Download.cs:290-317 - GetDownloadLicenseAsync
        if let Some(message) = license_json.get("message").and_then(|m| m.as_str()) {
            // A refusal can carry both a message and structured denial reasons; the
            // reasons say *why* (notably CustomerThrottled, which is temporary), so
            // prefer them and fall back to the bare message.
            let denial = Self::license_denied_error(license_json);
            if let LibationError::LicenseDenied { reason, throttled } = denial {
                if reason != "no reason given" {
                    return Err(LibationError::LicenseDenied { reason, throttled });
                }
            }
            let status = license_json
                .get("status_code")
                .and_then(|s| s.as_str())
                .unwrap_or("none");
            return Err(LibationError::ApiRequestFailed {
                message: format!(
                    "Audible rejected the license request: {message} \
                     [status={status}, fields={}]",
                    Self::json_keys(license_json)
                ),
                status_code: None,
                endpoint: Some(endpoint),
            });
        }

        match license_json.get("status_code").and_then(|s| s.as_str()) {
            Some(status) if status.eq_ignore_ascii_case("granted") => {}
            Some(status) if status.eq_ignore_ascii_case("denied") => {
                return Err(Self::license_denied_error(license_json));
            }
            Some(status) => {
                return Err(LibationError::InvalidApiResponse {
                    message: format!("Unrecognized license status \"{status}\" for {asin}"),
                    response_body: None,
                });
            }
            None => {
                // No status at all: report the shape rather than the contents, which
                // carry personally identifying fields.
                let keys = Self::json_keys(license_json);
                return Err(LibationError::InvalidApiResponse {
                    message: format!(
                        "License response for {asin} has no status_code (fields: {keys})"
                    ),
                    response_body: None,
                });
            }
        }

        serde_json::from_value(license_json.clone()).map_err(|e| {
            LibationError::InvalidApiResponse {
                message: format!(
                    "Failed to parse granted license for {asin}: {e} (fields: {})",
                    Self::json_keys(license_json)
                ),
                response_body: Some(license_json.to_string()),
            }
        })
    }

    /// Strip the customer id Audible embeds in denial messages, e.g.
    /// "License not granted to customer [A2HF4ET2OTZN6C] for asin [B002VA9SWS]".
    /// Upstream does the same before logging. Reference:
    /// AudibleApi/ApiExceptions/ContentLicenseDeniedException.cs:11,33-37
    fn redact_customer_id(message: &str) -> String {
        let mut out = String::with_capacity(message.len());
        let mut rest = message;
        while let Some(open) = rest.find('[') {
            let Some(close_rel) = rest[open..].find(']') else { break };
            let close = open + close_rel;
            let inner = &rest[open + 1..close];
            let looks_like_customer_id = inner.len() >= 11
                && inner.len() <= 21
                && inner.starts_with('A')
                && inner[1..].chars().all(|c| c.is_ascii_alphanumeric());
            out.push_str(&rest[..open]);
            out.push_str(if looks_like_customer_id {
                "[##############]"
            } else {
                &rest[open..=close]
            });
            rest = &rest[close + 1..];
        }
        out.push_str(rest);
        out
    }

    /// Turn a denied license into an error carrying Audible's own reasons.
    /// Reference: AudibleApi/ApiExceptions/ContentLicenseDeniedException.cs
    fn license_denied_error(license_json: &serde_json::Value) -> LibationError {
        let reasons = license_json
            .get("license_denial_reasons")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        let throttled = reasons.iter().any(|reason| {
            reason
                .get("rejectionReason")
                .and_then(|r| r.as_str())
                .is_some_and(|r| r.eq_ignore_ascii_case("CustomerThrottled"))
        });

        if throttled {
            return LibationError::LicenseDenied {
                reason: "Audible is temporarily throttling download licences for this \
                         account. Wait a while, then try again."
                    .to_string(),
                throttled: true,
            };
        }

        let detail = reasons
            .iter()
            .filter_map(|reason| {
                let validation = reason.get("validationType").and_then(|v| v.as_str());
                let rejection = reason.get("rejectionReason").and_then(|v| v.as_str());
                let message = reason.get("message").and_then(|v| v.as_str());
                match (validation, rejection, message) {
                    (None, None, None) => None,
                    _ => Some(format!(
                        "{}: {}{}",
                        validation.unwrap_or("Unknown"),
                        rejection.unwrap_or("denied"),
                        message
                            .map(|m| format!(" ({})", Self::redact_customer_id(m)))
                            .unwrap_or_default()
                    )),
                }
            })
            .collect::<Vec<_>>()
            .join("; ");

        LibationError::LicenseDenied {
            reason: if detail.is_empty() {
                "no reason given".to_string()
            } else {
                detail
            },
            throttled: false,
        }
    }

    /// Field names of a JSON object, for diagnostics that must not log the values.
    fn json_keys(value: &serde_json::Value) -> String {
        value
            .as_object()
            .map(|obj| obj.keys().cloned().collect::<Vec<_>>().join(","))
            .unwrap_or_else(|| "<not an object>".to_string())
    }

    /// A [`DownloadLicense`] for the legacy AAX path: a CDS download URL plus activation
    /// bytes, in place of an AAXC key/iv pair.
    ///
    /// Activation bytes are 4 bytes (8 hex characters) and are per-account, not per-book,
    /// so FFmpeg takes `-activation_bytes` here rather than `-audible_key`/`-audible_iv`.
    pub async fn aax_download_license(&self, asin: &str) -> Result<DownloadLicense> {
        let download_url = self.aax_workaround_url(asin).await?;

        let (locale, adp_token, private_key) = {
            let account_lock = self.account();
            let account = account_lock.lock().await;
            let identity = account.identity.as_ref().ok_or_else(|| {
                LibationError::InvalidState("No identity for AAX fallback".to_string())
            })?;
            (
                identity.locale.clone(),
                identity.adp_token.clone(),
                identity.device_private_key.clone(),
            )
        };

        let activation_bytes =
            crate::api::auth::get_activation_bytes(&locale, &adp_token, &private_key).await?;

        let key = hex::decode(&activation_bytes).map_err(|e| LibationError::InvalidApiResponse {
            message: format!("Activation bytes are not hex: {e}"),
            response_body: None,
        })?;

        Ok(DownloadLicense {
            drm_type: DrmType::Adrm,
            content_metadata: self.get_content_metadata(asin).await.unwrap_or_default(),
            // 4 bytes marks this as activation bytes; an AAXC key is 16.
            decryption_keys: Some(vec![KeyData { key_part_1: key, key_part_2: None }]),
            download_url,
        })
    }

    /// Codecs the legacy download service offers, best first.
    /// Reference: AudibleApi/Api.Download.cs:366 (CodecPreferenceOrder).
    const AAX_CODEC_PREFERENCE: [&'static str; 5] = [
        "LC_128_44100_stereo",
        "LC_64_44100_stereo",
        "LC_64_22050_stereo",
        "LC_32_22050_stereo",
        "AAX",
    ];

    /// The "AAX workaround": ask the legacy CDE service for a download URL instead of
    /// asking `licenserequest` for a licence.
    ///
    /// This is a completely separate service from the licence endpoint, authenticated by
    /// ADP signature rather than an access token, and it still serves accounts whose
    /// licence requests are refused with `CustomerThrottled` (verified 2026-09-03: a
    /// throttled account fetched an 806 MB `audio/vnd.audible.aax` asset seconds after
    /// being denied a licence for the same ASIN). The file is classic AAX, decrypted with
    /// activation bytes rather than an AAXC key/iv pair.
    ///
    /// Reference: AudibleApi/Api.Download.cs:334-368 (DownloadAaxWorkaroundAsync).
    pub async fn aax_workaround_url(&self, asin: &str) -> Result<String> {
        // Which encodings does this title actually have?
        let item: serde_json::Value = self
            .get_with_query(
                &format!("/1.0/library/{asin}"),
                &[("response_groups", "product_attrs,relationships")],
            )
            .await?;

        let codecs: Vec<String> = item
            .get("item")
            .unwrap_or(&item)
            .get("available_codecs")
            .and_then(|c| c.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|c| c.get("enhanced_codec").and_then(|e| e.as_str()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        // Order matters, so walk the preference list rather than intersecting.
        let codec = Self::AAX_CODEC_PREFERENCE
            .iter()
            .find(|preferred| codecs.iter().any(|c| c.eq_ignore_ascii_case(preferred)))
            .map(|c| c.to_string())
            .or_else(|| codecs.first().cloned())
            .ok_or_else(|| LibationError::InvalidApiResponse {
                message: format!("{asin} reports no available codecs; cannot fall back to AAX"),
                response_body: None,
            })?;

        let path = format!(
            "/FionaCDEServiceEngine/FSDownloadContent?type=AUDI&currentTransportMethod=WIFI&key={asin}&codec={codec}"
        );

        let (adp_token, private_key, store_domain) = {
            let account_lock = self.account();
            let account = account_lock.lock().await;
            let identity = account.identity.as_ref().ok_or_else(|| {
                LibationError::InvalidState("No identity for AAX workaround".to_string())
            })?;
            (
                identity.adp_token.clone(),
                identity.device_private_key.clone(),
                identity.locale.domain.clone(),
            )
        };

        let signature = crate::api::signing::sign_request(
            "GET",
            &path,
            "",
            &adp_token,
            &private_key,
            chrono::Utc::now(),
        )?;

        // The answer is a 302; following it would consume the signed URL here rather
        // than handing it to the downloader.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| LibationError::InternalError(format!("HTTP client: {e}")))?;

        let mut request = client.get(format!("https://cde-ta-g7g.amazon.com{path}"));
        for (name, value) in signature.headers() {
            request = request.header(name, value);
        }

        let response = request.send().await.map_err(|e| LibationError::NetworkError {
            message: format!("AAX workaround request failed: {e}"),
            is_transient: e.is_timeout() || e.is_connect(),
        })?;

        if response.status() != reqwest::StatusCode::FOUND {
            return Err(LibationError::ApiRequestFailed {
                message: format!(
                    "AAX workaround expected a 302 redirect, got HTTP {}",
                    response.status()
                ),
                status_code: Some(response.status().as_u16()),
                endpoint: Some("/FionaCDEServiceEngine/FSDownloadContent".to_string()),
            });
        }

        let url = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|l| l.to_str().ok())
            .ok_or_else(|| LibationError::InvalidApiResponse {
                message: "AAX workaround redirect carried no Location".to_string(),
                response_body: None,
            })?;

        // The content belongs to the store it was bought from, not to whichever
        // marketplace this identity happens to be registered with. `Locale::domain` is
        // the full store host ("audible.co.uk"), so swap the whole host.
        Ok(url.replace("cds.audible.com", &format!("cds.{store_domain}")))
    }

    /// Build download license with decryption keys
    ///
    /// # Reference
    /// C# method: DownloadOptions.Factory.cs:57-112 - ChooseContent()
    /// C# class: DownloadOptions.Factory.cs:41-55 - LicenseInfo
    ///
    /// This is a high-level method that:
    /// 1. Requests license from API
    /// 2. Parses voucher to extract keys
    /// 3. Validates download URL is present
    /// 4. Returns structured DownloadLicense
    ///
    /// # Arguments
    /// * `asin` - Audible product ID
    /// * `quality` - Download quality tier
    /// * `prefer_widevine` - Request Widevine DRM if available
    ///
    /// # Returns
    /// Download license ready for use with download/decrypt operations
    ///
    /// # Errors
    /// - `ApiRequestFailed` - License request failed
    /// - `MissingOfflineUrl` - No download URL in license
    /// - `InvalidInput` - Invalid voucher data
    pub async fn build_download_license(
        &self,
        asin: &str,
        quality: DownloadQuality,
        prefer_widevine: bool,
    ) -> Result<DownloadLicense> {
        let drm_type = if prefer_widevine {
            DrmType::Widevine
        } else {
            DrmType::Adrm
        };
        let request = Self::download_license_request(quality, drm_type);

        // Podcast episodes are plain MP3 assets. Audible rejects ADRM for them
        // with a Sable asset-details error, then accepts a Download+Mpeg request.
        let license = match self.get_download_license(asin, &request).await {
            Ok(license) => license,
            Err(error)
                if !prefer_widevine
                    && drm_type == DrmType::Adrm
                    && Self::should_retry_license_as_mpeg(&error) =>
            {
                let mpeg_request = Self::download_license_request(quality, DrmType::Mpeg);
                self.get_download_license(asin, &mpeg_request).await?
            }
            // Audible refused to licence the download. Since 2026-09-02 it refuses every
            // third-party client with `CustomerThrottled` while serving the official app,
            // and the discriminator is the device registration that issued the token —
            // nothing in the request can change it. The legacy AAX service is unaffected,
            // so take that route rather than failing the download.
            //
            // Upstream has this fallback but only reaches it when `licenserequest`
            // *throws*; a refusal arrives as HTTP 200 with `status_code: "Denied"`, so
            // its escape hatch never fires for the current outage.
            Err(error @ LibationError::LicenseDenied { .. }) => {
                eprintln!("Licence denied ({error}); falling back to the AAX service");
                return self.aax_download_license(asin).await.map_err(|fallback| {
                    // Report the original refusal: it is the actionable one.
                    eprintln!("AAX fallback also failed: {fallback}");
                    error
                });
            }
            Err(error) => return Err(error),
        };

        // Extract download URL
        // Reference: DownloadOptions.cs:61-62
        let download_url = license
            .content_metadata
            .content_url
            .as_ref()
            .and_then(|url| url.offline_url.clone())
            .ok_or(LibationError::MissingOfflineUrl)?;

        // Parse voucher to keys
        // Reference: DownloadOptions.Factory.cs:46-54 - DecryptionKeys = ToKeys(license.Voucher)
        let decryption_keys = if let Some(ref voucher) = license.voucher {
            // Structured voucher with key/iv fields (already decrypted)
            let key_data = KeyData::from_base64(&voucher.key, voucher.iv.as_deref())?;
            Some(vec![key_data])
        } else if license.drm_type.is_encrypted() {
            if let Some(ref license_response) = license.license_response {
                // For AAXC files, the license_response is AES-encrypted
                // Need device info to decrypt
                // Reference: ContentLicenseDtoV10.cs:13-14, 19-47
                let account_lock = self.account();
                let account = account_lock.lock().await;
                let identity = account.identity.as_ref().ok_or_else(|| {
                    LibationError::InvalidState(
                        "No identity in account - cannot decrypt license_response".to_string(),
                    )
                })?;

                let key_data = KeyData::from_license_response(
                    license_response,
                    &identity.device_type,
                    &identity.device_serial_number,
                    &identity.amazon_account_id,
                    asin,
                )?;
                Some(vec![key_data])
            } else {
                None
            }
        } else {
            None
        };

        Ok(DownloadLicense {
            drm_type: license.drm_type,
            content_metadata: license.content_metadata,
            decryption_keys,
            download_url,
        })
    }

    /// Get download URL for an audiobook
    ///
    /// # Reference
    /// This is a simplified convenience method that combines license request and URL extraction.
    ///
    /// # Arguments
    /// * `asin` - Audible product ID
    /// * `quality` - Download quality tier
    ///
    /// # Returns
    /// Direct CDN download URL (may be temporary/signed)
    ///
    /// # Errors
    /// - `ApiRequestFailed` - License request failed
    /// - `MissingOfflineUrl` - No download URL available
    ///
    /// # Note
    /// Download URLs may expire after a period (typically 24 hours).
    /// For long-term storage, keep the ASIN and re-request license when needed.
    pub async fn get_download_url(&self, asin: &str, quality: DownloadQuality) -> Result<String> {
        let license = self.build_download_license(asin, quality, false).await?;
        Ok(license.download_url)
    }

    /// Determine DRM type and file format from license
    ///
    /// # Reference
    /// C# code: DownloadOptions.cs:69-76 - InputType detection
    ///
    /// This method inspects the license and decryption keys to determine:
    /// - DRM type (Adrm, Widevine, None)
    /// - File format (AAX, AAXC, DASH, MP3)
    ///
    /// Detection logic:
    /// - Widevine → DASH
    /// - Adrm + 4-byte key, no IV → AAX
    /// - Adrm + 16-byte key + 16-byte IV → AAXC
    /// - None → MP3
    ///
    /// # Arguments
    /// * `license` - Download license to analyze
    ///
    /// # Returns
    /// Detected file type
    pub fn determine_file_type(license: &DownloadLicense) -> FileType {
        if let Some(keys) = &license.decryption_keys {
            if !keys.is_empty() {
                return keys[0].file_type(license.drm_type);
            }
        }

        // No keys - check DRM type
        match license.drm_type {
            DrmType::Widevine => FileType::Dash,
            DrmType::Mpeg => FileType::Mp3,
            DrmType::None => FileType::Mp3,
            _ => FileType::Unknown,
        }
    }

    /// Determine output format based on DRM and configuration
    ///
    /// # Reference
    /// C# code: DownloadOptions.cs:75-79
    /// ```csharp
    /// OutputFormat
    ///     = licInfo.DrmType is not AudibleApi.Common.DrmType.Adrm and not AudibleApi.Common.DrmType.Widevine ||
    ///     (config.AllowLibationFixup && config.DecryptToLossy && licInfo.ContentMetadata.ContentReference.Codec != AudibleApi.Codecs.AC_4)
    ///     ? OutputFormat.Mp3
    ///     : OutputFormat.M4b;
    /// ```
    ///
    /// # Arguments
    /// * `license` - Download license
    /// * `convert_to_mp3` - Whether to convert to lossy MP3 format
    ///
    /// # Returns
    /// Output format (M4b or Mp3)
    pub fn determine_output_format(
        license: &DownloadLicense,
        convert_to_mp3: bool,
    ) -> OutputFormat {
        // Unencrypted content is always MP3
        if !license.drm_type.is_encrypted() {
            return OutputFormat::Mp3;
        }

        // Convert to MP3 if requested, unless it's AC-4 spatial audio
        if convert_to_mp3 {
            if let Some(ref content_ref) = license.content_metadata.content_reference {
                // Audible reports the codec as a free-form string ("aac", "ac-4",
                // "LC_128_44100_stereo", …); only AC-4 spatial audio is worth keeping.
                let is_ac4 = content_ref
                    .codec
                    .as_deref()
                    .is_some_and(|codec| codec.to_ascii_lowercase().contains("ac-4")
                        || codec.to_ascii_lowercase().contains("ac_4"));
                if !is_ac4 {
                    return OutputFormat::Mp3;
                }
            } else {
                // No codec info available, safe to convert to MP3
                return OutputFormat::Mp3;
            }
        }

        // Default to M4B
        OutputFormat::M4b
    }
}

/// Output audio format
/// Reference: AaxDecrypter/OutputFormat.cs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// M4B format (Apple audiobook)
    M4b,

    /// MP3 format (lossy compression)
    Mp3,
}

// ============================================================================
// WIDEVINE LICENSE EXCHANGE (Future Implementation)
// ============================================================================

/// Widevine license challenge and response
///
/// # Reference
/// C# implementation: DownloadOptions.Factory.cs:98-102
/// ```csharp
/// using var session = cdm.OpenSession();
/// var challenge = session.GetLicenseChallenge(dash);
/// var licenseMessage = await api.WidevineDrmLicense(libraryBook.Book.AudibleProductId, challenge);
/// var keys = session.ParseLicense(licenseMessage);
/// ```
///
/// # TODO
/// This requires porting or integrating with a Widevine CDM library.
/// Options:
/// 1. Port Libation's Widevine/Cdm.cs implementation
/// 2. Use existing Rust Widevine library (if available)
/// 3. Interface with Python pywidevine library via FFI
///
/// Key files to port:
/// - AudibleUtilities/Widevine/Cdm.cs
/// - AudibleUtilities/Widevine/Device.cs
/// - AudibleUtilities/Widevine/LicenseProtocol.cs (protobuf definitions)
/// - AudibleUtilities/Widevine/MpegDash.cs (DASH manifest parsing)
impl AudibleClient {
    /// Request Widevine DRM license (exchange challenge for keys)
    ///
    /// # Reference
    /// C# method: `Api.WidevineDrmLicense(asin, challenge)`
    /// Location: DownloadOptions.Factory.cs:100
    ///
    /// # Endpoint
    /// `POST /1.0/content/{asin}/licenseRequest`
    /// Content-Type: application/octet-stream
    ///
    /// # Arguments
    /// * `asin` - Audible product ID
    /// * `challenge` - Widevine license challenge (binary protobuf)
    ///
    /// # Returns
    /// Widevine license response (binary protobuf)
    ///
    /// # Errors
    /// - `NotImplemented` - Widevine support not yet implemented
    /// - `ApiRequestFailed` - License exchange failed
    ///
    /// # Note
    /// This requires Widevine CDM integration which is not yet implemented.
    /// See TODO comments above for implementation options.
    pub async fn widevine_license_exchange(
        &self,
        _asin: &str,
        _challenge: &[u8],
    ) -> Result<Vec<u8>> {
        Err(LibationError::not_implemented(
            "Widevine license exchange requires CDM integration (see license.rs TODO)",
        ))
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_data_file_type_aax() {
        let key_data = KeyData {
            key_part_1: vec![0x01, 0x02, 0x03, 0x04], // 4 bytes
            key_part_2: None,
        };

        assert_eq!(key_data.file_type(DrmType::Adrm), FileType::Aax);
    }

    #[test]
    fn test_key_data_file_type_aaxc() {
        let key_data = KeyData {
            key_part_1: vec![0; 16],       // 16 bytes
            key_part_2: Some(vec![0; 16]), // 16 bytes
        };

        assert_eq!(key_data.file_type(DrmType::Adrm), FileType::Aaxc);
    }

    #[test]
    fn test_key_data_file_type_widevine() {
        let key_data = KeyData {
            key_part_1: vec![0; 16],
            key_part_2: Some(vec![0; 16]),
        };

        assert_eq!(key_data.file_type(DrmType::Widevine), FileType::Dash);
    }

    #[test]
    fn test_key_data_from_base64() {
        use base64::{engine::general_purpose, Engine as _};

        let key = general_purpose::STANDARD.encode(b"testkey1234567890");
        let iv = general_purpose::STANDARD.encode(b"testiv1234567890");

        let key_data = KeyData::from_base64(&key, Some(&iv)).unwrap();
        assert_eq!(key_data.key_part_1, b"testkey1234567890");
        assert_eq!(key_data.key_part_2, Some(b"testiv1234567890".to_vec()));
    }

    #[test]
    fn denial_messages_do_not_leak_the_customer_id() {
        let raw = "License not granted to customer [A2HF4ET2OTZN6C] for asin [B002VA9SWS]";
        let clean = AudibleClient::redact_customer_id(raw);
        assert!(!clean.contains("A2HF4ET2OTZN6C"), "customer id must not survive: {clean}");
        // The ASIN is not personal and stays: it is what makes the error actionable.
        assert!(clean.contains("[B002VA9SWS]"), "asin should be kept: {clean}");
        assert!(clean.contains("[##############]"));

        // Nothing bracketed and harmless should be mangled.
        assert_eq!(AudibleClient::redact_customer_id("no brackets"), "no brackets");
        assert_eq!(AudibleClient::redact_customer_id("[Client] denied"), "[Client] denied");
        // An unclosed bracket must not eat the rest of the message.
        assert_eq!(AudibleClient::redact_customer_id("open [ only"), "open [ only");
    }

    #[test]
    fn test_license_request_default() {
        let request = LicenseRequest::default();
        assert_eq!(request.quality, DownloadQuality::High);
        assert_eq!(request.consumption_type, ConsumptionType::Download);
        assert_eq!(
            request.supported_media_features.chapter_titles_type,
            ChapterTitlesType::Tree
        );

        // The body shape is the contract: Audible returns a license without a
        // content_url for a flat request, which surfaces as an unreadable parse error.
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["tenant_id"], "Audible");
        assert_eq!(json["consumption_type"], "Download");
        assert_eq!(json["spatial"], false);
        assert!(json["response_groups"].as_str().unwrap().contains("chapter_info"));
        let features = &json["supported_media_features"];
        assert_eq!(features["drm_types"][0], "Adrm");
        assert_eq!(features["drm_types"][1], "Mpeg");
        assert_eq!(features["codecs"][0], "mp4a.40.2");
        assert_eq!(features["chapter_titles_type"], "Tree");
        assert_eq!(features["previews"], false);
        assert_eq!(features["catalog_samples"], false);
    }

    // ============================================================================
    // Integration Tests (require real API credentials)
    // ============================================================================

    /// Integration test: Download book B07T2F8VJM using test credentials
    ///
    /// This test demonstrates the complete download flow:
    /// 1. Load authenticated account from test fixture
    /// 2. Request download license for ASIN B07T2F8VJM
    /// 3. Extract download URL from license
    /// 4. Verify URL is valid with HEAD request
    /// 5. Display file size and content type
    ///
    /// # Reference
    /// - C# equivalent: DownloadOptions.Factory.cs:57-112 - ChooseContent()
    /// - C# equivalent: AudiobookDownloadBase.cs:178-216 - OpenNetworkFileStream()
    ///
    /// # Run with
    /// ```bash
    /// cargo test --ignored test_download_book_b07t2f8vjm -- --nocapture
    /// ```
    ///
    /// # Test Book: B07T2F8VJM
    /// Title: "Atomic Habits" by James Clear
    /// This is a popular book chosen for testing as it's widely available
    #[tokio::test]
    #[ignore] // Only run with --ignored flag since it requires real API credentials
    async fn test_download_book_b07t2f8vjm() {
        use crate::api::auth::Account;
        use crate::api::client::AudibleClient;
        use crate::api::registration::RegistrationResponse;

        println!("\n=== Download Book B07T2F8VJM Integration Test ===\n");

        // Step 1: Load test account from fixture
        println!("📦 Loading test account from fixture...");
        const TEST_FIXTURE: &str = include_str!("../../test_fixtures/registration_response.json");

        let reg_response = RegistrationResponse::from_json(TEST_FIXTURE)
            .expect("Failed to parse registration response fixture");

        let locale = crate::api::auth::Locale::us();
        let identity = reg_response
            .to_identity(locale)
            .expect("Failed to convert registration to identity");

        // Create account with identity
        let mut account =
            Account::new(identity.customer_info.user_id.clone()).expect("Failed to create account");
        account.set_account_name(identity.customer_info.name.clone());
        account.set_identity(identity);

        println!("✅ Account loaded: {}", account.masked_log_entry());

        // Step 2: Create API client
        println!("\n🔧 Creating Audible API client...");
        let client = AudibleClient::new(account).expect("Failed to create API client");
        println!("✅ Client created");

        // Step 3: Request download license for B07T2F8VJM
        const TEST_ASIN: &str = "B07T2F8VJM";
        println!("\n📥 Requesting download license for ASIN: {}", TEST_ASIN);
        println!("   Quality: High");

        let license_result = client
            .build_download_license(
                TEST_ASIN,
                DownloadQuality::High,
                false, // Don't prefer Widevine (use AAX/AAXC)
            )
            .await;

        let license = match license_result {
            Ok(lic) => {
                println!("✅ License acquired successfully");
                lic
            }
            Err(e) => {
                eprintln!("❌ Failed to get download license: {:?}", e);
                panic!("License request failed - check credentials and API status");
            }
        };

        // Step 4: Display license information
        println!("\n📋 License Information:");
        println!("   DRM Type: {:?}", license.drm_type);
        println!(
            "   Content URL: {}",
            if license.download_url.len() > 100 {
                format!("{}...", &license.download_url[..100])
            } else {
                license.download_url.clone()
            }
        );

        // Check decryption keys
        if let Some(ref keys) = license.decryption_keys {
            let file_type = AudibleClient::determine_file_type(&license);
            println!("   File Type: {:?}", file_type);
            println!("   Key Count: {}", keys.len());
            if !keys.is_empty() {
                println!("   Key 1 Length: {} bytes", keys[0].key_part_1.len());

                // Display activation bytes/keys as hex
                if keys[0].key_part_1.len() == 4 {
                    // AAX: 4-byte activation bytes
                    let hex_bytes = keys[0]
                        .key_part_1
                        .iter()
                        .map(|b| format!("{:02x}", b))
                        .collect::<String>();
                    println!("   Activation Bytes (AAX): {}", hex_bytes);
                } else if keys[0].key_part_1.len() == 16 {
                    // AAXC: 16-byte key
                    let hex_key = keys[0]
                        .key_part_1
                        .iter()
                        .map(|b| format!("{:02x}", b))
                        .collect::<String>();
                    println!("   Key (AAXC): {}", hex_key);
                }

                if let Some(ref key2) = keys[0].key_part_2 {
                    println!("   Key 2 Length: {} bytes", key2.len());
                    if key2.len() == 16 {
                        let hex_iv = key2
                            .iter()
                            .map(|b| format!("{:02x}", b))
                            .collect::<String>();
                        println!("   IV (AAXC): {}", hex_iv);
                    }
                }
            }
        } else {
            println!("   Decryption Keys: None (unencrypted or Widevine)");
        }

        // Display content metadata
        let metadata = &license.content_metadata;
        println!("\n📊 Content Metadata:");

        if let Some(ref content_ref) = metadata.content_reference {
            println!("   Codec: {:?}", content_ref.codec);
            println!("   ACR: {:?}", content_ref.acr);
            println!("   Version: {:?}", content_ref.version);
        } else {
            println!("   Content Reference: Not available (license response only)");
        }

        if let Some(ref chapter_info) = metadata.chapter_info {
            println!("   Chapter Count: {}", chapter_info.chapters.len());
            println!("   Brand Intro: {}ms", chapter_info.brand_intro_duration_ms);
            println!("   Brand Outro: {}ms", chapter_info.brand_outro_duration_ms);
            println!("   Runtime: {}ms", chapter_info.runtime_length_ms);

            // Display first few chapters
            if !chapter_info.chapters.is_empty() {
                println!("\n   First Chapters:");
                for (i, chapter) in chapter_info.chapters.iter().take(3).enumerate() {
                    println!(
                        "     {}. {} ({}ms - {}ms)",
                        i + 1,
                        chapter.title,
                        chapter.start_offset_ms,
                        chapter.start_offset_ms + chapter.length_ms
                    );
                }
                if chapter_info.chapters.len() > 3 {
                    println!(
                        "     ... and {} more chapters",
                        chapter_info.chapters.len() - 3
                    );
                }
            }
        } else {
            println!("   Chapter Info: Not available (call get_content_metadata separately)");
        }

        // Step 5: Verify download URL with HEAD request
        println!("\n🌐 Verifying download URL...");
        let head_result = reqwest::Client::new()
            .head(&license.download_url)
            .send()
            .await;

        match head_result {
            Ok(response) => {
                println!("✅ URL is accessible");
                println!("   Status: {}", response.status());

                // Get file size from Content-Length header
                if let Some(content_length) = response.headers().get("content-length") {
                    if let Ok(size_str) = content_length.to_str() {
                        if let Ok(size) = size_str.parse::<u64>() {
                            let size_mb = size as f64 / (1024.0 * 1024.0);
                            println!("   File Size: {:.2} MB ({} bytes)", size_mb, size);
                        }
                    }
                }

                // Get content type
                if let Some(content_type) = response.headers().get("content-type") {
                    if let Ok(ct) = content_type.to_str() {
                        println!("   Content Type: {}", ct);
                    }
                }

                // Get other useful headers
                if let Some(server) = response.headers().get("server") {
                    if let Ok(s) = server.to_str() {
                        println!("   Server: {}", s);
                    }
                }

                if let Some(last_modified) = response.headers().get("last-modified") {
                    if let Ok(lm) = last_modified.to_str() {
                        println!("   Last Modified: {}", lm);
                    }
                }
            }
            Err(e) => {
                eprintln!("❌ Failed to verify URL: {:?}", e);
                panic!("URL verification failed - download URL may be invalid");
            }
        }

        // Step 6: Save license info to test fixture for future use
        println!("\n💾 Saving license info...");
        let mut license_json = serde_json::json!({
            "asin": TEST_ASIN,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "drm_type": format!("{:?}", license.drm_type),
            "download_url": license.download_url,
            "file_type": format!("{:?}", AudibleClient::determine_file_type(&license)),
            "has_decryption_keys": license.decryption_keys.is_some(),
        });

        // Add optional fields if available
        if let Some(ref content_ref) = metadata.content_reference {
            license_json["codec"] = serde_json::json!(format!("{:?}", content_ref.codec));
            license_json["acr"] = serde_json::json!(&content_ref.acr);
            license_json["version"] = serde_json::json!(&content_ref.version);
        }

        if let Some(ref chapter_info) = metadata.chapter_info {
            license_json["chapter_count"] = serde_json::json!(chapter_info.chapters.len());
            license_json["runtime_ms"] = serde_json::json!(chapter_info.runtime_length_ms);
        }

        // Save activation bytes hex if available
        if let Some(ref keys) = license.decryption_keys {
            if !keys.is_empty() && keys[0].key_part_1.len() == 4 {
                let hex = keys[0]
                    .key_part_1
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>();
                license_json["activation_bytes_hex"] = serde_json::json!(hex);
            }
        }

        // Try to save to multiple possible locations
        let cache_paths = [
            "test_fixtures/download_license_b07t2f8vjm.json",
            "/tmp/librisync_download_license_b07t2f8vjm.json",
        ];

        let license_json_str = serde_json::to_string_pretty(&license_json).unwrap();
        for path in &cache_paths {
            if let Ok(_) = std::fs::write(path, &license_json_str) {
                println!("   ✅ Saved to: {}", path);
                break;
            }
        }

        println!("\n✅ Test Complete!");
        println!("\n📝 Summary:");
        println!("   • License acquired successfully");
        println!("   • Download URL verified and accessible");
        println!("   • Ready for download and decryption");
        println!("\n💡 Next steps:");
        println!("   • Use download URL to fetch the audiobook file");
        println!("   • Use decryption keys to decrypt AAX/AAXC file");
        println!("   • Convert to M4B using FFmpeg");
    }
}
