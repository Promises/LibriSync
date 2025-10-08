//! Test book #81 specifically to find the parsing issue
//!
//! Usage:
//! ```bash
//! cargo run --example test_book_81
//! ```

use rust_core::api::{
    auth::{Locale, Account},
    registration::RegistrationResponse,
    library::LibraryOptions,
};
use std::path::PathBuf;
use std::fs;

const TEST_FIXTURE_PATH: &str = "test_fixtures/registration_response.json";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing Book #81 Specifically\n");

    // Load account
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(TEST_FIXTURE_PATH);
    let registration_json = fs::read_to_string(&fixture_path)?;
    let response = RegistrationResponse::from_json(&registration_json)?;

    let locale = Locale::us();
    let identity = response.to_identity(locale.clone())?;

    let mut account = Account::new(identity.amazon_account_id.clone())?;
    account.set_account_name(identity.customer_info.name.clone());
    account.set_identity(identity);

    // Create client
    let mut client = rust_core::api::client::AudibleClient::new(account.clone())?;

    // Fetch book #81 with page_size=1
    let mut options = LibraryOptions::default();
    options.page_number = 81;
    options.number_of_results_per_page = 1;

    println!("Fetching book #81 raw JSON...");

    // Build request manually to get raw response
    let api_url = "https://api.audible.com/1.0/library";

    let response = reqwest::Client::new()
        .get(api_url)
        .query(&options)
        .header("Authorization", format!("Bearer {}", account.identity.as_ref().unwrap().access_token.token))
        .header("Accept", "application/json")
        .send()
        .await?;

    let response_text = response.text().await?;

    // Save to file
    let output_path = "/tmp/book81_response.json";
    fs::write(output_path, &response_text)?;

    println!("✅ Raw JSON saved to: {}", output_path);
    println!("   Length: {} bytes", response_text.len());

    // Show snippet around column 8228
    if response_text.len() > 8228 {
        println!("\n   Char at column 8228:");
        let start = 8200;
        let end = 8260.min(response_text.len());
        println!("   {}", &response_text[start..end]);
    }

    Ok(())
}
