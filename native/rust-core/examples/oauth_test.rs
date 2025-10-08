//! OAuth Authentication Test Tool
//!
//! This tool helps test the OAuth flow by:
//! 1. Generating an authorization URL
//! 2. Accepting the callback URL after login
//! 3. Exchanging the code for tokens
//!
//! Usage:
//! ```bash
//! cargo run --example oauth_test
//! ```

use rust_core::api::auth::{
    generate_authorization_url, parse_authorization_callback, exchange_authorization_code,
    Locale, PkceChallenge, OAuthState,
};
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🔐 Audible OAuth Authentication Test Tool\n");

    // Step 1: Generate device serial
    let device_serial = uuid::Uuid::new_v4()
        .to_string()
        .replace("-", "")
        .to_uppercase();

    println!("📱 Generated Device Serial: {}\n", device_serial);

    // Step 2: Select locale
    println!("📍 Select your Audible region:");
    println!("  1. United States (us)");
    println!("  2. United Kingdom (uk)");
    println!("  3. Germany (de)");
    println!("  4. France (fr)");
    println!("  5. Canada (ca)");
    print!("\nEnter number [1]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let locale = match input.trim() {
        "2" => Locale::uk(),
        "3" => Locale::de(),
        "4" => Locale::fr(),
        "5" => Locale::ca(),
        _ => Locale::us(),
    };

    println!("✓ Using locale: {} ({})\n", locale.name, locale.country_code);

    // Step 3: Generate PKCE and OAuth URL
    let pkce = PkceChallenge::generate()?;
    let state = OAuthState::generate();

    println!("🔑 PKCE Verifier: {}", pkce.verifier);
    println!("🎲 OAuth State: {}\n", state.value);

    let auth_url = generate_authorization_url(&locale, &device_serial, &pkce, &state)?;

    println!("{}", "=".repeat(80));
    println!("📋 STEP 1: Copy this URL and open it in your browser:");
    println!("{}", "=".repeat(80));
    println!("\n{}\n", auth_url);
    println!("{}", "=".repeat(80));
    println!("\n📝 Instructions:");
    println!("  1. Copy the URL above");
    println!("  2. Open it in your browser");
    println!("  3. Log in with your Audible account");
    println!("  4. After login, you'll be redirected to /ap/maplanding");
    println!("  5. Copy the ENTIRE URL from the address bar\n");

    // Step 4: Wait for callback URL
    print!("📥 Paste the callback URL here and press Enter:\n> ");
    io::stdout().flush()?;

    let mut callback_url = String::new();
    io::stdin().read_line(&mut callback_url)?;
    let callback_url = callback_url.trim();

    if callback_url.is_empty() {
        println!("❌ No callback URL provided. Exiting.");
        return Ok(());
    }

    println!("\n🔍 Parsing callback URL...");
    println!("URL: {}\n", callback_url);

    // Step 5: Parse authorization code
    match parse_authorization_callback(callback_url) {
        Ok(auth_code) => {
            println!("✅ Authorization Code: {}\n", auth_code);

            // Step 6: Exchange code for tokens
            println!("🔄 Exchanging authorization code for tokens...\n");

            match exchange_authorization_code(&locale, &auth_code, &device_serial, &pkce).await {
                Ok(tokens) => {
                    println!("\n🎉 SUCCESS! Authentication Complete!\n");
                    println!("{}", "=".repeat(80));
                    println!("Access Token: {}...", &tokens.access_token[..30]);
                    println!("Refresh Token: {}...", &tokens.refresh_token[..30]);
                    println!("Token Type: {}", tokens.token_type);
                    println!("Expires In: {} seconds", tokens.expires_in);
                    println!("{}", "=".repeat(80));
                    println!("\n✨ You can now use these tokens to access your Audible library!");
                }
                Err(e) => {
                    println!("\n❌ Token Exchange Failed!");
                    println!("Error: {:?}\n", e);
                    println!("This is the issue we need to fix.");
                }
            }
        }
        Err(e) => {
            println!("\n❌ Failed to Parse Callback URL!");
            println!("Error: {:?}\n", e);
            println!("Expected format:");
            println!("  https://www.amazon.com/ap/maplanding?...&openid.oa2.authorization_code=XXXXX");
        }
    }

    Ok(())
}
