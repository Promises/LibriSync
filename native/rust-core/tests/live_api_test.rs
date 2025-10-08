//! Live API Integration Tests
//!
//! These tests connect to the actual Audible API to verify:
//! 1. Complete OAuth registration flow
//! 2. Library synchronization
//! 3. Token refresh
//! 4. Activation bytes extraction
//!
//! # Running These Tests
//!
//! These tests are ignored by default. Run them individually with:
//!
//! ```bash
//! # Step 1: Run OAuth registration (first time only)
//! cargo test --test live_api_test test_01_interactive_oauth_registration -- --ignored --nocapture
//!
//! # Step 2: Test library sync
//! cargo test --test live_api_test test_02_library_sync -- --ignored --nocapture
//!
//! # Step 3: Test token refresh
//! cargo test --test live_api_test test_03_token_refresh -- --ignored --nocapture
//!
//! # Step 4: Test activation bytes
//! cargo test --test live_api_test test_04_activation_bytes -- --ignored --nocapture
//!
//! # Run all live tests
//! cargo test --test live_api_test -- --ignored --nocapture --test-threads=1
//! ```
//!
//! # Prerequisites
//!
//! - Valid Audible account
//! - Internet connection
//! - Browser for OAuth flow (step 1)

mod helpers;

use helpers::*;
use rust_core::api::{
    auth::*,
    registration::RegistrationResponse,
    library::{LibraryOptions, LibraryResponse},
};
use rust_core::error::Result;

// ============================================================================
// Test 1: Interactive OAuth Registration
// ============================================================================

