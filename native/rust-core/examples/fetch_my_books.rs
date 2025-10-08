//! Fetch actual books from library using test fixture credentials
//!
//! This example:
//! - Loads credentials from test_fixtures/registration_response.json
//! - Automatically refreshes expired tokens
//! - Saves refreshed tokens back to the file
//! - Fetches your actual audiobook library
//!
//! Usage:
//! ```bash
//! cargo run --example fetch_my_books
//! ```

use rust_core::api::{
    auth::{Locale, Account, refresh_access_token},
    registration::RegistrationResponse,
    library::{LibraryOptions, LibraryResponse},
};
use std::path::PathBuf;
use std::fs;

const TEST_FIXTURE_PATH: &str = "test_fixtures/registration_response.json";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("═══════════════════════════════════════════════════════════");
    println!("  Fetching Your Audible Library");
    println!("═══════════════════════════════════════════════════════════\n");

    // Step 1: Load registration response from file
    println!("📝 Step 1: Loading registration data from file...");
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(TEST_FIXTURE_PATH);
    let registration_json = fs::read_to_string(&fixture_path)?;
    let mut response = RegistrationResponse::from_json(&registration_json)?;
    println!("   ✅ Registration data loaded from: {}", TEST_FIXTURE_PATH);
    println!("   File: {}\n", fixture_path.display());

    // Step 2: Create identity and account
    println!("🔐 Step 2: Creating identity...");
    let locale = Locale::us();
    let identity = response.to_identity(locale.clone())?;
    println!("   ✅ Identity created");
    println!("   Account: {}", identity.customer_info.name);
    println!("   Region: {}\n", identity.locale.name);

    let mut account = Account::new(identity.amazon_account_id.clone())?;
    account.set_account_name(identity.customer_info.name.clone());
    account.set_identity(identity);

    // Step 3: Check token expiration and refresh if needed
    println!("⏰ Step 3: Checking token expiration...");
    let now = chrono::Utc::now();
    let expires_at = account.identity.as_ref().unwrap().access_token.expires_at;
    let time_until_expiry = expires_at - now;

    if now >= expires_at {
        println!("   ⚠️  Token expired {} ago", now - expires_at);
        println!("   Expires at: {}", expires_at);

        println!("\n🔄 Step 4: Refreshing access token...");
        println!("   Using refresh_token to get new access_token...");

        match account.refresh_tokens().await {
            Ok(()) => {
                println!("   ✅ Token refreshed successfully!");
                let new_identity = account.identity.as_ref().unwrap();
                let new_expires = new_identity.access_token.expires_at;
                println!("   New token expires at: {}", new_expires);
                println!("   Valid for: {}", format_duration(new_expires - now));

                // Update the response with new tokens
                response.response.success.tokens.bearer.access_token =
                    new_identity.access_token.token.clone();
                response.response.success.tokens.bearer.refresh_token =
                    new_identity.refresh_token.clone();

                // Calculate new expires_in (seconds from now)
                let new_expires_in = (new_expires - now).num_seconds();
                response.response.success.tokens.bearer.expires_in =
                    new_expires_in.to_string();

                // Save updated response back to file
                println!("\n💾 Step 5: Saving refreshed tokens to file...");
                let updated_json = serde_json::to_string_pretty(&response)?;
                fs::write(&fixture_path, updated_json)?;
                println!("   ✅ Updated tokens saved to: {}", TEST_FIXTURE_PATH);
                println!("   File: {}\n", fixture_path.display());
            }
            Err(e) => {
                println!("   ❌ Token refresh failed: {}", e);
                println!("\n   Please run the OAuth registration test to get fresh credentials:");
                println!("   cargo test test_01_interactive_oauth_registration -- --ignored --nocapture\n");
                return Err(e.into());
            }
        }
    } else {
        println!("   ✅ Token valid for: {}", format_duration(time_until_expiry));
        println!("   Expires at: {}\n", expires_at);
    }

    // Step 4 (or 6): Fetch library
    let step_num = if time_until_expiry.num_seconds() < 0 { 6 } else { 4 };
    println!("📚 Step {}: Fetching your library from Audible API...", step_num);

    let identity = account.identity.as_ref().unwrap();
    let api_url = identity.locale.api_url();
    println!("   API: {}", api_url);

    let options = LibraryOptions {
        number_of_results_per_page: 5,
        page_number: 1,
        ..Default::default()
    };

    let client = reqwest::Client::new();
    let http_response = client
        .get(format!("{}/1.0/library", api_url))
        .header("Authorization", format!("Bearer {}", identity.access_token.token))
        .query(&options)
        .send()
        .await?;

    let status = http_response.status();
    println!("   Response Status: {}", status);

    let response_text = http_response.text().await?;
    println!("   Response Preview: {}...\n", &response_text[..response_text.len().min(200)]);

    if !status.is_success() {
        if status.as_u16() == 403 && response_text.contains("could not be authenticated") {
            println!("   ⚠️  Authentication failed - token may be invalid");
            println!("\n🔄 Attempting to refresh token...");

            match account.refresh_tokens().await {
                Ok(()) => {
                    println!("   ✅ Token refreshed successfully!");
                    let new_identity = account.identity.as_ref().unwrap();
                    let new_expires = new_identity.access_token.expires_at;
                    println!("   New token expires at: {}", new_expires);

                    // Update the response with new tokens
                    response.response.success.tokens.bearer.access_token =
                        new_identity.access_token.token.clone();
                    response.response.success.tokens.bearer.refresh_token =
                        new_identity.refresh_token.clone();

                    let new_expires_in = (new_expires - now).num_seconds();
                    response.response.success.tokens.bearer.expires_in =
                        new_expires_in.to_string();

                    // Save updated response back to file
                    println!("\n💾 Saving refreshed tokens to file...");
                    let updated_json = serde_json::to_string_pretty(&response)?;
                    fs::write(&fixture_path, updated_json)?;
                    println!("   ✅ Updated tokens saved to: {}", TEST_FIXTURE_PATH);

                    // Retry the API call with new token
                    println!("\n🔄 Retrying library fetch with new token...");
                    let identity = account.identity.as_ref().unwrap();
                    let retry_response = client
                        .get(format!("{}/1.0/library", api_url))
                        .header("Authorization", format!("Bearer {}", identity.access_token.token))
                        .query(&options)
                        .send()
                        .await?;

                    let retry_status = retry_response.status();
                    let retry_text = retry_response.text().await?;

                    if !retry_status.is_success() {
                        println!("   ❌ API Error after refresh:");
                        println!("{}\n", retry_text);
                        return Err(format!("API still failing after token refresh: {}", retry_status).into());
                    }

                    let library: LibraryResponse = serde_json::from_str(&retry_text)?;
                    println!("   ✅ Library retrieved after token refresh!\n");

                    // Continue with displaying books
                    display_books(&library);
                    return Ok(());
                }
                Err(e) => {
                    println!("   ❌ Token refresh failed: {}", e);
                    println!("\n   API Error Response:");
                    println!("{}\n", response_text);
                    return Err(e.into());
                }
            }
        } else {
            println!("   ❌ API Error Response:");
            println!("{}\n", response_text);
            return Err(format!("API returned error status: {}", status).into());
        }
    }

    // Try to parse, but show raw data if it fails
    match serde_json::from_str::<LibraryResponse>(&response_text) {
        Ok(library) => {
            println!("   ✅ Library retrieved!\n");
            display_books(&library);
            Ok(())
        }
        Err(e) => {
            println!("   ⚠️  Parse error: {}", e);
            println!("\n   Response length: {} bytes", response_text.len());

            // Save full response to file for debugging
            let debug_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("test_fixtures/library_response_debug.json");
            match std::fs::write(&debug_path, &response_text) {
                Ok(_) => println!("   💾 Full response saved to: {}", debug_path.display()),
                Err(e) => println!("   ⚠️  Could not save debug file: {}", e),
            }

            println!("\n   Raw response (first 5000 chars):");
            println!("{}\n", &response_text[..response_text.len().min(5000)]);

            // Try to at least show some book data
            if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&response_text) {
                if let Some(items) = json_value["items"].as_array() {
                    println!("   Found {} items in response", items.len());
                    println!("\n   First item (pretty, first 2000 chars):");
                    let first_item_str = serde_json::to_string_pretty(&items[0]).unwrap_or_default();
                    println!("{}\n", &first_item_str[..first_item_str.len().min(2000)]);
                }
            }

            Err(e.into())
        }
    }
}

