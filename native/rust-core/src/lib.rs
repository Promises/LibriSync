uniffi::setup_scaffolding!();

// JNI bridge for Android (DO NOT MODIFY - existing bridge)
#[cfg(target_os = "android")]
mod jni_bridge;

// C FFI bridge for iOS
#[cfg(target_os = "ios")]
pub mod ios_bridge;

// Core modules
pub mod error;
pub mod api;
pub mod crypto;
pub mod download;
pub mod audio;
pub mod storage;
pub mod file;

// Re-export commonly used types for convenience
pub use error::{LibationError, Result};

// Existing log_from_rust function (DO NOT MODIFY - used by existing bridge)
#[uniffi::export]
pub fn log_from_rust(message: String) -> String {
    let log_message = format!("Rust native module says: {message}");
    println!("{log_message}");
    log_message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_from_rust() {
        let result = log_from_rust("Hello".to_string());
        assert!(result.contains("Rust native module says: Hello"));
    }
}
