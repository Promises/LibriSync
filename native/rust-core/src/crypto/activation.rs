//! Activation bytes retrieval and management
//!
//! # Reference C# Sources
//! - `AudibleUtilities/Account.cs` - DecryptKey property (activation bytes storage)
//! - External tools referenced by Libation:
//!   - audible-cli (Python) - Can extract activation bytes
//!   - RainbowCrack tables - Pre-computed activation bytes
//!   - Direct API method (if available)
//!
//! # What are Activation Bytes?
//! - 4-byte key derived from Audible account credentials
//! - Used to decrypt AAX files (AES encryption)
//! - Unique per account
//! - Format: 8 hex characters (e.g., "1CEB00DA")
//! - Stored in Account.DecryptKey in Libation
//!
//! # Extraction Methods (from Libation documentation)
//! 1. **From Audible API**:
//!    - Some accounts can retrieve directly via API call
//!    - Not documented by Audible, may be unofficial
//! 2. **From audible-cli**:
//!    - Python tool that can extract from authentication
//!    - Command: audible activation-bytes
//! 3. **Manual extraction**:
//!    - Reverse engineer from Audible app memory
//!    - Use tools like Cheat Engine on Windows
//! 4. **Pre-computed tables**:
//!    - Rainbow tables for common credentials
//!    - Not recommended (security/legal concerns)
//!
//! # Storage
//! - Store in Account.decrypt_key field
//! - Validate format (8 hex chars)
//! - Never log or expose in plaintext

use crate::error::{LibationError, Result};

/// Newtype wrapper around activation bytes to provide type safety
///
/// Activation bytes are a 4-byte key used to decrypt AAX files.
/// This wrapper ensures the bytes are always valid and provides
/// convenient conversion methods.
///
/// # C# Reference
/// Corresponds to Account.DecryptKey property in Account.cs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivationBytes([u8; 4]);

impl ActivationBytes {
    /// Create ActivationBytes from a 4-byte array
    pub fn new(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }

    /// Parse activation bytes from hex string
    ///
    /// # C# Reference
    /// Similar to Convert.FromHexString in KeyData.cs
    ///
    /// # Arguments
    /// * `hex` - Hex string (8 characters, e.g., "1CEB00DA")
    ///
    /// # Errors
    /// - InvalidActivationBytes if the string is not 8 hex characters
    pub fn from_hex(hex: &str) -> Result<Self> {
        parse_activation_bytes(hex).map(Self)
    }

    /// Format activation bytes as hex string
    ///
    /// # C# Reference
    /// Similar to Convert.ToHexString in AaxcDownloadConvertBase.cs
    ///
    /// # Returns
    /// Uppercase hex string (8 characters, e.g., "1CEB00DA")
    pub fn to_hex(&self) -> String {
        format_activation_bytes(&self.0)
    }

    /// Get the raw bytes
    pub fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }

    /// Consume and return the raw bytes
    pub fn into_bytes(self) -> [u8; 4] {
        self.0
    }
}

/// Validate and parse activation bytes from hex string
///
/// # C# Reference
/// Corresponds to validation logic in Account.cs and KeyData.cs
///
/// # Arguments
/// * `hex` - Hex string (8 characters, case-insensitive)
///
/// # Returns
/// - Ok([u8; 4]) if valid
/// - Err(InvalidActivationBytes) if invalid
///
/// # Example
/// ```
/// use rust_core::crypto::activation::validate_activation_bytes;
///
/// let bytes = validate_activation_bytes("1CEB00DA").unwrap();
/// assert_eq!(bytes, [0x1C, 0xEB, 0x00, 0xDA]);
/// ```
pub fn validate_activation_bytes(hex: &str) -> Result<[u8; 4]> {
    parse_activation_bytes(hex)
}

/// Parse hex string to 4-byte array
///
/// # C# Reference
/// Corresponds to Convert.FromHexString in KeyData.cs constructor
///
/// # Arguments
/// * `hex` - Hex string (8 characters, case-insensitive)
///
/// # Returns
/// - Ok([u8; 4]) if valid hex string
/// - Err(InvalidActivationBytes) if invalid format
///
/// # Format Rules
/// - Must be exactly 8 characters (4 bytes)
/// - Only valid hex digits (0-9, A-F, a-f)
/// - Whitespace is trimmed
/// - Case-insensitive
///
/// # Example
/// ```
/// use rust_core::crypto::activation::parse_activation_bytes;
///
/// let bytes = parse_activation_bytes("1ceb00da").unwrap();
/// assert_eq!(bytes, [0x1C, 0xEB, 0x00, 0xDA]);
///
/// let bytes = parse_activation_bytes("1CEB00DA").unwrap();
/// assert_eq!(bytes, [0x1C, 0xEB, 0x00, 0xDA]);
/// ```
pub fn parse_activation_bytes(hex: &str) -> Result<[u8; 4]> {
    // Trim whitespace
    let hex = hex.trim();

    // Validate length (must be exactly 8 hex characters for 4 bytes)
    if hex.len() != 8 {
        return Err(LibationError::InvalidActivationBytes(format!(
            "Expected 8 hex characters, got {}. Example format: 1CEB00DA",
            hex.len()
        )));
    }

    // Validate that all characters are valid hex digits
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(LibationError::InvalidActivationBytes(
            format!("Invalid hex characters in '{}'. Must contain only 0-9, A-F (case-insensitive)", hex)
        ));
    }

    // Parse hex string to bytes
    let mut bytes = [0u8; 4];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        // Convert two hex chars to one byte
        let hex_pair = std::str::from_utf8(chunk)
            .map_err(|_| LibationError::InvalidActivationBytes("Invalid UTF-8 in hex string".to_string()))?;

        bytes[i] = u8::from_str_radix(hex_pair, 16)
            .map_err(|e| LibationError::InvalidActivationBytes(format!("Failed to parse hex pair '{}': {}", hex_pair, e)))?;
    }

    Ok(bytes)
}