fn display_books(library: &LibraryResponse) {
    println!("═══════════════════════════════════════════════════════════");
    if let Some(total) = library.total_results {
        println!("  Your Audiobooks ({} total)", total);
    } else {
        println!("  Your Audiobooks");
    }
    println!("═══════════════════════════════════════════════════════════\n");

    if library.items.is_empty() {
        println!("📭 Your library is empty\n");
        return;
    }

    for (i, book) in library.items.iter().enumerate() {
        println!("{}. {}", i + 1, book.title);

        if let Some(subtitle) = &book.subtitle {
            println!("   Subtitle: {}", subtitle);
        }

        // Authors
        if !book.authors.is_empty() {
            let authors = book.authors
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            println!("   By: {}", authors);
        }

        // Narrators
        if !book.narrators.is_empty() {
            let narrators = book.narrators
                .iter()
                .map(|n| n.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            println!("   Narrated by: {}", narrators);
        }

        // Runtime
        if let Some(runtime) = book.length_in_minutes {
            let hours = runtime / 60;
            let mins = runtime % 60;
            println!("   Runtime: {}h {}m", hours, mins);
        }

        // Language
        if let Some(lang) = &book.language {
            println!("   Language: {}", lang);
        }

        // Publisher
        if let Some(publisher) = &book.publisher {
            println!("   Publisher: {}", publisher);
        }

        // Release date
        if let Some(release) = book.release_date {
            println!("   Released: {}", release);
        }

        // Purchase date
        println!("   Purchased: {}", book.purchase_date.format("%Y-%m-%d"));

        // ASIN
        println!("   ASIN: {}", book.asin);

        // Format/codec
        if !book.available_codecs.is_empty() {
            let codecs: Vec<String> = book.available_codecs
                .iter()
                .filter_map(|c| c.name.as_ref())
                .map(|s| s.clone())
                .collect();
            if !codecs.is_empty() {
                println!("   Formats: {}", codecs.join(", "));
            }
        }

        // Series
        if let Some(series_list) = &book.series {
            if !series_list.is_empty() {
                for series in series_list {
                    let title = series.title.as_deref().unwrap_or("Unknown Series");
                    let sequence = series.sequence.as_deref().unwrap_or("?");
                    println!("   Series: {} (Book {})", title, sequence);
                }
            }
        }

        // Rating
        if let Some(rating) = &book.rating {
            if let Some(overall) = &rating.overall_distribution {
                if let (Some(avg), Some(count)) = (overall.average_rating, overall.num_ratings) {
                    println!("   Rating: {:.1} stars ({} reviews)", avg, count);
                }
            }
        }

        // Description (truncated)
        if let Some(desc) = &book.description {
            let truncated = if desc.len() > 150 {
                format!("{}...", &desc[..150])
            } else {
                desc.clone()
            };
            println!("   Description: {}", truncated);
        }

        println!();
    }

    println!("═══════════════════════════════════════════════════════════");
    if let Some(total) = library.total_results {
        println!("  Total books in your library: {}", total);
        println!("  Showing: {} of {} books", library.items.len(), total);
    } else {
        println!("  Showing: {} books", library.items.len());
    }
    println!("═══════════════════════════════════════════════════════════\n");

    println!("💡 To see more books, modify the fetch_my_books.rs example");
    println!("   and change 'number_of_results_per_page: 5' to a higher number.\n");
}

fn format_duration(duration: chrono::Duration) -> String {
    let seconds = duration.num_seconds();
    if seconds < 60 {
        format!("{} seconds", seconds)
    } else if seconds < 3600 {
        format!("{} minutes", seconds / 60)
    } else {
        format!("{} hours {} minutes", seconds / 3600, (seconds % 3600) / 60)
    }
}
