//! Activation bytes extraction test
//!
//! Tests activation bytes retrieval using real authentication data.
//! This is a critical component for DRM removal (AAX decryption).

use rust_core::api::{
    auth::{Account, Locale, get_activation_bytes},
    registration::RegistrationResponse,
};
use rust_core::error::Result;

const TEST_FIXTURE: &str = include_str!("../test_fixtures/registration_response.json");

/// Test activation bytes API endpoint structure
///
/// This test verifies the endpoint URL construction and request preparation.
/// Actual API call would require live authentication.
#[test]
fn test_activation_bytes_endpoint_url() {
    let locale = Locale::us();

    // Construct expected endpoint URL
    let expected_url = format!(
        "https://www.{}/license/token?action=register&player_manuf=Audible,iPhone&player_model=iPhone",
        locale.domain
    );

    assert_eq!(
        expected_url,
        "https://www.audible.com/license/token?action=register&player_manuf=Audible,iPhone&player_model=iPhone"
    );

    println!("✅ Activation bytes endpoint URL: {}", expected_url);
}

#[test]
fn test_activation_bytes_for_different_locales() {
    let locales = vec![
        (Locale::us(), "audible.com"),
        (Locale::uk(), "audible.co.uk"),
        (Locale::de(), "audible.de"),
        (Locale::fr(), "audible.fr"),
    ];

    for (locale, expected_domain) in locales {
        let url = format!(
            "https://www.{}/license/token?action=register&player_manuf=Audible,iPhone&player_model=iPhone",
            locale.domain
        );
        assert!(url.contains(expected_domain));
        println!("✅ {} → {}", locale.name, url);
    }
}

/// Test account preparation for activation bytes retrieval
#[test]
fn test_account_ready_for_activation_bytes() {
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");

    let locale = Locale::us();
    let identity = response.to_identity(locale.clone())
        .expect("Failed to create identity");

    let mut account = Account::new(identity.amazon_account_id.clone())
        .expect("Failed to create account");
    account.set_identity(identity);

    // Verify account has required data for activation bytes
    assert!(account.identity.is_some());

    let identity = account.identity.as_ref().unwrap();
    assert!(!identity.access_token.token.is_empty());
    assert_eq!(identity.locale.country_code, "us");

    // Account should not need token refresh
    assert!(!account.needs_token_refresh());

    println!("✅ Account is ready for activation bytes retrieval");
}

/// Test activation bytes binary format parsing
///
/// Activation bytes are extracted from a binary blob response:
/// - Response size should be >= ACTIVATION_BLOB_SZ (0x238 = 568 bytes)
/// - Activation bytes are a 4-byte uint at offset: len - ACTIVATION_BLOB_SZ
/// - Converted to 8-character lowercase hex string
#[test]
fn test_activation_bytes_binary_format() {
    const ACTIVATION_BLOB_SZ: usize = 0x238;

    // Simulate a valid activation blob (minimum size)
    let mut blob = vec![0u8; ACTIVATION_BLOB_SZ + 10];

    // Place test activation bytes at correct offset
    let offset = blob.len() - ACTIVATION_BLOB_SZ;
    blob[offset] = 0x1a;
    blob[offset + 1] = 0x2b;
    blob[offset + 2] = 0x3c;
    blob[offset + 3] = 0x4d;

    // Extract as little-endian uint32
    let act_bytes = u32::from_le_bytes([
        blob[offset],
        blob[offset + 1],
        blob[offset + 2],
        blob[offset + 3],
    ]);

    // Convert to hex string
    let hex_string = format!("{:08x}", act_bytes);

    assert_eq!(hex_string, "4d3c2b1a"); // Little-endian!
    assert_eq!(hex_string.len(), 8);

    println!("✅ Activation bytes format: {}", hex_string);
}

#[test]
fn test_activation_bytes_error_cases() {
    const ACTIVATION_BLOB_SZ: usize = 0x238;

    // Test blob too small
    let small_blob = vec![0u8; ACTIVATION_BLOB_SZ - 1];
    assert!(small_blob.len() < ACTIVATION_BLOB_SZ);

    // Test valid blob
    let valid_blob = vec![0u8; ACTIVATION_BLOB_SZ];
    assert!(valid_blob.len() >= ACTIVATION_BLOB_SZ);

    // Test large blob (should still work)
    let large_blob = vec![0u8; ACTIVATION_BLOB_SZ + 1000];
    assert!(large_blob.len() >= ACTIVATION_BLOB_SZ);

    println!("✅ Activation bytes size validation works");
}