/// Interactive OAuth registration test
///
/// This test guides you through the complete OAuth flow:
/// 1. Generates authorization URL
/// 2. Opens in browser (you log in)
/// 3. You paste callback URL
/// 4. Exchanges code for tokens
/// 5. Saves credentials for other tests
///
/// Run with:
/// ```bash
/// cargo test --test live_api_test test_01_interactive_oauth_registration -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore] // Only run manually
async fn test_01_interactive_oauth_registration() -> Result<()> {
    print_header("LIVE TEST 1: Interactive OAuth Registration");

    println!("\nThis test will guide you through the Audible OAuth flow.");
    println!("You'll need to log in to your Audible account in a browser.");
    wait_for_confirmation("Ready to start?");

    // Step 1: Generate authorization URL
    print_section("Step 1: Generate Authorization URL");

    let locale = Locale::us();
    println!("Select your Audible region:");
    println!("  1. United States (audible.com)");
    println!("  2. United Kingdom (audible.co.uk)");
    println!("  3. Germany (audible.de)");
    println!("  4. France (audible.fr)");
    println!("  5. Canada (audible.ca)");

    let choice = prompt("Enter choice (1-5): ");
    let locale = match choice.as_str() {
        "1" => Locale::us(),
        "2" => Locale::uk(),
        "3" => Locale::de(),
        "4" => Locale::fr(),
        "5" => Locale::ca(),
        _ => {
            println!("Invalid choice, using US");
            Locale::us()
        }
    };

    println!("\n✅ Selected: {} ({})", locale.name, locale.domain);

    // Generate device serial (32-char hex like Libation)
    let random_bytes: [u8; 16] = rand::random();
    let device_serial = random_bytes
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<String>();

    let pkce = PkceChallenge::generate()?;
    let state = OAuthState::generate();

    let auth_url = generate_authorization_url(&locale, &device_serial, &pkce, &state)?;

    print_row("Device Serial", &device_serial);
    print_row("PKCE Challenge", &truncate(&pkce.challenge, 40));
    print_row("State", &truncate(&state.value, 40));

    // Step 2: User opens URL and logs in
    print_section("Step 2: Complete OAuth in Browser");

    println!("\n🔗 Authorization URL:");
    println!("{}", auth_url);
    println!("\n📋 Instructions:");
    println!("1. Copy the URL above");
    println!("2. Open it in your browser");
    println!("3. Log in to your Audible account");
    println!("4. Complete any 2FA/CVF if prompted");
    println!("5. After redirect, copy the ENTIRE callback URL from browser");
    println!("   (It should start with 'https://www.amazon.com/ap/maplanding')");

    #[cfg(target_os = "macos")]
    {
        println!("\n💡 Tip: Opening URL in default browser...");
        std::process::Command::new("open")
            .arg(&auth_url)
            .spawn()
            .ok();
    }

    // Step 3: Get callback URL
    print_section("Step 3: Parse Callback URL");

    let callback_url = prompt("\n🔙 Paste the callback URL here: ");

    println!("\n⚙️  Parsing callback...");
    let authorization_code = parse_authorization_callback(&callback_url)?;

    println!("✅ Authorization code received: {}...", truncate(&authorization_code, 30));

    // Step 4: Exchange code for tokens
    print_section("Step 4: Exchange Code for Tokens");

    println!("\n🔄 Calling /auth/register endpoint...");
    println!("   This will:");
    println!("   - Exchange authorization code for tokens");
    println!("   - Register device with Audible");
    println!("   - Retrieve customer information");

    let token_response = exchange_authorization_code(
        &locale,
        &authorization_code,
        &device_serial,
        &pkce,
    ).await?;

    println!("\n✅ Token Exchange Successful!");
    print_row("Access Token", &truncate(&token_response.bearer.access_token, 40));
    print_row("Refresh Token", &truncate(&token_response.bearer.refresh_token, 40));
    print_row("Expires In", &format!("{} seconds", token_response.bearer.expires_in));

    // Step 5: Parse full registration response
    print_section("Step 5: Parse Full Registration Response");

    println!("\n🔍 Looking for saved registration response...");

    // Try to load from saved file (from exchange_authorization_code)
    let registration_json = std::fs::read_to_string("/tmp/audible_registration.json")
        .or_else(|_| std::fs::read_to_string("/data/data/com.rnaudible.app/cache/registration.json"))
        .or_else(|_| std::fs::read_to_string("/sdcard/Download/audible_registration.json"));

    let identity = if let Ok(json) = registration_json {
        println!("✅ Found saved registration response");
        let response = RegistrationResponse::from_json(&json)?;
        response.to_identity(locale.clone())?
    } else {
        println!("⚠️  Saved registration not found, creating minimal identity");
        println!("   (You may need to get activation bytes manually)");

        // Create identity from token response
        let expires_in: i64 = token_response.bearer.expires_in.parse().unwrap_or(3600);
        Identity::new(
            AccessToken {
                token: token_response.bearer.access_token.clone(),
                expires_at: chrono::Utc::now() + chrono::Duration::seconds(expires_in),
            },
            token_response.bearer.refresh_token.clone(),
            token_response.mac_dms.device_private_key.clone(),
            token_response.mac_dms.adp_token.clone(),
            locale.clone(),
        )
    };

    // Step 6: Create account
    print_section("Step 6: Create Account");

    let account_id = if !identity.customer_info.user_id.is_empty() {
        identity.customer_info.user_id.clone()
    } else {
        prompt("\nEnter your Audible email: ")
    };

    let mut account = Account::new(account_id)?;

    if !identity.customer_info.name.is_empty() {
        account.set_account_name(identity.customer_info.name.clone());
    } else {
        let name = prompt("Enter your name: ");
        account.set_account_name(name);
    }

    account.set_identity(identity.clone());

    println!("\n✅ Account Created!");
    print_row("Name", &account.account_name);
    print_row("ID", &truncate(&account.account_id, 40));
    print_row("Locale", &account.locale().unwrap().name);

    // Step 7: Save credentials
    print_section("Step 7: Save Credentials");

    save_credentials(&account)?;

    println!("\n✅ Credentials saved for reuse in other tests");

    // Step 8: Summary
    print_header("REGISTRATION COMPLETE");

    println!("\n📊 Summary:");
    println!("   ✅ OAuth authorization successful");
    println!("   ✅ Device registered with Audible");
    println!("   ✅ Tokens obtained and saved");
    println!("   ✅ Account created");
    println!("   ✅ Ready for API calls");

    println!("\n📋 Next Steps:");
    println!("   1. Run library sync test:");
    println!("      cargo test test_02_library_sync -- --ignored --nocapture");
    println!("   2. Run activation bytes test:");
    println!("      cargo test test_04_activation_bytes -- --ignored --nocapture");

    Ok(())
}

// ============================================================================
// Test 2: Library Synchronization
// ============================================================================

