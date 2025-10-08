//! Error types for RN Audible
//!
//! This module defines error types using thiserror for ergonomic error handling.
//! Errors are categorized by domain (API, crypto, storage, etc.) for better
//! error handling and reporting.
//!
//! ## Mapping from Libation C# Exceptions
//!
//! This error type system is based on exception patterns from the original Libation C# codebase.
//! The following C# exceptions have been mapped to Rust error variants:
//!
//! ### API/Network Errors (from AudibleUtilities, Cdm.Api.cs)
//! - `ApiErrorException` → `ApiError`
//! - `HttpRequestException` → `NetworkError`, `ApiRequestFailed`
//! - `WebException` → `WebError`
//!
//! ### Data Validation (from AudibleUtilities)
//! - `ImportValidationException` → `ImportValidation`
//! - `InvalidDataException` → `InvalidData`
//!
//! ### DRM/Crypto (from AudibleUtilities/Widevine, AaxDecrypter)
//! - `InvalidDataException` (CDM-related) → `WidevineCdmError`, `InvalidCdmFile`
//! - Decryption failures → `DecryptionFailed`
//!
//! ### Download/Network (from AaxDecrypter/NetworkFileStream.cs)
//! - `WebException` → `WebError`, `DownloadFailed`
//! - `HttpIOException` → `DownloadInterrupted`, `NetworkError`
//! - Resume failures → `DownloadInterrupted`
//!
//! ### File Operations (from FileLiberator, FileManager)
//! - `DirectoryNotFoundException` → `FileNotFound`, `InvalidPath`
//! - `IOException` → `FileIoError`
//! - `UnauthorizedAccessException` → `PermissionDenied`
//!
//! ### Database (from DataLayer)
//! - `InvalidOperationException` (DB context) → `DatabaseError`
//! - Entity Framework exceptions → `SqlxError` (via `#[from]`)
//!
//! ### Configuration/State (from LibationFileManager)
//! - `InvalidOperationException` → `InvalidState`
//! - `ApplicationException` → `ConfigurationError`
//! - `NullReferenceException` → `MissingRequiredField`
//!
//! ### Audio Processing (from FileLiberator, AaxDecrypter)
//! - FFmpeg failures → `FfmpegError`, `FfmpegNotFound`
//! - Format detection failures → `UnsupportedAudioFormat`, `InvalidAudioFile`

use thiserror::Error;

/// Result type alias using our LibationError type
pub type Result<T> = std::result::Result<T, LibationError>;

/// Main error type for RN Audible
///
/// This enum provides comprehensive error handling for all operations in the application.
/// Each variant includes descriptive error messages and relevant context.
#[derive(Error, Debug)]
pub enum LibationError {
    // ===== API Errors =====
    // Corresponds to C# ApiErrorException, authentication failures in ApiExtended.cs

    /// Authentication with Audible API failed (maps to C# authentication exceptions)
    #[error("API authentication failed: {message}")]
    AuthenticationFailed {
        message: String,
        /// Account ID if available
        account_id: Option<String>,
    },

    /// Generic API request failure (maps to C# HttpRequestException, ApiErrorException)
    #[error("API request failed: {message}")]
    ApiRequestFailed {
        message: String,
        /// HTTP status code if available
        status_code: Option<u16>,
        /// API endpoint that failed
        endpoint: Option<String>,
    },

    /// API returned invalid or unexpected response format
    #[error("Invalid API response: {message}")]
    InvalidApiResponse {
        message: String,
        /// Response body snippet for debugging
        response_body: Option<String>,
    },

    /// API rate limiting (HTTP 429)
    #[error("API rate limit exceeded. Retry after {retry_after_seconds} seconds")]
    RateLimitExceeded {
        /// Seconds to wait before retrying
        retry_after_seconds: u64,
        /// Endpoint that was rate limited
        endpoint: String,
    },

    /// Account not found in local database
    #[error("Account not found: {0}")]
    AccountNotFound(String),

    /// Account validation failed (maps to C# InvalidOperationException in DownloadDecryptBook.cs)
    #[error("Account validation failed: missing {field} for book '{book_title}'")]
    AccountValidationFailed {
        field: String,
        book_title: String,
    },

    /// Invalid activation bytes format
    #[error("Invalid activation bytes: {0}")]
    InvalidActivationBytes(String),

    /// Access token expired or invalid
    #[error("Token expired or invalid")]
    TokenExpired,

