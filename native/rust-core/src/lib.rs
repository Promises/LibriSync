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
pub mod providers;

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
