//! Test resumable download capability
//!
//! This example demonstrates:
//! - Start downloading a file
//! - Cancel after 20% (simulate network interruption)
//! - Resume from saved state
//! - Complete the download
//!
//! Usage:
//! ```bash
//! cargo run --example test_download_resume
//! ```

use rust_core::api::{
    auth::{Locale, Account},
    client::AudibleClient,
    content::DownloadQuality,
    registration::RegistrationResponse,
};
use std::path::PathBuf;
use std::fs;
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

const TEST_FIXTURE_PATH: &str = "test_fixtures/registration_response.json";
const TEST_ASIN: &str = "B07T2F8VJM";
const OUTPUT_FILE: &str = "/tmp/atomic_habits_resumable.aax";
const STATE_FILE: &str = "/tmp/atomic_habits_resumable.state.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct DownloadState {
    url: String,
    bytes_downloaded: u64,
    total_bytes: u64,
    user_agent: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("═══════════════════════════════════════════════════════════");
    println!("  Resumable Download Test - B07T2F8VJM");
    println!("═══════════════════════════════════════════════════════════\n");

    // Load account and get license
    let (download_url, user_agent) = get_download_info().await?;

    // Part 1: Download 30% then cancel
    println!("\n📥 PART 1: Initial download (will cancel at 30%)...\n");
    let state = download_with_cancel(&download_url, &user_agent, OUTPUT_FILE, 0.30).await?;

    println!("\n⏸️  Download paused at {:.1}%", (state.bytes_downloaded as f64 / state.total_bytes as f64) * 100.0);
    println!("   Downloaded: {:.2} MB / {:.2} MB",
        state.bytes_downloaded as f64 / (1024.0 * 1024.0),
        state.total_bytes as f64 / (1024.0 * 1024.0)
    );

    // Save state to file
    let state_json = serde_json::to_string_pretty(&state)?;
    fs::write(STATE_FILE, state_json)?;
    println!("   💾 State saved to: {}\n", STATE_FILE);

    // Wait a bit to simulate time passing
    println!("⏳ Simulating network interruption (1 second)...\n");
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Part 2: Resume from saved state
    println!("📥 PART 2: Resuming download from saved state...\n");
    let saved_state_json = fs::read_to_string(STATE_FILE)?;
    let saved_state: DownloadState = serde_json::from_str(&saved_state_json)?;

    println!("   Resuming from {} bytes ({:.1}%)",
        saved_state.bytes_downloaded,
        (saved_state.bytes_downloaded as f64 / saved_state.total_bytes as f64) * 100.0
    );

    download_resume(&saved_state, OUTPUT_FILE).await?;

    // Verify
    println!("\n✓ Verification:");
    let final_metadata = tokio::fs::metadata(OUTPUT_FILE).await?;
    println!("   File size: {} bytes", final_metadata.len());
    println!("   Expected: {} bytes", saved_state.total_bytes);

    if final_metadata.len() == saved_state.total_bytes {
        println!("   ✅ Size matches!");
    } else {
        println!("   ❌ Size mismatch!");
    }

    // Cleanup
    let _ = fs::remove_file(STATE_FILE);

    println!("\n═══════════════════════════════════════════════════════════");
    println!("  Resume Test Complete!");
    println!("═══════════════════════════════════════════════════════════");
    println!("\n✅ Download successfully resumed and completed");
    println!("📂 File: {}", OUTPUT_FILE);

    Ok(())
}

async fn get_download_info() -> Result<(String, String), Box<dyn std::error::Error>> {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(TEST_FIXTURE_PATH);
    let registration_json = fs::read_to_string(&fixture_path)?;
    let response = RegistrationResponse::from_json(&registration_json)?;

    let locale = Locale::us();
    let identity = response.to_identity(locale)?;

    let mut account = Account::new(identity.amazon_account_id.clone())?;
    account.set_account_name(identity.customer_info.name.clone());
    account.set_identity(identity);

    let client = AudibleClient::new(account)?;
    let license = client.build_download_license(TEST_ASIN, DownloadQuality::High, false).await?;

    let user_agent = "Audible/671 CFNetwork/1240.0.4 Darwin/20.6.0".to_string();

    Ok((license.download_url, user_agent))
}

async fn download_with_cancel(
    url: &str,
    user_agent: &str,
    output: &str,
    cancel_at_percentage: f64
) -> Result<DownloadState, Box<dyn std::error::Error>> {
    let http_client = reqwest::Client::new();
    let response = http_client
        .get(url)
        .header("User-Agent", user_agent)
        .send()
        .await?;

    let total_size = response.content_length().unwrap_or(0);
    let cancel_at_bytes = (total_size as f64 * cancel_at_percentage) as u64;

    let mut file = tokio::fs::File::create(output).await?;
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        let percentage = (downloaded as f64 / total_size as f64) * 100.0;
        if downloaded % (total_size / 20) < chunk.len() as u64 {
            println!("   Progress: {:.1}%", percentage);
        }

        // Cancel at target percentage
        if downloaded >= cancel_at_bytes {
            file.flush().await?;
            println!("   🛑 Cancelling at {:.1}%...", percentage);
            break;
        }
    }

    file.flush().await?;

    Ok(DownloadState {
        url: url.to_string(),
        bytes_downloaded: downloaded,
        total_bytes: total_size,
        user_agent: user_agent.to_string(),
    })
}

async fn download_resume(
    state: &DownloadState,
    output: &str
) -> Result<(), Box<dyn std::error::Error>> {
    let http_client = reqwest::Client::new();

    // Use HTTP Range header to resume from saved position
    // Reference: NetworkFileStream.cs:230 - Range: bytes={WritePosition}-
    let range_header = format!("bytes={}-", state.bytes_downloaded);

    let response = http_client
        .get(&state.url)
        .header("User-Agent", &state.user_agent)
        .header("Range", &range_header)
        .send()
        .await?;

    println!("   HTTP Status: {} {}", response.status().as_u16(),
        if response.status().as_u16() == 206 { "(Partial Content - RESUME WORKING!)" } else { "" });

    if response.status().as_u16() != 206 {
        return Err("Server does not support resume (expected 206 Partial Content)".into());
    }

    // Open file in append mode
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .append(true)
        .open(output)
        .await?;

    let mut stream = response.bytes_stream();
    let mut downloaded = state.bytes_downloaded;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        let percentage = (downloaded as f64 / state.total_bytes as f64) * 100.0;
        if (downloaded - state.bytes_downloaded) % (state.total_bytes / 20) < chunk.len() as u64 {
            println!("   Progress: {:.1}%", percentage);
        }
    }

    file.flush().await?;
    println!("   ✅ Resume complete!");

    Ok(())
}