    /// API returned unknown Audible domain
    #[error("Unknown Audible API domain: {0}")]
    UnknownApiDomain(String),

    // ===== Crypto/DRM Errors =====
    // Corresponds to Widevine errors in Cdm.cs, Device.cs, decryption in AaxDecrypter

    /// Generic decryption failure
    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    /// Invalid DRM format or unsupported DRM scheme
    #[error("Invalid DRM format: {0}")]
    InvalidDrmFormat(String),

    /// Widevine CDM operational error (maps to C# exceptions in Cdm.cs)
    #[error("Widevine CDM error: {message}")]
    WidevineCdmError {
        message: String,
        /// CDM operation that failed (e.g., "license_challenge", "parse_license")
        operation: Option<String>,
    },

    /// Invalid Widevine CDM file format (maps to InvalidDataException in Device.cs)
    #[error("Invalid CDM file: {reason}")]
    InvalidCdmFile {
        reason: String,
        /// Expected value
        expected: Option<String>,
        /// Actual value found
        actual: Option<String>,
    },

    /// Activation bytes not found for the given account
    #[error("Activation bytes not found for account: {0}")]
    ActivationBytesNotFound(String),

    /// License parsing or validation failed (maps to InvalidDataException in Cdm.cs)
    #[error("Invalid license: {0}")]
    InvalidLicense(String),

    /// Signature verification failed (maps to InvalidDataException in Cdm.cs)
    #[error("Message signature is invalid")]
    InvalidSignature,

    // ===== Download Errors =====
    // Corresponds to download failures in NetworkFileStream.cs, AaxcDownloadConvertBase.cs

    /// Generic download failure
    #[error("Download failed: {0}")]
    DownloadFailed(String),

    /// Network connectivity error (maps to C# WebException, HttpIOException)
    #[error("Network error: {message}")]
    NetworkError {
        message: String,
        /// Whether this error might be transient
        is_transient: bool,
    },

    /// Download was interrupted and cannot be resumed
    #[error("Download interrupted and cannot be resumed")]
    DownloadInterrupted,

    /// Download resumed but file size mismatch (maps to WebException in NetworkFileStream.cs)
    #[error("Download file size mismatch: expected {expected} bytes, got {actual} bytes")]
    FileSizeMismatch {
        expected: u64,
        actual: u64,
    },

    /// Server returned unexpected status code (maps to WebException in NetworkFileStream.cs)
    #[error("Server responded with unexpected status code: {status_code}")]
    UnexpectedStatusCode {
        status_code: u16,
        host: String,
    },

    /// Invalid download URL format or protocol
    #[error("Invalid download URL: {0}")]
    InvalidDownloadUrl(String),

    /// Content license missing offline URL (maps to InvalidDataException in DownloadOptions.cs)
    #[error("Content license doesn't contain an offline URL")]
    MissingOfflineUrl,

    /// MPEG-DASH content URL retrieval failed (maps to InvalidDataException in DownloadOptions.Factory.cs)
    #[error("Failed to get mpeg-dash content download URL")]
    MpegDashUrlFailed,

    // ===== Audio/Conversion Errors =====
    // Corresponds to audio processing in FileLiberator, ConvertToMp3.cs

    /// Audio conversion process failed
    #[error("Audio conversion failed: {0}")]
    ConversionFailed(String),

    /// Audio format is not supported by this application
    #[error("Unsupported audio format: {0}")]
    UnsupportedAudioFormat(String),

    /// Unsupported record export format (maps to NotSupportedException in DownloadDecryptBook.cs)
    #[error("Unsupported record export format: {0}")]
    UnsupportedExportFormat(String),

    /// FFmpeg execution error
    #[error("FFmpeg error: {0}")]
    FfmpegError(String),

    /// FFmpeg binary not found in PATH
    #[error("FFmpeg not found. Please install FFmpeg and ensure it's in your PATH.")]
    FfmpegNotFound,

    /// Audio file is corrupted or has invalid metadata
    #[error("Invalid audio file: {0}")]
    InvalidAudioFile(String),

    /// Failed to determine audio format from file (non-fatal in C# - DownloadDecryptBook.cs)
    #[error("Failed to determine output audio format for file: {0}")]
    AudioFormatDetectionFailed(String),

    // ===== File/Storage Errors =====
    // Corresponds to file operations in FileManager, FileLiberator, NetworkFileStream.cs

    /// File or directory not found (maps to DirectoryNotFoundException, FileNotFoundException)
    #[error("File not found: {0}")]
    FileNotFound(String),

