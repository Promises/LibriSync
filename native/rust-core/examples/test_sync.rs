//! Test library sync with actual Audible account
//!
//! This example mimics what the React Native app does:
//! - Loads account from test fixture
//! - Creates AudibleClient
//! - Syncs library to database
//!
//! Usage:
//! ```bash
//! cargo run --example test_sync
//! ```

use rust_core::api::{
    auth::{Locale, Account},
    registration::RegistrationResponse,
};
use rust_core::storage::Database;
use std::path::PathBuf;
use std::fs;

const TEST_FIXTURE_PATH: &str = "test_fixtures/registration_response.json";
const DB_PATH: &str = "/tmp/test_audible.db";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("═══════════════════════════════════════════════════════════");
    println!("  Testing Library Sync");
    println!("═══════════════════════════════════════════════════════════\n");

    // Step 1: Load registration response
    println!("📝 Step 1: Loading registration data...");
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(TEST_FIXTURE_PATH);
    let registration_json = fs::read_to_string(&fixture_path)?;
    let response = RegistrationResponse::from_json(&registration_json)?;
    println!("   ✅ Loaded\n");

    // Step 2: Create identity and account
    println!("🔐 Step 2: Creating account...");
    let locale = Locale::us();
    let identity = response.to_identity(locale.clone())?;

    let mut account = Account::new(identity.amazon_account_id.clone())?;
    account.set_account_name(identity.customer_info.name.clone());
    account.set_identity(identity);
    println!("   ✅ Account created");
    println!("   Name: {}", account.account_name);
    println!("   ID: {}\n", account.account_id);

    // Step 3: Initialize database
    println!("💾 Step 3: Initializing database...");
    let db = Database::new(DB_PATH).await?;
    println!("   ✅ Database initialized: {}\n", DB_PATH);

    // Step 4: Fetch all books with 20 per page until empty
    println!("🔄 Step 4: Fetching all books (20 per page)...");
    let mut client = rust_core::api::client::AudibleClient::new(account.clone())?;
    let mut page_num = 1;
    let mut total_books = 0;
    let target_asin = "B073PG4DX8";
    let mut found_target = false;

    loop {
        let mut options = rust_core::api::library::LibraryOptions::default();
        options.page_number = page_num;
        options.number_of_results_per_page = 20;

        println!("\n   Page {}: ", page_num);
        let response: Result<rust_core::api::library::LibraryResponse, _> = client.get_with_query("/1.0/library", &options).await;

        match response {
            Ok(r) => {
                if r.items.is_empty() {
                    println!("   (no more books - end of library)");
                    break;
                }

                println!("   Retrieved {} books", r.items.len());
                for (i, item) in r.items.iter().enumerate() {
                    total_books += 1;
                    let marker = if item.asin == target_asin { " 🎯 TARGET FOUND!" } else { "" };
                    println!("   Book #{}: ✅ {} (ASIN: {}){}", total_books, item.title, item.asin, marker);

                    if item.asin == target_asin {
                        found_target = true;
                    }
                }

                page_num += 1;
            }
            Err(e) => {
                println!("   ❌ FAILED!");
                println!("\n=== PROBLEMATIC PAGE FOUND ===");
                println!("Page: {}", page_num);
                println!("Error: {}\n", e);
                break;
            }
        }
    }

    println!("\n📊 Summary:");
    println!("   Total books fetched: {}", total_books);
    println!("   Target ASIN {}: {}", target_asin, if found_target { "✅ FOUND" } else { "❌ NOT FOUND" });

    println!("\n═══════════════════════════════════════════════════════════");
    println!("  Test Complete!");
    println!("═══════════════════════════════════════════════════════════");

    Ok(())
}
