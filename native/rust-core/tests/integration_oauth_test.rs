//! Integration test for complete OAuth authentication flow
//!
//! This test uses actual registration response data from the test fixture
//! to verify:
//! 1. Registration response parsing
//! 2. Identity creation
//! 3. Account creation with full data
//! 4. Token expiration checking
//! 5. Preparation for library sync

use rust_core::api::{
    auth::{Account, Locale},
    registration::RegistrationResponse,
};
use rust_core::error::Result;

const TEST_FIXTURE: &str = include_str!("../test_fixtures/registration_response.json");

#[test]
fn test_parse_real_registration_response() {
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");

    // Verify response structure
    assert_eq!(response.request_id, "8e570a18-8232-4df0-a212-c0d21860fcd5");
    assert_eq!(
        response.response.success.customer_id,
        "amzn1.account.AGMGLSGIFYVALF2MEO4F3JJQRLSA"
    );

    println!("✅ Registration response parsed successfully");
}

#[test]
fn test_extract_all_tokens() {
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");

    let locale = Locale::us();
    let data = response.extract_data(locale)
        .expect("Failed to extract registration data");

    // Bearer tokens
    assert!(data.access_token.token.starts_with("Atna|"));
    assert!(data.refresh_token.starts_with("Atnr|"));
    assert!(data.access_token.expires_at > chrono::Utc::now());

    // MAC DMS tokens
    assert!(data.device_private_key.len() > 1000);
    assert!(data.adp_token.contains("{enc:"));
    assert!(data.adp_token.contains("{key:"));

    // Device info
    assert_eq!(data.device_serial_number, "B45EF975C33A7B7E8DAF4D96E39B8040");
    assert_eq!(data.device_type, "A10KISP2GWF0E4");
    assert_eq!(data.device_name, "Henning's 7th Android");

    // Customer info
    assert_eq!(data.customer_info.name, "Henning Berge");
    assert_eq!(data.customer_info.given_name, "Henning");
    assert_eq!(data.customer_info.home_region, "NA");

    // Cookies
    assert!(data.cookies.contains_key("session-id"));
    assert!(data.cookies.contains_key("at-main"));
    assert!(data.cookies.contains_key("x-main"));
    assert!(data.cookies.len() >= 5);

    // Store auth cookie
    assert!(data.store_authentication_cookie.len() > 50);

    println!("✅ All tokens extracted successfully");
}

#[test]
fn test_create_identity_from_real_data() {
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");

    let locale = Locale::us();
    let identity = response.to_identity(locale.clone())
        .expect("Failed to create identity");

    // Verify Identity has all required fields
    assert!(identity.access_token.token.len() > 50);
    assert!(identity.refresh_token.len() > 50);
    assert_eq!(identity.device_serial_number, "B45EF975C33A7B7E8DAF4D96E39B8040");
    assert_eq!(identity.device_type, "A10KISP2GWF0E4");
    assert_eq!(identity.amazon_account_id, "amzn1.account.AGMGLSGIFYVALF2MEO4F3JJQRLSA");
    assert_eq!(identity.locale, locale);

    // Verify not expired
    assert!(!identity.is_expired());

    println!("✅ Identity created with complete data");
}

#[test]
fn test_create_account_with_real_identity() {
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");

    let locale = Locale::us();
    let identity = response.to_identity(locale)
        .expect("Failed to create identity");

    // Create account (using customer email or ID)
    let account_id = identity.amazon_account_id.clone();
    let mut account = Account::new(account_id.clone())
        .expect("Failed to create account");

    // Set friendly name from customer info
    account.set_account_name(identity.customer_info.name.clone());

    // Set identity
    account.set_identity(identity.clone());

    // Verify account
    assert_eq!(account.account_id, account_id);
    assert_eq!(account.account_name, "Henning Berge");
    assert!(account.identity.is_some());
    assert!(!account.needs_token_refresh());

    // Verify locale
    let account_locale = account.locale().expect("No locale");
    assert_eq!(account_locale.country_code, "us");

    println!("✅ Account created with full identity");
}

#[test]
fn test_token_expiration_logic() {
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");

    let locale = Locale::us();
    let identity = response.to_identity(locale)
        .expect("Failed to create identity");

    let mut account = Account::new(identity.amazon_account_id.clone())
        .expect("Failed to create account");
    account.set_identity(identity);

    // Should not need refresh (token is fresh)
    assert!(!account.needs_token_refresh());

    // Test with expired token
    let mut expired_account = account.clone();
    if let Some(identity) = &mut expired_account.identity {
        identity.access_token.expires_at = chrono::Utc::now() - chrono::Duration::hours(1);
    }
    assert!(expired_account.needs_token_refresh());

    println!("✅ Token expiration logic working correctly");
}

#[test]
fn test_all_cookies_extracted() {
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");

    let cookies = &response.response.success.tokens.website_cookies;

    // Verify all 5 cookies are present
    let expected_cookies = vec![
        "session-id",
        "ubid-main",
        "x-main",
        "at-main",
        "sess-at-main",
    ];

    for expected in &expected_cookies {
        assert!(
            cookies.iter().any(|c| c.name == *expected),
            "Missing cookie: {}",
            expected
        );
    }

    // Verify critical authentication cookies
    let at_main = cookies.iter().find(|c| c.name == "at-main").unwrap();
    assert!(at_main.value.starts_with("\"Atza|"));
    assert_eq!(at_main.http_only, "true");
    assert_eq!(at_main.secure, "true");

    println!("✅ All cookies extracted and validated");
}