    /// Generic file I/O error
    #[error("File I/O error: {0}")]
    FileIoError(String),

    /// Insufficient disk space for operation
    #[error("Insufficient disk space (need {need} bytes, have {have} bytes)")]
    InsufficientDiskSpace {
        need: u64,
        have: u64,
    },

    /// File operation permission denied (maps to UnauthorizedAccessException)
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// Invalid file path (maps to ArgumentException in NetworkFileStream.cs, FileUtility.cs)
    #[error("Invalid path: {0}")]
    InvalidPath(String),

    /// File already exists and overwrite not allowed
    #[error("File already exists: {0}")]
    FileAlreadyExists(String),

    /// Download directory doesn't exist (maps to ArgumentException in NetworkFileStream.cs)
    #[error("Download directory does not exist: {0}")]
    DownloadDirectoryNotFound(String),

    /// Write position exceeds content length (maps to WebException in NetworkFileStream.cs)
    #[error("Write position (0x{position:X}) exceeds content length (0x{content_length:X})")]
    WritePositionExceedsLength {
        position: u64,
        content_length: u64,
    },

    // ===== Database Errors =====
    // Corresponds to DataLayer, DtoImporterService exceptions

    /// Generic database error
    #[error("Database error: {0}")]
    DatabaseError(String),

    /// Database query execution failed
    #[error("Database query failed: {0}")]
    QueryFailed(String),

    /// Database schema migration failed
    #[error("Database migration failed: {0}")]
    MigrationFailed(String),

    /// Database record not found (maps to InvalidOperationException in Book.cs)
    #[error("Record not found: {0}")]
    RecordNotFound(String),

    /// Failed to load valid entity from database (maps to InvalidOperationException in Book.cs)
    #[error("Could not load a valid {entity_type} from database")]
    InvalidDatabaseEntity {
        entity_type: String,
    },

    // ===== Import/Validation Errors =====
    // Corresponds to ImportValidationException, importer validation in DtoImporterService

    /// Import validation failed with multiple errors (maps to ImportValidationException)
    #[error("Import validation failed with {error_count} errors")]
    ImportValidation {
        error_count: usize,
        /// Individual validation error messages
        errors: Vec<String>,
    },

    /// Importer validation failed (maps to AggregateException in ImporterBase.cs)
    #[error("Importer validation failed with {error_count} errors")]
    ImporterValidation {
        error_count: usize,
        errors: Vec<String>,
    },

    /// Invalid data format or content
    #[error("Invalid data: {0}")]
    InvalidData(String),