/// Format 4-byte array as hex string
///
/// # C# Reference
/// Corresponds to Convert.ToHexString in AaxcDownloadConvertBase.cs (line 62)
///
/// # Arguments
/// * `bytes` - 4-byte array to format
///
/// # Returns
/// Uppercase hex string (8 characters, e.g., "1CEB00DA")
///
/// # Example
/// ```
/// use rust_core::crypto::activation::format_activation_bytes;
///
/// let hex = format_activation_bytes(&[0x1C, 0xEB, 0x00, 0xDA]);
/// assert_eq!(hex, "1CEB00DA");
/// ```
pub fn format_activation_bytes(bytes: &[u8; 4]) -> String {
    // Format as uppercase hex, no separators
    format!(
        "{:02X}{:02X}{:02X}{:02X}",
        bytes[0], bytes[1], bytes[2], bytes[3]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_uppercase() {
        let bytes = parse_activation_bytes("1CEB00DA").unwrap();
        assert_eq!(bytes, [0x1C, 0xEB, 0x00, 0xDA]);
    }

    #[test]
    fn test_parse_valid_lowercase() {
        let bytes = parse_activation_bytes("1ceb00da").unwrap();
        assert_eq!(bytes, [0x1C, 0xEB, 0x00, 0xDA]);
    }

    #[test]
    fn test_parse_valid_mixed_case() {
        let bytes = parse_activation_bytes("1CeB00dA").unwrap();
        assert_eq!(bytes, [0x1C, 0xEB, 0x00, 0xDA]);
    }

    #[test]
    fn test_parse_with_whitespace() {
        let bytes = parse_activation_bytes("  1CEB00DA  ").unwrap();
        assert_eq!(bytes, [0x1C, 0xEB, 0x00, 0xDA]);
    }

    #[test]
    fn test_parse_invalid_length_too_short() {
        let result = parse_activation_bytes("1CEB00");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Expected 8 hex characters"));
    }

    #[test]
    fn test_parse_invalid_length_too_long() {
        let result = parse_activation_bytes("1CEB00DAFF");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Expected 8 hex characters"));
    }

    #[test]
    fn test_parse_invalid_characters() {
        let result = parse_activation_bytes("1CEB00DG");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid hex characters"));
    }

    #[test]
    fn test_parse_special_characters() {
        let result = parse_activation_bytes("1CEB-0DA");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid hex characters"));
    }

    #[test]
    fn test_format_activation_bytes() {
        let bytes = [0x1C, 0xEB, 0x00, 0xDA];
        let hex = format_activation_bytes(&bytes);
        assert_eq!(hex, "1CEB00DA");
    }

    #[test]
    fn test_format_with_leading_zeros() {
        let bytes = [0x00, 0x01, 0x0A, 0xFF];
        let hex = format_activation_bytes(&bytes);
        assert_eq!(hex, "00010AFF");
    }

    #[test]
    fn test_round_trip() {
        let original = "1CEB00DA";
        let bytes = parse_activation_bytes(original).unwrap();
        let formatted = format_activation_bytes(&bytes);
        assert_eq!(formatted, original);
    }

    #[test]
    fn test_activation_bytes_new() {
        let bytes = ActivationBytes::new([0x1C, 0xEB, 0x00, 0xDA]);
        assert_eq!(bytes.as_bytes(), &[0x1C, 0xEB, 0x00, 0xDA]);
    }

    #[test]
    fn test_activation_bytes_from_hex() {
        let bytes = ActivationBytes::from_hex("1CEB00DA").unwrap();
        assert_eq!(bytes.as_bytes(), &[0x1C, 0xEB, 0x00, 0xDA]);
    }

    #[test]
    fn test_activation_bytes_to_hex() {
        let bytes = ActivationBytes::new([0x1C, 0xEB, 0x00, 0xDA]);
        assert_eq!(bytes.to_hex(), "1CEB00DA");
    }

    #[test]
    fn test_activation_bytes_round_trip() {
        let original = "1CEB00DA";
        let bytes = ActivationBytes::from_hex(original).unwrap();
        let hex = bytes.to_hex();
        assert_eq!(hex, original);
    }

    #[test]
    fn test_activation_bytes_equality() {
        let bytes1 = ActivationBytes::from_hex("1CEB00DA").unwrap();
        let bytes2 = ActivationBytes::from_hex("1ceb00da").unwrap();
        assert_eq!(bytes1, bytes2);
    }
}
