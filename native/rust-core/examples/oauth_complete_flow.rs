//! Complete OAuth Flow Example
//!
//! This example demonstrates the complete OAuth authentication flow
//! using real registration response data.
//!
//! Usage:
//! ```bash
//! cargo run --example oauth_complete_flow
//! ```

use rust_core::api::{
    auth::{Account, Locale},
    registration::RegistrationResponse,
};

const EXAMPLE_REGISTRATION_JSON: &str = include_str!("../test_fixtures/registration_response.json");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Complete OAuth Flow Example ===\n");

    // Step 1: Parse registration response
    println!("📝 Step 1: Parse registration response from JSON");
    let response = RegistrationResponse::from_json(EXAMPLE_REGISTRATION_JSON)?;
    println!("   ✅ Registration response parsed");
    println!("   Request ID: {}", response.request_id);

    // Step 2: Extract registration data
    println!("\n🔐 Step 2: Extract complete registration data");
    let locale = Locale::us();
    let identity = response.to_identity(locale.clone())?;
    println!("   ✅ Identity created");
    println!("   Device: {}", identity.device_name);
    println!("   Serial: {}", identity.device_serial_number);
    println!("   Type: {}", identity.device_type);

    // Step 3: Create account
    println!("\n👤 Step 3: Create account");
    let mut account = Account::new(identity.amazon_account_id.clone())?;
    account.set_account_name(identity.customer_info.name.clone());
    account.set_identity(identity.clone());
    println!("   ✅ Account created");
    println!("   Name: {}", account.account_name);
    println!("   ID: {}...", &account.account_id[..30]);

    // Step 4: Verify authentication state
    println!("\n✓ Step 4: Verify authentication state");
    assert!(account.identity.is_some(), "Account should have identity");
    assert!(!account.needs_token_refresh(), "Tokens should be fresh");
    println!("   ✅ Authentication state valid");
    println!("   Token expires: {}", identity.access_token.expires_at);
    println!("   Time until expiry: {:?}", identity.time_until_expiry());

    // Step 5: Show available data
    println!("\n📊 Step 5: Available authentication data");
    println!("   Bearer Tokens:");
    println!("     - Access Token: {}...", &identity.access_token.token[..30]);
    println!("     - Refresh Token: {}...", &identity.refresh_token[..30]);
    println!("   ");
    println!("   Device Credentials:");
    println!("     - Private Key: {} chars", identity.device_private_key.len());
    println!("     - ADP Token: {} chars", identity.adp_token.len());
    println!("   ");
    println!("   Session Cookies: {}", identity.cookies.len());
    for (name, value) in &identity.cookies {
        println!("     - {}: {}...", name, &value[..value.len().min(30)]);
    }
    println!("   ");
    println!("   Customer Info:");
    println!("     - Name: {}", identity.customer_info.name);
    println!("     - Region: {}", identity.customer_info.home_region);
    println!("     - Account Pool: {}", identity.customer_info.account_pool);

    // Step 6: Ready for API calls
    println!("\n🚀 Step 6: Ready for API calls");
    let api_url = identity.locale.api_url();
    println!("   API Base URL: {}", api_url);
    println!("   Library endpoint: {}/1.0/library", api_url);
    println!("   Content endpoint: {}/1.0/content", api_url);
    println!("   License endpoint: https://www.{}/license/token", identity.locale.domain);

    // Step 7: Show next steps
    println!("\n📋 Next Steps:");
    println!("   1. Use access_token for authenticated API calls");
    println!("   2. Call library sync: GET {}/1.0/library", api_url);
    println!("   3. Get activation bytes from license endpoint");
    println!("   4. Download and decrypt audiobooks");

    println!("\n=================================");
    println!("🎉 OAuth flow complete!");
    println!("   Account: {}", account.account_name);
    println!("   Ready: ✅");
    println!("=================================\n");

    Ok(())
}
