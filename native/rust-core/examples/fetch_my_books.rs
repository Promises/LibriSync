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
use std::collections::HashSet;

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

    // First, get total count
    let first_page_options = LibraryOptions {
        number_of_results_per_page: 1,
        page_number: 1,
        ..Default::default()
    };

    let client = reqwest::Client::new();
    let first_response = client
        .get(format!("{}/1.0/library", api_url))
        .header("Authorization", format!("Bearer {}", identity.access_token.token))
        .query(&first_page_options)
        .send()
        .await?;

    let first_text = first_response.text().await?;
    let first_library: LibraryResponse = serde_json::from_str(&first_text)?;
    let total_books = first_library.total_results.unwrap_or(0);

    println!("   Total books in library: {}", total_books);
    println!("   Fetching all books...\n");

    // Now fetch all books (max 1000 per page)
    let options = LibraryOptions {
        number_of_results_per_page: 1000,
        page_number: 1,
        ..Default::default()
    };

    let http_response = client
        .get(format!("{}/1.0/library", api_url))
        .header("Authorization", format!("Bearer {}", identity.access_token.token))
        .query(&options)
        .send()
        .await?;

    let status = http_response.status();
    println!("   Response Status: {}", status);

    let response_text = http_response.text().await?;

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

            // Save full library JSON to file for analysis
            let library_json_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("test_fixtures/full_library.json");
            fs::write(&library_json_path, &response_text)?;
            println!("💾 Full library saved to: {}\n", library_json_path.display());

            display_books(&library);
            analyze_naming_conventions(&library);
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
}

fn analyze_naming_conventions(library: &LibraryResponse) {
    println!("\n═══════════════════════════════════════════════════════════");
    println!("  Naming Convention Analysis");
    println!("═══════════════════════════════════════════════════════════\n");

    let mut special_chars_in_titles = HashSet::new();
    let mut special_chars_in_authors = HashSet::new();
    let mut special_chars_in_series = HashSet::new();
    let mut problematic_titles = Vec::new();
    let mut books_with_series = 0;
    let mut books_with_subtitles = 0;

    for book in &library.items {
        // Analyze title
        for ch in book.title.chars() {
            if !ch.is_alphanumeric() && ch != ' ' {
                special_chars_in_titles.insert(ch);

                // Flag potentially problematic characters for filenames
                if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                    problematic_titles.push((book.title.clone(), ch));
                }
            }
        }

        // Check for subtitle
        if book.subtitle.is_some() {
            books_with_subtitles += 1;
        }

        // Analyze authors
        for author in &book.authors {
            for ch in author.name.chars() {
                if !ch.is_alphanumeric() && ch != ' ' {
                    special_chars_in_authors.insert(ch);
                }
            }
        }

        // Analyze series
        if let Some(series_list) = &book.series {
            if !series_list.is_empty() {
                books_with_series += 1;
                for series in series_list {
                    if let Some(title) = &series.title {
                        for ch in title.chars() {
                            if !ch.is_alphanumeric() && ch != ' ' {
                                special_chars_in_series.insert(ch);
                            }
                        }
                    }
                }
            }
        }
    }

    // Report findings
    println!("📊 Statistics:");
    println!("   Total books: {}", library.items.len());
    println!("   Books with series: {} ({:.1}%)", books_with_series, (books_with_series as f64 / library.items.len() as f64) * 100.0);
    println!("   Books with subtitles: {} ({:.1}%)", books_with_subtitles, (books_with_subtitles as f64 / library.items.len() as f64) * 100.0);

    println!("\n🔤 Special Characters Found:");
    println!("   In titles: {}", format_char_set(&special_chars_in_titles));
    println!("   In authors: {}", format_char_set(&special_chars_in_authors));
    println!("   In series: {}", format_char_set(&special_chars_in_series));

    if !problematic_titles.is_empty() {
        println!("\n⚠️  Problematic Characters for Filenames ({} books):", problematic_titles.len());
        for (title, ch) in problematic_titles.iter().take(10) {
            println!("   '{}' in: {}", ch, title);
        }
        if problematic_titles.len() > 10 {
            println!("   ... and {} more", problematic_titles.len() - 10);
        }
    }

    // Show sample naming patterns
    println!("\n📝 Sample Naming Patterns:\n");
    for (i, book) in library.items.iter().take(5).enumerate() {
        let authors = book.authors.iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        let series_info = if let Some(series_list) = &book.series {
            series_list.first().and_then(|s| {
                let title = s.title.as_deref().unwrap_or("Unknown");
                let seq = s.sequence.as_deref().unwrap_or("?");
                Some(format!(" [{}{}]", title, if seq != "?" { format!(" #{}", seq) } else { String::new() }))
            })
        } else {
            None
        };

        println!("{}. Author/Title format:", i + 1);
        println!("   {}/{}.m4b",
            sanitize_filename(&authors),
            sanitize_filename(&book.title)
        );

        println!("   Series/Title format:");
        if let Some(series) = series_info {
            println!("   {}/{}.m4b",
                sanitize_filename(&series.trim_start_matches(' ').trim_start_matches('[')),
                sanitize_filename(&book.title)
            );
        } else {
            println!("   (no series)");
        }

        println!();
    }

    println!("═══════════════════════════════════════════════════════════\n");
}

fn format_char_set(chars: &HashSet<char>) -> String {
    if chars.is_empty() {
        return "none".to_string();
    }
    let mut sorted: Vec<_> = chars.iter().collect();
    sorted.sort();
    sorted.iter().map(|c| format!("'{}'", c)).collect::<Vec<_>>().join(", ")
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
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
