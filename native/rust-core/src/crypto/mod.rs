//! Cryptography and DRM removal
//!
//! This module handles decryption of Audible's DRM-protected audio formats.
//! It ports functionality from Libation's AaxDecrypter project.
//!
//! # Reference C# Sources
//! - `AaxDecrypter/` - Main decryption logic for AAX and AAXC formats
//! - `AudibleUtilities/Widevine/` - Widevine CDM implementation for AAXC
//!
//! # DRM Formats
//! - **AAX** (legacy): AES encryption with activation bytes
//! - **AAXC** (current): Widevine DRM with chunked MPEG-DASH delivery
//! - **Unencrypted**: Direct MP3/M4B for podcasts

pub mod activation;
pub mod aax;
pub mod aaxc;
pub mod widevine;

// Re-export commonly used types from activation module
pub use activation::{
    ActivationBytes,
    format_activation_bytes,
    parse_activation_bytes,
    validate_activation_bytes,
};

// Re-export commonly used types from AAX module
pub use aax::{
    AaxDecrypter,
    is_aax_file,
    verify_activation_bytes,
};

// Re-export AAXC decrypter (placeholder for now)
pub use aaxc::AaxcDecrypter;
