//! Test helpers for live API integration tests
//!
//! This module provides utilities for managing test credentials,
//! interactive OAuth flows, and API request helpers.

use rust_core::api::{
    auth::{Account, Identity, Locale},
    registration::RegistrationResponse,
};
use rust_core::error::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Test credentials stored on disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCredentials {
    pub account: Account,
    pub saved_at: String,
}

/// Get the test credentials file path
pub fn credentials_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("test_credentials.json");
    path
}

/// Save credentials to disk for reuse in tests
pub fn save_credentials(account: &Account) -> Result<()> {
    let credentials = TestCredentials {
        account: account.clone(),
        saved_at: chrono::Utc::now().to_rfc3339(),
    };

    let json = serde_json::to_string_pretty(&credentials)?;
    fs::write(credentials_path(), json)?;

    println!("✅ Credentials saved to: {:?}", credentials_path());
    Ok(())
}

/// Load credentials from disk
pub fn load_credentials() -> Result<Account> {
    let path = credentials_path();
    if !path.exists() {
        return Err(rust_core::error::LibationError::InvalidInput(
            format!("Credentials file not found: {:?}\nRun the interactive OAuth test first!", path)
        ));
    }

    let json = fs::read_to_string(path)?;
    let credentials: TestCredentials = serde_json::from_str(&json)?;

    println!("✅ Credentials loaded from disk");
    println!("   Account: {}", credentials.account.account_name);
    println!("   Saved at: {}", credentials.saved_at);

    Ok(credentials.account)
}

/// Check if credentials exist and are valid
pub fn has_valid_credentials() -> bool {
    match load_credentials() {
        Ok(account) => !account.needs_token_refresh(),
        Err(_) => false,
    }
}

/// Interactive prompt helper
pub fn prompt(message: &str) -> String {
    use std::io::{self, Write};

    print!("{}", message);
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    input.trim().to_string()
}

/// Wait for user confirmation
pub fn wait_for_confirmation(message: &str) {
    println!("\n{}", message);
    prompt("Press Enter to continue...");
}

/// Pretty print a table row
pub fn print_row(label: &str, value: &str) {
    println!("   {:25} {}", format!("{}:", label), value);
}

/// Print a section header
pub fn print_header(title: &str) {
    println!("\n{}", "=".repeat(60));
    println!("{}", title);
    println!("{}", "=".repeat(60));
}

/// Print a subsection header
pub fn print_section(title: &str) {
    println!("\n{}", title);
    println!("{}", "-".repeat(60));
}

/// Truncate a string for display
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_path() {
        let path = credentials_path();
        assert!(path.to_string_lossy().ends_with("test_credentials.json"));
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello...");
    }
}
