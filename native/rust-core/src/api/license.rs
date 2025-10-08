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

use crate::error::{LibationError, Result};
use crate::api::client::AudibleClient;
use crate::api::content::{
    DrmType, Codec, DownloadQuality, ChapterTitlesType, ContentMetadata
};
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

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
    /// Download quality (Normal, High, Extreme)
    /// Reference: DownloadOptions.Factory.cs:59 - dlQuality
    #[serde(rename = "quality")]
    pub quality: DownloadQuality,

    /// Consumption type (Download vs Streaming)
    /// Always "Download" for offline use
    #[serde(rename = "consumption_type")]
    pub consumption_type: ConsumptionType,

    /// DRM type preference (Adrm, Mpeg/Widevine, or None)
    /// Reference: DownloadOptions.Factory.cs:80 - DrmType.Widevine or implicit Adrm
    #[serde(rename = "drm_type", skip_serializing_if = "Option::is_none")]
    pub drm_type: Option<DrmType>,

    /// Chapter titles type (Flat or Tree)
    /// Reference: DownloadOptions.Factory.cs:80 - ChapterTitlesType.Tree
    #[serde(rename = "chapter_titles_type", skip_serializing_if = "Option::is_none")]
    pub chapter_titles_type: Option<ChapterTitlesType>,

    /// Request spatial audio if available
    /// Reference: DownloadOptions.Factory.cs:82 - config.RequestSpatial
    #[serde(rename = "request_spatial", skip_serializing_if = "Option::is_none")]
    pub request_spatial: Option<bool>,

    /// Preferred AAC codec (AAC_LC or xHE_AAC)
    /// Reference: DownloadOptions.Factory.cs:72 - aacCodecChoice
    #[serde(rename = "aac_codec", skip_serializing_if = "Option::is_none")]
    pub aac_codec: Option<Codec>,

    /// Preferred spatial codec (EC_3 or AC_4)
    /// Reference: DownloadOptions.Factory.cs:74 - spatialCodecChoice
    #[serde(rename = "spatial_codec", skip_serializing_if = "Option::is_none")]
    pub spatial_codec: Option<Codec>,
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

impl Default for LicenseRequest {
    fn default() -> Self {
        Self {
            quality: DownloadQuality::High,
            consumption_type: ConsumptionType::Download,
            drm_type: None,
            chapter_titles_type: Some(ChapterTitlesType::Tree),
            request_spatial: Some(false),
            aac_codec: Some(Codec::AacLc),
            spatial_codec: Some(Codec::Ec3),
        }
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
    /// Create KeyData from Base64 encoded key and IV
    ///
    /// # Reference
    /// C# code: DownloadOptions.Factory.cs:53-54
    /// ```csharp
    /// private static KeyData[]? ToKeys(VoucherDtoV10? voucher)
    ///     => voucher is null ? null : [new KeyData(voucher.Key, voucher.Iv)];
    /// ```
    pub fn from_base64(key: &str, iv: Option<&str>) -> Result<Self> {
        use base64::{Engine as _, engine::general_purpose};

        let key_bytes = general_purpose::STANDARD
            .decode(key)
            .map_err(|e| LibationError::InvalidInput(format!("Invalid base64 key: {}", e)))?;

        let iv_bytes = if let Some(iv_str) = iv {
            Some(general_purpose::STANDARD
                .decode(iv_str)
                .map_err(|e| LibationError::InvalidInput(format!("Invalid base64 IV: {}", e)))?)
        } else {
            None
        };

        Ok(Self {
            key_part_1: key_bytes,
            key_part_2: iv_bytes,
        })
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
    /// let client = AudibleClient::new(Account::default())?;
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

        let response: serde_json::Value = self.post(&endpoint, request).await?;

        // Parse license response
        // The API may wrap in "content_license" or return directly
        let license_json = response
            .get("content_license")
            .unwrap_or(&response);

        serde_json::from_value(license_json.clone())
            .map_err(|e| LibationError::InvalidApiResponse {
                message: format!("Failed to parse content license: {}", e),
                response_body: Some(license_json.to_string()),
            })
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
        // Build license request
        // Reference: DownloadOptions.Factory.cs:59-84
        let mut request = LicenseRequest {
            quality,
            consumption_type: ConsumptionType::Download,
            chapter_titles_type: Some(ChapterTitlesType::Tree),
            request_spatial: Some(false),
            aac_codec: Some(Codec::AacLc),
            spatial_codec: Some(Codec::Ec3),
            drm_type: None,
        };

        // Request Widevine if preferred and supported
        // Reference: DownloadOptions.Factory.cs:68-112
        if prefer_widevine {
            request.drm_type = Some(DrmType::Widevine);
        }

        // Request license
        let license = self.get_download_license(asin, &request).await?;

        // Extract download URL
        // Reference: DownloadOptions.cs:61-62
        let download_url = license
            .content_metadata
            .content_url
            .offline_url
            .clone()
            .ok_or(LibationError::MissingOfflineUrl)?;

        // Parse voucher to keys
        // Reference: DownloadOptions.Factory.cs:46-54 - DecryptionKeys = ToKeys(license.Voucher)
        let decryption_keys = if let Some(ref voucher) = license.voucher {
            let key_data = KeyData::from_base64(
                &voucher.key,
                voucher.iv.as_deref(),
            )?;
            Some(vec![key_data])
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
    pub fn determine_output_format(license: &DownloadLicense, convert_to_mp3: bool) -> OutputFormat {
        // Unencrypted content is always MP3
        if !license.drm_type.is_encrypted() {
            return OutputFormat::Mp3;
        }

        // Convert to MP3 if requested, unless it's AC-4 spatial audio
        if convert_to_mp3 {
            let codec = license.content_metadata.content_reference.codec;
            if !matches!(codec, Codec::Ac4) {
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
            "Widevine license exchange requires CDM integration (see license.rs TODO)"
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
            key_part_1: vec![0; 16], // 16 bytes
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
        use base64::{Engine as _, engine::general_purpose};

        let key = general_purpose::STANDARD.encode(b"testkey1234567890");
        let iv = general_purpose::STANDARD.encode(b"testiv1234567890");

        let key_data = KeyData::from_base64(&key, Some(&iv)).unwrap();
        assert_eq!(key_data.key_part_1, b"testkey1234567890");
        assert_eq!(key_data.key_part_2, Some(b"testiv1234567890".to_vec()));
    }

    #[test]
    fn test_license_request_default() {
        let request = LicenseRequest::default();
        assert_eq!(request.quality, DownloadQuality::High);
        assert_eq!(request.consumption_type, ConsumptionType::Download);
        assert_eq!(request.chapter_titles_type, Some(ChapterTitlesType::Tree));
    }
}