#[test]
fn test_device_credentials_complete() {
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");

    let locale = Locale::us();
    let data = response.extract_data(locale)
        .expect("Failed to extract data");

    // Verify device private key (RSA key in PEM-like format, but base64 only)
    assert!(data.device_private_key.starts_with("MII"));
    assert!(data.device_private_key.len() > 1500); // RSA-2048 keys are ~1700 chars

    // Verify ADP token has encrypted structure
    assert!(data.adp_token.contains("{enc:"));
    assert!(data.adp_token.contains("{key:"));
    assert!(data.adp_token.contains("{iv:"));
    assert!(data.adp_token.contains("{name:"));
    assert!(data.adp_token.contains("{serial:"));

    // Verify store auth cookie
    assert!(data.store_authentication_cookie.len() > 50);

    println!("✅ Device credentials complete and properly formatted");
}

#[test]
fn test_customer_info_complete() {
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");

    let customer_info = &response.response.success.extensions.customer_info;

    // Verify all fields
    assert_eq!(customer_info.account_pool, "Amazon");
    assert_eq!(customer_info.user_id, "amzn1.account.AGMGLSGIFYVALF2MEO4F3JJQRLSA");
    assert_eq!(customer_info.home_region, "NA");
    assert_eq!(customer_info.name, "Henning Berge");
    assert_eq!(customer_info.given_name, "Henning");

    // Verify customer_id matches user_id
    assert_eq!(
        response.response.success.customer_id,
        customer_info.user_id
    );

    println!("✅ Customer info complete and consistent");
}

#[test]
fn test_ready_for_library_sync() {
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");

    let locale = Locale::us();
    let identity = response.to_identity(locale.clone())
        .expect("Failed to create identity");

    // Verify we have everything needed for API calls
    assert!(!identity.access_token.token.is_empty());
    assert!(!identity.refresh_token.is_empty());
    assert!(!identity.device_serial_number.is_empty());
    assert!(!identity.device_type.is_empty());
    assert!(!identity.amazon_account_id.is_empty());

    // Verify locale is correct
    assert_eq!(identity.locale.country_code, "us");
    assert_eq!(identity.locale.domain, "audible.com");

    // Verify API URL can be constructed
    let api_url = identity.locale.api_url();
    assert_eq!(api_url, "https://api.audible.com");

    println!("✅ Identity is ready for library sync API calls");
}

#[test]
fn test_masked_logging_safety() {
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");

    let locale = Locale::us();
    let identity = response.to_identity(locale)
        .expect("Failed to create identity");

    let mut account = Account::new(identity.amazon_account_id.clone())
        .expect("Failed to create account");
    account.set_account_name(identity.customer_info.name.clone());
    account.set_identity(identity);

    // Get masked log entry
    let masked = account.masked_log_entry();

    // Should NOT contain full tokens or sensitive data
    assert!(!masked.contains("Atna|"));
    assert!(!masked.contains("Atnr|"));
    assert!(!masked.contains(&account.account_id));

    // Should contain masked versions
    assert!(masked.contains("AccountId="));
    assert!(masked.contains("AccountName="));
    assert!(masked.contains("Locale="));

    println!("✅ Masked logging is safe");
    println!("   Masked output: {}", masked);
}

/// Test helper: Print complete registration summary
#[test]
fn test_print_registration_summary() {
    let response = RegistrationResponse::from_json(TEST_FIXTURE)
        .expect("Failed to parse registration response");

    let locale = Locale::us();
    let data = response.extract_data(locale)
        .expect("Failed to extract data");

    println!("\n=== Registration Data Summary ===");
    println!("📱 Device Info:");
    println!("   Serial: {}", data.device_serial_number);
    println!("   Type: {}", data.device_type);
    println!("   Name: {}", data.device_name);

    println!("\n👤 Customer Info:");
    println!("   Name: {}", data.customer_info.name);
    println!("   Given Name: {}", data.customer_info.given_name);
    println!("   User ID: {}", data.amazon_account_id);
    println!("   Home Region: {}", data.customer_info.home_region);
    println!("   Account Pool: {}", data.customer_info.account_pool);

    println!("\n🔑 Tokens:");
    println!("   Access Token: {}...", &data.access_token.token[..30]);
    println!("   Refresh Token: {}...", &data.refresh_token[..30]);
    println!("   Expires At: {}", data.access_token.expires_at);
    println!("   Device Private Key: {} chars", data.device_private_key.len());
    println!("   ADP Token: {} chars", data.adp_token.len());

    println!("\n🍪 Cookies:");
    for (name, value) in &data.cookies {
        println!("   {}: {}...", name, &value[..value.len().min(30)]);
    }

    println!("\n🏪 Store Auth Cookie: {} chars", data.store_authentication_cookie.len());
    println!("================================\n");
}