    /// Generic input validation error
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Required field is missing (maps to NullReferenceException, ArgumentNullException in C#)
    #[error("Missing required field: {0}")]
    MissingRequiredField(String),

    /// Configuration is invalid or incomplete
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    // ===== Configuration/State Errors =====
    // Corresponds to configuration errors in LibationFileManager, AppScaffolding

    /// Application state is invalid for the requested operation (maps to InvalidOperationException)
    #[error("Invalid state: {0}")]
    InvalidState(String),

    /// Configuration file error (maps to ApplicationException, InvalidDataException in Configuration.LibationFiles.cs)
    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    /// Settings file issue (maps to InvalidOperationException in Configuration.PersistentSettings.cs)
    #[error("Settings not initialized. {0}")]
    SettingsNotInitialized(String),

    /// Could not locate or create required file (maps to ApplicationException in Configuration.LibationFiles.cs)
    #[error("Could not locate or create {0}")]
    RequiredFileNotFound(String),

    /// Platform-specific operation not supported (maps to PlatformNotSupportedException)
    #[error("Platform not supported: {0}")]
    PlatformNotSupported(String),

    // ===== General Errors =====

    /// Operation was cancelled by user or system
    #[error("Operation cancelled")]
    Cancelled,

    /// Operation timed out
    #[error("Operation timed out after {0} seconds")]
    Timeout(u64),

    /// Feature not yet implemented
    #[error("Not implemented: {0}")]
    NotImplemented(String),

    /// Internal error that should not normally occur
    #[error("Internal error: {0}")]
    InternalError(String),

    // ===== External Library Errors =====
    // Automatic conversions from external error types

    /// HTTP client error from reqwest
    #[error("HTTP client error: {0}")]
    ReqwestError(#[from] reqwest::Error),

    /// JSON serialization/deserialization error
    #[error("JSON serialization error: {0}")]
    SerdeJsonError(#[from] serde_json::Error),

    /// Database driver error from sqlx
    #[error("Database error: {0}")]
    SqlxError(#[from] sqlx::Error),

    /// Standard I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// JNI bridge error (Android only)
    #[error("JNI error: {0}")]
    #[cfg(target_os = "android")]
    JniError(String),
}

// Implement From conversions for common error types
impl From<std::string::FromUtf8Error> for LibationError {
    fn from(err: std::string::FromUtf8Error) -> Self {
        LibationError::InternalError(format!("UTF-8 conversion error: {}", err))
    }
}

impl From<std::num::ParseIntError> for LibationError {
    fn from(err: std::num::ParseIntError) -> Self {
        LibationError::InvalidInput(format!("Failed to parse integer: {}", err))
    }
}

impl From<std::num::ParseFloatError> for LibationError {
    fn from(err: std::num::ParseFloatError) -> Self {
        LibationError::InvalidInput(format!("Failed to parse float: {}", err))
    }
}

// Helper methods for creating common errors
impl LibationError {
    /// Create a RecordNotFound error with a resource name
    pub fn not_found<S: Into<String>>(resource: S) -> Self {
        LibationError::RecordNotFound(resource.into())
    }

    /// Create an InvalidInput error with a message
    pub fn invalid_input<S: Into<String>>(message: S) -> Self {
        LibationError::InvalidInput(message.into())
    }

    /// Create an InternalError with a message
    pub fn internal<S: Into<String>>(message: S) -> Self {
        LibationError::InternalError(message.into())
    }

    /// Create a NotImplemented error with a feature name
    pub fn not_implemented<S: Into<String>>(feature: S) -> Self {
        LibationError::NotImplemented(feature.into())
    }

    /// Create an AuthenticationFailed error
    pub fn auth_failed<S: Into<String>>(message: S, account_id: Option<String>) -> Self {
        LibationError::AuthenticationFailed {
            message: message.into(),
            account_id,
        }
    }

    /// Create an ApiRequestFailed error
    pub fn api_failed<S: Into<String>>(
        message: S,
        status_code: Option<u16>,
        endpoint: Option<String>,
    ) -> Self {
        LibationError::ApiRequestFailed {
            message: message.into(),
            status_code,
            endpoint,
        }
    }

    /// Create a NetworkError
    pub fn network_error<S: Into<String>>(message: S, is_transient: bool) -> Self {
        LibationError::NetworkError {
            message: message.into(),
            is_transient,
        }
    }

    /// Create a WidevineCdmError
    pub fn cdm_error<S: Into<String>>(message: S, operation: Option<String>) -> Self {
        LibationError::WidevineCdmError {
            message: message.into(),
            operation,
        }
    }

    /// Create an InvalidCdmFile error
    pub fn invalid_cdm_file<S: Into<String>>(
        reason: S,
        expected: Option<String>,
        actual: Option<String>,
    ) -> Self {
        LibationError::InvalidCdmFile {
            reason: reason.into(),
            expected,
            actual,
        }
    }

    /// Check if error is retryable (network errors, timeouts, etc.)
    ///
    /// Returns `true` for transient errors that might succeed on retry:
    /// - Network errors marked as transient
    /// - Timeouts
    /// - Some API failures (rate limiting will have retry_after info)
    /// - Download interruptions (can resume)
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            LibationError::NetworkError { is_transient: true, .. }
                | LibationError::Timeout(_)
                | LibationError::ApiRequestFailed { status_code: Some(500..=599), .. }
                | LibationError::DownloadInterrupted
                | LibationError::RateLimitExceeded { .. }
        )
    }

    /// Check if error is due to authentication/authorization
    ///
    /// Returns `true` for errors that indicate the user needs to re-authenticate
    /// or lacks permission to perform the operation.
    pub fn is_auth_error(&self) -> bool {
        matches!(
            self,
            LibationError::AuthenticationFailed { .. }
                | LibationError::TokenExpired
                | LibationError::AccountNotFound(_)
                | LibationError::PermissionDenied(_)
        )
    }

    /// Check if error is related to file/disk operations
    pub fn is_file_error(&self) -> bool {
        matches!(
            self,
            LibationError::FileNotFound(_)
                | LibationError::FileIoError(_)
                | LibationError::InsufficientDiskSpace { .. }
                | LibationError::PermissionDenied(_)
                | LibationError::InvalidPath(_)
                | LibationError::FileAlreadyExists(_)
                | LibationError::DownloadDirectoryNotFound(_)
        )
    }

    /// Check if error is related to DRM/crypto operations
    pub fn is_crypto_error(&self) -> bool {
        matches!(
            self,
            LibationError::DecryptionFailed(_)
                | LibationError::InvalidDrmFormat(_)
                | LibationError::WidevineCdmError { .. }
                | LibationError::InvalidCdmFile { .. }
                | LibationError::ActivationBytesNotFound(_)
                | LibationError::InvalidLicense(_)
                | LibationError::InvalidSignature
                | LibationError::InvalidActivationBytes(_)
        )
    }

    /// Get retry delay in seconds for retryable errors
    ///
    /// Returns `Some(seconds)` if the error includes retry timing information,
    /// `None` otherwise. Callers should implement their own backoff strategy
    /// for errors without explicit retry delays.
    pub fn retry_after_seconds(&self) -> Option<u64> {
        match self {
            LibationError::RateLimitExceeded { retry_after_seconds, .. } => {
                Some(*retry_after_seconds)
            }
            _ => None,
        }
    }

    /// Get user-friendly error message suitable for display
    ///
    /// This returns actionable error messages that can be shown to end users,
    /// with technical details omitted where appropriate.
    pub fn user_message(&self) -> String {
        match self {
            LibationError::FfmpegNotFound => {
                "FFmpeg is required but not found. Please install FFmpeg and ensure it's in your PATH.".to_string()
            }
            LibationError::ActivationBytesNotFound(account) => {
                format!("Activation bytes not found for account '{}'. Please provide activation bytes to decrypt AAX files.", account)
            }
            LibationError::AuthenticationFailed { message, account_id } => {
                if let Some(id) = account_id {
                    format!("Authentication failed for account '{}': {}. Please check your credentials and try again.", id, message)
                } else {
                    format!("Authentication failed: {}. Please check your credentials and try again.", message)
                }
            }
            LibationError::TokenExpired => {
                "Your session has expired. Please log in again.".to_string()
            }
            LibationError::InsufficientDiskSpace { need, have } => {
                format!(
                    "Insufficient disk space. Need {} MB, but only {} MB available.",
                    need / 1_000_000,
                    have / 1_000_000
                )
            }
            LibationError::RateLimitExceeded { retry_after_seconds, .. } => {
                format!(
                    "API rate limit exceeded. Please wait {} seconds before trying again.",
                    retry_after_seconds
                )
            }
            LibationError::MissingOfflineUrl => {
                "This audiobook's license doesn't support offline playback.".to_string()
            }
            LibationError::FileSizeMismatch { expected, actual } => {
                format!(
                    "Download verification failed: file size mismatch (expected {} MB, got {} MB). Please try downloading again.",
                    expected / 1_000_000,
                    actual / 1_000_000
                )
            }
            LibationError::DownloadInterrupted => {
                "Download was interrupted. Please try again.".to_string()
            }
            LibationError::ImportValidation { error_count, errors } => {
                let error_list = errors.iter()
                    .take(3)
                    .map(|e| format!("  - {}", e))
                    .collect::<Vec<_>>()
                    .join("\n");
                let remaining = if *error_count > 3 {
                    format!("\n  ... and {} more errors", error_count - 3)
                } else {
                    String::new()
                };
                format!("Import validation failed with {} errors:\n{}{}", error_count, error_list, remaining)
            }
            LibationError::AccountValidationFailed { field, book_title } => {
                format!("Cannot process '{}': missing {} information. Please check your account settings.", book_title, field)
            }
            _ => self.to_string(),
        }
    }
}

// ===== IMPLEMENTATION NOTES =====
//
// ## Error Handling Strategy
//
// 1. **Use thiserror for ergonomic error definitions**
//    - Each variant has a clear, descriptive `#[error]` message
//    - Structured errors include context fields (status codes, URLs, etc.)
//    - Automatic conversions via `#[from]` for external errors
//
// 2. **Use anyhow for application-level error handling (with context)**
//    - Add context to errors for better debugging
//    - Example:
//      ```rust
//      use anyhow::Context;
//      let file = std::fs::read_to_string(path)
//          .context(format!("Failed to read file: {}", path.display()))?;
//      ```
//
// 3. **Return Result<T> = std::result::Result<T, LibationError> from all fallible functions**
//    - Consistent error type across the codebase
//    - Use the `Result<T>` type alias from this module
//
// 4. **Provide user-friendly messages via user_message()**
//    - Technical errors → actionable user messages
//    - Sensitive data sanitized before display
//
// ## Error Categories
//
// - **API**: Authentication, requests, responses, rate limiting
// - **Crypto/DRM**: Decryption, Widevine CDM, activation bytes, licenses
// - **Download**: Network errors, interruptions, resume failures, size mismatches
// - **Audio**: Conversion, format detection, FFmpeg operations
// - **File**: I/O, permissions, disk space, paths
// - **Database**: Queries, migrations, records, entity loading
// - **Import/Validation**: Data validation, import errors, schema validation
// - **Configuration**: App state, settings, platform support
// - **General**: Cancellation, timeouts, internal errors
//
// ## Retryable vs Non-Retryable Errors
//
// ### Retryable Errors (check with `is_retryable()`)
// - Network errors marked as transient
// - Timeouts (may succeed on retry)
// - Download interruptions (can resume)
// - 5xx server errors (temporary server issues)
// - Rate limiting (with explicit retry_after)
//
// ### Non-Retryable Errors
// - Authentication failures (need new credentials)
// - Validation errors (bad input won't improve)
// - File not found (won't exist on retry)
// - Insufficient disk space (need to free space first)
// - Invalid configuration (needs manual fix)
// - Crypto errors (wrong keys/activation bytes)
//
// ## Error Categorization Methods
//
// - `is_retryable()` - Can this error succeed on retry?
// - `is_auth_error()` - Does user need to re-authenticate?
// - `is_file_error()` - Is this a file/disk operation error?
// - `is_crypto_error()` - Is this a DRM/decryption error?
// - `retry_after_seconds()` - Get explicit retry delay if available
//
// ## Usage Examples
//
// ### Creating errors with context
//
// ```rust
// // Simple string errors
// return Err(LibationError::FileNotFound("book.aax".to_string()));
//
// // Structured errors with context
// return Err(LibationError::auth_failed(
//     "Invalid credentials",
//     Some("user@example.com".to_string())
// ));
//
// return Err(LibationError::api_failed(
//     "Request timeout",
//     Some(504),
//     Some("/library".to_string())
// ));
//
// // Automatic conversion from external errors
// let data = std::fs::read_to_string(path)?; // std::io::Error → LibationError::IoError
// let json: Value = serde_json::from_str(&data)?; // serde_json::Error → LibationError::SerdeJsonError
// ```
//
// ### Handling errors with retry logic
//
// ```rust
// use std::time::Duration;
// use tokio::time::sleep;
//
// async fn download_with_retry(url: &str) -> Result<Vec<u8>> {
//     let mut attempts = 0;
//     loop {
//         match download(url).await {
//             Ok(data) => return Ok(data),
//             Err(e) if e.is_retryable() && attempts < 3 => {
//                 let delay = e.retry_after_seconds()
//                     .unwrap_or(2u64.pow(attempts)); // Exponential backoff
//                 sleep(Duration::from_secs(delay)).await;
//                 attempts += 1;
//             }
//             Err(e) => return Err(e),
//         }
//     }
// }
// ```
//
// ### Displaying user-friendly messages
//
// ```rust
// match operation() {
//     Ok(result) => println!("Success: {:?}", result),
//     Err(e) => {
//         eprintln!("Error: {}", e.user_message());
//         if e.is_auth_error() {
//             // Redirect to login screen
//         }
//     }
// }
// ```
//
// ## Error Logging Best Practices
//
// - Log errors with appropriate levels (error!, warn!, debug!)
// - Include context and stack traces for debugging
// - **IMPORTANT**: Sanitize sensitive data before logging:
//   - Access tokens
//   - Activation bytes
//   - User credentials
//   - API keys
// - Use structured logging with error categorization:
//   ```rust
//   if let Err(e) = operation() {
//       error!(
//           error = ?e,
//           retryable = e.is_retryable(),
//           category = if e.is_auth_error() { "auth" }
//                      else if e.is_crypto_error() { "crypto" }
//                      else { "other" },
//           "Operation failed"
//       );
//   }
//   ```
//
// ## Mapping from Libation C# Exceptions
//
// This error system closely mirrors the exception handling in the original Libation C# codebase.
// When porting C# code, refer to the documentation at the top of this file for the mapping
// between C# exception types and Rust error variants.
//
// Key differences from C#:
// - Rust uses Result<T, E> instead of exceptions
// - Structured errors with context fields instead of exception properties
// - Explicit error categorization (retryable, auth, file, crypto)
// - User-friendly messages via method instead of exception messages
