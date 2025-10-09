//! Test actual download of book B07T2F8VJM
//!
//! This example:
//! - Loads account from test fixture
//! - Requests download license
//! - Downloads the encrypted AAX file
//! - Shows download progress
//!
//! Usage:
//! ```bash
//! cargo run --example test_download_b07t2f8vjm
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
const TEST_ASIN: &str = "B07T2F8VJM"; // "Atomic Habits" by James Clear
const OUTPUT_FILE: &str = "/tmp/atomic_habits.aax";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("═══════════════════════════════════════════════════════════");
    println!("  Download Book B07T2F8VJM Test");
    println!("═══════════════════════════════════════════════════════════\n");

    // Step 1: Load account
    println!("📝 Step 1: Loading account from fixture...");
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(TEST_FIXTURE_PATH);
    let registration_json = fs::read_to_string(&fixture_path)?;
    let response = RegistrationResponse::from_json(&registration_json)?;

    let locale = Locale::us();
    let identity = response.to_identity(locale)?;

    let mut account = Account::new(identity.amazon_account_id.clone())?;
    account.set_account_name(identity.customer_info.name.clone());
    account.set_identity(identity);
    println!("   ✅ Account: {}\n", account.account_name);

    // Step 2: Create API client
    println!("🔧 Step 2: Creating API client...");
    let client = AudibleClient::new(account)?;
    println!("   ✅ Client created\n");

    // Step 3: Request download license
    println!("📥 Step 3: Requesting download license...");
    println!("   ASIN: {}", TEST_ASIN);
    println!("   Quality: High");

    let license = client.build_download_license(
        TEST_ASIN,
        DownloadQuality::High,
        false // Use AAX/AAXC, not Widevine
    ).await?;

    println!("   ✅ License acquired");
    println!("   DRM Type: {:?}", license.drm_type);
    println!("   URL: {}...\n", &license.download_url[..80]);

    // Step 4: Download the file
    println!("⬇️  Step 4: Downloading audiobook file...");
    println!("   Output: {}", OUTPUT_FILE);
    println!("   Starting download...\n");

    // CloudFront requires User-Agent header
    // Reference: DownloadOptions.cs:31 - UserAgent => AudibleApi.Resources.Download_User_Agent
    // Reference: NetworkFileStream.cs:204 - RequestHeaders["User-Agent"]
    let user_agent = "Audible/671 CFNetwork/1240.0.4 Darwin/20.6.0";

    let http_client = reqwest::Client::new();
    let response = http_client
        .get(&license.download_url)
        .header("User-Agent", user_agent)
        .send()
        .await?;

    if !response.status().is_success() {
        eprintln!("❌ HTTP {}: {}", response.status(), response.status().canonical_reason().unwrap_or("Unknown"));
        return Err("Download request failed".into());
    }

    let total_size = response.content_length().unwrap_or(0);
    let total_mb = total_size as f64 / (1024.0 * 1024.0);
    println!("   Total size: {:.2} MB ({} bytes)", total_mb, total_size);

    let mut file = tokio::fs::File::create(OUTPUT_FILE).await?;
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_report = 0u64;
    let report_interval = total_size / 20; // Report every 5%

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        // Report progress every 5%
        if downloaded - last_report >= report_interval || downloaded == total_size {
            let percentage = (downloaded as f64 / total_size as f64) * 100.0;
            let mb_downloaded = downloaded as f64 / (1024.0 * 1024.0);
            println!("   Progress: {:.1}% ({:.2} MB / {:.2} MB)",
                percentage, mb_downloaded, total_mb);
            last_report = downloaded;
        }
    }

    file.flush().await?;
    println!("\n   ✅ Download complete!");

    // Step 5: Verify file
    println!("\n✓ Step 5: Verifying downloaded file...");
    let file_metadata = tokio::fs::metadata(OUTPUT_FILE).await?;
    let actual_size = file_metadata.len();
    println!("   File size: {} bytes", actual_size);

    if actual_size == total_size {
        println!("   ✅ Size matches expected!");
    } else {
        println!("   ⚠️  Size mismatch: expected {}, got {}", total_size, actual_size);
    }

    println!("\n═══════════════════════════════════════════════════════════");
    println!("  Download Complete!");
    println!("═══════════════════════════════════════════════════════════");
    println!("\n📂 Downloaded file: {}", OUTPUT_FILE);
    println!("📊 Size: {:.2} MB", actual_size as f64 / (1024.0 * 1024.0));
    println!("\n💡 Next steps:");
    println!("   • Parse license_response to extract activation bytes");
    println!("   • Decrypt AAX file using activation bytes");
    println!("   • Convert to M4B format");

    Ok(())
}