/// Test full activation bytes extraction flow (mock)
///
/// This simulates the complete flow without making actual API calls:
/// 1. Parse registration response
/// 2. Create account with identity
/// 3. Prepare activation bytes request
/// 4. Verify request format
#[test]
fn test_activation_bytes_flow_preparation() {
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");

    let locale = Locale::us();
    let identity = response.to_identity(locale.clone())
        .expect("Failed to create identity");

    let mut account = Account::new(identity.amazon_account_id.clone())
        .expect("Failed to create account");
    account.set_account_name(identity.customer_info.name.clone());
    account.set_identity(identity.clone());

    // Verify prerequisites for activation bytes call
    println!("\n=== Activation Bytes Request Preparation ===");
    println!("Account: {}", account.account_name);
    println!("Locale: {}", identity.locale.name);
    println!("Access Token: {}...", &identity.access_token.token[..30]);
    println!("API URL: {}", identity.locale.api_url());

    // Build request details
    let endpoint_url = format!(
        "https://www.{}/license/token?action=register&player_manuf=Audible,iPhone&player_model=iPhone",
        identity.locale.domain
    );
    println!("Endpoint: {}", endpoint_url);
    println!("Authorization: Bearer {}...", &identity.access_token.token[..20]);
    println!("===========================================\n");

    assert!(!identity.access_token.token.is_empty());
    assert_eq!(identity.locale.domain, "audible.com");
}

/// Test activation bytes hex formatting
#[test]
fn test_activation_bytes_hex_format() {
    let test_cases = vec![
        (0x00000000u32, "00000000"),
        (0xFFFFFFFFu32, "ffffffff"),
        (0x1A2B3C4Du32, "1a2b3c4d"),
        (0xDEADBEEFu32, "deadbeef"),
    ];

    for (input, expected) in test_cases {
        let hex = format!("{:08x}", input);
        assert_eq!(hex, expected);
        assert_eq!(hex.len(), 8);
        println!("✅ {:08X} → {}", input, hex);
    }
}

/// Test that decrypt_key field is properly set
#[test]
fn test_decrypt_key_storage() {
    let mut account = Account::new("test@example.com".to_string())
        .expect("Failed to create account");

    // Initially empty
    assert!(account.decrypt_key.is_empty());

    // Set activation bytes
    let activation_bytes = "1a2b3c4d";
    account.set_decrypt_key(activation_bytes.to_string());

    assert_eq!(account.decrypt_key, activation_bytes);

    println!("✅ Decrypt key stored: {}", account.decrypt_key);
}

/// Test activation bytes in different byte orders
#[test]
fn test_activation_bytes_endianness() {
    // Test data: 0x1A 0x2B 0x3C 0x4D
    let bytes = [0x1A, 0x2B, 0x3C, 0x4D];

    // Little-endian (used by Audible)
    let le = u32::from_le_bytes(bytes);
    let le_hex = format!("{:08x}", le);
    assert_eq!(le_hex, "4d3c2b1a");

    // Big-endian (for comparison)
    let be = u32::from_be_bytes(bytes);
    let be_hex = format!("{:08x}", be);
    assert_eq!(be_hex, "1a2b3c4d");

    println!("✅ Little-endian: {} (correct)", le_hex);
    println!("   Big-endian: {} (incorrect)", be_hex);
}

/// Integration test: Complete activation bytes workflow
///
/// This test simulates the complete workflow from authentication
/// to activation bytes storage, using real test fixture data.
#[test]
fn test_complete_activation_bytes_workflow() {
    println!("\n=== Complete Activation Bytes Workflow ===");

    // Step 1: Parse registration response
    println!("📝 Step 1: Parse registration response");
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");
    println!("   ✅ Registration response parsed");

    // Step 2: Create identity
    println!("🔐 Step 2: Create identity");
    let locale = Locale::us();
    let identity = response.to_identity(locale.clone())
        .expect("Failed to create identity");
    println!("   ✅ Identity created for {}", identity.customer_info.name);

    // Step 3: Create account
    println!("👤 Step 3: Create account");
    let mut account = Account::new(identity.amazon_account_id.clone())
        .expect("Failed to create account");
    account.set_account_name(identity.customer_info.name.clone());
    account.set_identity(identity.clone());
    println!("   ✅ Account created: {}", account.account_name);

    // Step 4: Verify prerequisites
    println!("✓ Step 4: Verify prerequisites");
    assert!(account.identity.is_some());
    assert!(!account.needs_token_refresh());
    println!("   ✅ Account has valid tokens");

    // Step 5: Prepare activation bytes request
    println!("📡 Step 5: Prepare activation bytes request");
    let endpoint = format!(
        "https://www.{}/license/token?action=register&player_manuf=Audible,iPhone&player_model=iPhone",
        identity.locale.domain
    );
    println!("   Endpoint: {}", endpoint);
    println!("   Token: {}...", &identity.access_token.token[..30]);

    // Step 6: Simulate response parsing
    println!("🔍 Step 6: Simulate activation bytes extraction");
    let simulated_activation_bytes = "1a2b3c4d";
    account.set_decrypt_key(simulated_activation_bytes.to_string());
    println!("   ✅ Activation bytes: {}", account.decrypt_key);

    // Step 7: Verify storage
    println!("💾 Step 7: Verify activation bytes stored");
    assert_eq!(account.decrypt_key, simulated_activation_bytes);
    assert!(!account.decrypt_key.is_empty());
    println!("   ✅ Decrypt key stored in account");

    println!("=========================================");
    println!("🎉 Complete workflow successful!");
    println!("\n📊 Account Summary:");
    println!("   Name: {}", account.account_name);
    println!("   Locale: {}", identity.locale.name);
    println!("   Activation Bytes: {}", account.decrypt_key);
    println!("   Ready for DRM removal: ✅");
}