/// Test library synchronization with real API
///
/// This test:
/// 1. Loads saved credentials
/// 2. Connects to Audible API
/// 3. Fetches your library
/// 4. Displays book information
///
/// Run with:
/// ```bash
/// cargo test --test live_api_test test_02_library_sync -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore] // Only run manually
async fn test_02_library_sync() -> Result<()> {
    print_header("LIVE TEST 2: Library Synchronization");

    // Step 1: Load credentials
    print_section("Step 1: Load Credentials");

    let mut account = load_credentials()?;

    print_row("Account", &account.account_name);
    print_row("Locale", &account.locale().unwrap().name);

    // Check token expiration
    if account.needs_token_refresh() {
        println!("\n⚠️  Access token expired, refreshing...");
        account.refresh_tokens().await?;
        save_credentials(&account)?;
        println!("✅ Tokens refreshed");
    } else {
        println!("✅ Access token is valid");
    }

    // Step 2: Prepare API request
    print_section("Step 2: Prepare API Request");

    let identity = account.identity.as_ref().unwrap();
    let api_url = identity.locale.api_url();

    print_row("API URL", &api_url);
    print_row("Access Token", &truncate(&identity.access_token.token, 40));

    // Step 3: Fetch library
    print_section("Step 3: Fetch Library from Audible");

    println!("\n🔄 Calling GET /1.0/library...");
    println!("   This may take a moment for large libraries...");

    // Make direct HTTP request to library endpoint
    let options = LibraryOptions {
        number_of_results_per_page: 50,
        page_number: 1,
        ..Default::default()
    };

    let client = reqwest::Client::new();
    let library_response: LibraryResponse = client
        .get(format!("{}/1.0/library", api_url))
        .header("Authorization", format!("Bearer {}", identity.access_token.token))
        .query(&options)
        .send()
        .await?
        .json()
        .await?;

    println!("\n✅ Library Retrieved!");
    print_row("Total Items", &library_response.total_results.unwrap_or(0).to_string());
    print_row("Items Retrieved", &library_response.items.len().to_string());

    // Step 4: Display books
    print_section("Step 4: Your Audiobooks");

    if library_response.items.is_empty() {
        println!("\n📚 Your library is empty");
    } else {
        println!("\n📚 First {} books:", library_response.items.len().min(10));

        for (i, item) in library_response.items.iter().take(10).enumerate() {
            println!("\n{}. {}", i + 1, item.title);
            if let Some(subtitle) = &item.subtitle {
                println!("   Subtitle: {}", subtitle);
            }
            if !item.authors.is_empty() {
                let authors = item.authors.iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("   By: {}", authors);
            }
            if !item.narrators.is_empty() {
                let narrators = item.narrators.iter()
                    .map(|n| n.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("   Narrated by: {}", narrators);
            }
            if let Some(runtime) = item.length_in_minutes {
                let hours = runtime / 60;
                let mins = runtime % 60;
                println!("   Runtime: {}h {}m", hours, mins);
            }
            println!("   ASIN: {}", item.asin);
        }

        if library_response.items.len() > 10 {
            println!("\n... and {} more books", library_response.items.len() - 10);
        }
    }

    // Step 5: Summary
    print_header("LIBRARY SYNC COMPLETE");

    println!("\n📊 Summary:");
    println!("   ✅ Connected to Audible API");
    println!("   ✅ Library fetched successfully");
    println!("   📚 Total books: {}", library_response.total_results.unwrap_or(0));

    Ok(())
}

// ============================================================================
// Test 3: Token Refresh
// ============================================================================

/// Test token refresh flow
///
/// This test:
/// 1. Loads credentials
/// 2. Forces token to appear expired
/// 3. Refreshes tokens using refresh_token
/// 4. Saves new tokens
///
/// Run with:
/// ```bash
/// cargo test --test live_api_test test_03_token_refresh -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore] // Only run manually
async fn test_03_token_refresh() -> Result<()> {
    print_header("LIVE TEST 3: Token Refresh");

    // Step 1: Load credentials
    print_section("Step 1: Load Credentials");

    let mut account = load_credentials()?;
    let identity = account.identity.as_ref().unwrap();

    print_row("Account", &account.account_name);
    print_row("Current Token", &truncate(&identity.access_token.token, 40));
    print_row("Expires At", &identity.access_token.expires_at.to_string());

    // Step 2: Check current state
    print_section("Step 2: Check Token State");

    let needs_refresh = account.needs_token_refresh();
    println!("\nToken Status:");
    print_row("Needs Refresh", &needs_refresh.to_string());

    if !needs_refresh {
        println!("\n💡 Token is still valid, but we'll refresh anyway to test the flow");
    }

    // Step 3: Refresh tokens
    print_section("Step 3: Refresh Access Token");

    println!("\n🔄 Calling /auth/token with refresh_token...");

    let old_token = identity.access_token.token.clone();
    account.refresh_tokens().await?;

    let new_identity = account.identity.as_ref().unwrap();
    let new_token = &new_identity.access_token.token;

    println!("\n✅ Token Refresh Successful!");
    print_row("Old Token", &truncate(&old_token, 40));
    print_row("New Token", &truncate(new_token, 40));
    print_row("New Expires At", &new_identity.access_token.expires_at.to_string());
    print_row("Time Until Expiry", &format!("{:?}", new_identity.time_until_expiry()));

    // Verify tokens are different
    if old_token != *new_token {
        println!("\n✅ Token was successfully refreshed (new token received)");
    } else {
        println!("\n⚠️  Token appears unchanged (server may have cached)");
    }

    // Step 4: Save new credentials
    print_section("Step 4: Save Updated Credentials");

    save_credentials(&account)?;

    // Step 5: Verify new token works
    print_section("Step 5: Verify New Token Works");

    println!("\n🔄 Testing new token with API call...");

    let api_url = new_identity.locale.api_url();
    let options = LibraryOptions {
        number_of_results_per_page: 1,
        page_number: 1,
        ..Default::default()
    };

    let client = reqwest::Client::new();
    let library_response: LibraryResponse = client
        .get(format!("{}/1.0/library", api_url))
        .header("Authorization", format!("Bearer {}", new_identity.access_token.token))
        .query(&options)
        .send()
        .await?
        .json()
        .await?;

    println!("✅ New token works! Retrieved library with {} total items", library_response.total_results.unwrap_or(0));

    // Step 6: Summary
    print_header("TOKEN REFRESH COMPLETE");

    println!("\n📊 Summary:");
    println!("   ✅ Token refreshed successfully");
    println!("   ✅ New token validated with API call");
    println!("   ✅ Credentials saved");

    Ok(())
}

// ============================================================================
// Test 4: Activation Bytes Extraction
// ============================================================================

/// Test activation bytes extraction
///
/// This test:
/// 1. Loads credentials
/// 2. Calls license endpoint
/// 3. Extracts activation bytes from binary response
/// 4. Saves to account
///
/// Run with:
/// ```bash
/// cargo test --test live_api_test test_04_activation_bytes -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore] // Only run manually
async fn test_04_activation_bytes() -> Result<()> {
    print_header("LIVE TEST 4: Activation Bytes Extraction");

    // Step 1: Load credentials
    print_section("Step 1: Load Credentials");

    let mut account = load_credentials()?;

    print_row("Account", &account.account_name);
    print_row("Current Activation Bytes", &account.decrypt_key);

    if !account.decrypt_key.is_empty() {
        println!("\n💡 Account already has activation bytes");
        println!("   We'll fetch them again to verify");
    }

    // Check token
    if account.needs_token_refresh() {
        println!("\n⚠️  Token expired, refreshing...");
        account.refresh_tokens().await?;
    }

    // Step 2: Request activation bytes
    print_section("Step 2: Request Activation Bytes");

    let identity = account.identity.as_ref().unwrap();
    let endpoint = format!(
        "https://www.{}/license/token?action=register&player_manuf=Audible,iPhone&player_model=iPhone",
        identity.locale.domain
    );

    println!("\n🔄 Calling license endpoint...");
    print_row("Endpoint", &endpoint);
    print_row("Authorization", &format!("Bearer {}...", truncate(&identity.access_token.token, 30)));

    let activation_bytes = account.get_activation_bytes().await?;

    // Step 3: Display results
    print_section("Step 3: Activation Bytes Retrieved");

    println!("\n✅ Activation Bytes Extracted!");
    print_row("Activation Bytes", &activation_bytes);
    print_row("Length", &format!("{} characters (4 bytes as hex)", activation_bytes.len()));
    print_row("Format", "Lowercase hexadecimal (XXXXXXXX)");

    // Verify format
    assert_eq!(activation_bytes.len(), 8, "Activation bytes should be 8 hex characters");
    assert!(activation_bytes.chars().all(|c| c.is_ascii_hexdigit()), "Should be hex");

    // Step 4: Save to account
    print_section("Step 4: Save to Account");

    save_credentials(&account)?;

    println!("\n✅ Activation bytes saved to account");

    // Step 5: Explain usage
    print_section("Step 5: Usage");

    println!("\n🔐 These activation bytes are used for:");
    println!("   1. Decrypting AAX audiobook files");
    println!("   2. Converting AAX to M4B format");
    println!("   3. Removing Audible DRM");

    println!("\n💡 Example FFmpeg command:");
    println!("   ffmpeg -activation_bytes {} -i input.aax -c copy output.m4b", activation_bytes);

    // Step 6: Summary
    print_header("ACTIVATION BYTES COMPLETE");

    println!("\n📊 Summary:");
    println!("   ✅ Activation bytes retrieved from Audible");
    println!("   ✅ Format validated (8 hex chars)");
    println!("   ✅ Saved to account");
    println!("   🔐 Ready for DRM removal");

    Ok(())
}

// ============================================================================
// Test 5: Complete End-to-End Flow
// ============================================================================

/// Complete end-to-end test
///
/// This test runs the complete flow:
/// 1. Loads credentials (or prompts to run test_01 first)
/// 2. Refreshes tokens if needed
/// 3. Syncs library
/// 4. Gets activation bytes
/// 5. Verifies everything works
///
/// Run with:
/// ```bash
/// cargo test --test live_api_test test_05_complete_flow -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore] // Only run manually
async fn test_05_complete_flow() -> Result<()> {
    print_header("LIVE TEST 5: Complete End-to-End Flow");

    println!("\nThis test verifies the complete Audible integration:");
    println!("  ✓ Credential management");
    println!("  ✓ Token refresh");
    println!("  ✓ Library sync");
    println!("  ✓ Activation bytes");

    // Load credentials
    print_section("Loading Credentials");

    let mut account = load_credentials()?;
    println!("✅ Credentials loaded: {}", account.account_name);

    // Refresh if needed
    if account.needs_token_refresh() {
        print_section("Refreshing Tokens");
        account.refresh_tokens().await?;
        save_credentials(&account)?;
        println!("✅ Tokens refreshed");
    }

    // Fetch library
    let api_url = {
        let identity = account.identity.as_ref().unwrap();
        identity.locale.api_url()
    };

    print_section("Syncing Library");

    let options = LibraryOptions {
        number_of_results_per_page: 5,
        page_number: 1,
        ..Default::default()
    };

    let access_token = account.identity.as_ref().unwrap().access_token.token.clone();
    let client = reqwest::Client::new();
    let library: LibraryResponse = client
        .get(format!("{}/1.0/library", api_url))
        .header("Authorization", format!("Bearer {}", access_token))
        .query(&options)
        .send()
        .await?
        .json()
        .await?;

    println!("✅ Library synced: {} total books", library.total_results.unwrap_or(0));

    if !library.items.is_empty() {
        println!("\n📚 Sample books:");
        for (i, item) in library.items.iter().enumerate() {
            println!("   {}. {} by {}",
                i + 1,
                item.title,
                item.authors.first().map(|a| a.name.as_str()).unwrap_or("Unknown")
            );
        }
    }

    // Get activation bytes (if not already present)
    if account.decrypt_key.is_empty() {
        print_section("Getting Activation Bytes");

        let activation_bytes = account.get_activation_bytes().await?;
        println!("✅ Activation bytes: {}", activation_bytes);

        save_credentials(&account)?;
    } else {
        print_section("Activation Bytes");
        println!("✅ Already have activation bytes: {}", account.decrypt_key);
    }

    // Final summary
    let locale_name = account.identity.as_ref().unwrap().locale.name.clone();

    print_header("END-TO-END TEST COMPLETE");

    println!("\n🎉 All systems operational!");
    println!("\n📊 Account Status:");
    print_row("Name", &account.account_name);
    print_row("Locale", &locale_name);
    print_row("Library Size", &library.total_results.unwrap_or(0).to_string());
    print_row("Activation Bytes", &account.decrypt_key);
    print_row("Token Valid", &(!account.needs_token_refresh()).to_string());

    println!("\n✅ Ready for:");
    println!("   • Downloading audiobooks");
    println!("   • DRM removal");
    println!("   • Library management");

    Ok(())
}
