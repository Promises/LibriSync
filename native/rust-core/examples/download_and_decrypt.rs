//! Complete download and decrypt pipeline
//!
//! This example demonstrates the full workflow:
//! 1. Request download license from Audible API
//! 2. Download encrypted AAX file with progress tracking
//! 3. Extract activation bytes from license
//! 4. Decrypt AAX → M4B using FFmpeg
//! 5. Verify playable output
//!
//! Usage:
//! ```bash
//! cargo run --example download_and_decrypt
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
use std::process::Command;

const TEST_FIXTURE_PATH: &str = "test_fixtures/registration_response.json";
const TEST_ASIN: &str = "B07T2F8VJM";
const ENCRYPTED_FILE: &str = "/tmp/book_encrypted.aax";
const DECRYPTED_FILE: &str = "/tmp/book_decrypted.m4b";
const USER_AGENT: &str = "Audible/671 CFNetwork/1240.0.4 Darwin/20.6.0";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("═══════════════════════════════════════════════════════════");
    println!("  Complete Download + Decrypt Pipeline");
    println!("  Book: B07T2F8VJM");
    println!("═══════════════════════════════════════════════════════════\n");

    // Step 1: Load account and create client
    println!("📝 Step 1: Loading account...");
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(TEST_FIXTURE_PATH);
    let registration_json = fs::read_to_string(&fixture_path)?;
    let response = RegistrationResponse::from_json(&registration_json)?;

    let locale = Locale::us();
    let identity = response.to_identity(locale)?;

    let mut account = Account::new(identity.amazon_account_id.clone())?;
    let account_name = identity.customer_info.name.clone();
    account.set_account_name(account_name.clone());
    account.set_identity(identity);

    let client = AudibleClient::new(account)?;
    println!("   ✅ Account: {}\n", account_name);

    // Step 2: Request download license
    println!("📥 Step 2: Requesting download license...");
    let license = client.build_download_license(TEST_ASIN, DownloadQuality::High, false).await?;
    println!("   ✅ License acquired");
    println!("   DRM: {:?}", license.drm_type);

    // Extract activation bytes
    let activation_bytes_hex = if let Some(ref keys) = license.decryption_keys {
        if !keys.is_empty() && keys[0].key_part_1.len() == 4 {
            let hex = keys[0].key_part_1.iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>();
            println!("   Activation Bytes: {}", hex);
            hex
        } else {
            return Err("No valid activation bytes in license".into());
        }
    } else {
        return Err("No decryption keys in license".into());
    };

    // Step 3: Download encrypted file
    println!("\n⬇️  Step 3: Downloading encrypted AAX file...");
    println!("   Output: {}", ENCRYPTED_FILE);

    let http_client = reqwest::Client::new();
    let response = http_client
        .get(&license.download_url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!("Download failed: HTTP {}", response.status()).into());
    }

    let total_size = response.content_length().unwrap_or(0);
    println!("   Size: {:.2} MB", total_size as f64 / (1024.0 * 1024.0));

    let mut file = tokio::fs::File::create(ENCRYPTED_FILE).await?;
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        // Progress every 10%
        if downloaded % (total_size / 10) < chunk.len() as u64 {
            let pct = (downloaded as f64 / total_size as f64) * 100.0;
            print!("   {:.0}%... ", pct);
            std::io::Write::flush(&mut std::io::stdout())?;
        }
    }
    file.flush().await?;
    println!("\n   ✅ Download complete!\n");

    // Step 4: Decrypt with FFmpeg
    println!("🔐 Step 4: Decrypting AAX → M4B...");
    println!("   Activation bytes: {}", activation_bytes_hex);
    println!("   Running ffmpeg...");

    let ffmpeg_status = Command::new("ffmpeg")
        .arg("-y")
        .arg("-activation_bytes")
        .arg(&activation_bytes_hex)
        .arg("-i")
        .arg(ENCRYPTED_FILE)
        .arg("-c")
        .arg("copy")
        .arg("-vn")
        .arg(DECRYPTED_FILE)
        .stderr(std::process::Stdio::null())  // Suppress ffmpeg output
        .status()?;

    if !ffmpeg_status.success() {
        return Err(format!("FFmpeg failed: {:?}", ffmpeg_status.code()).into());
    }
    println!("   ✅ Decryption complete!\n");

    // Step 5: Verify output
    println!("✓ Step 5: Verifying decrypted file...");
    let metadata = tokio::fs::metadata(DECRYPTED_FILE).await?;
    println!("   Size: {:.2} MB", metadata.len() as f64 / (1024.0 * 1024.0));

    // Get duration with ffprobe
    let ffprobe_output = Command::new("ffprobe")
        .arg("-v")
        .arg("quiet")
        .arg("-print_format")
        .arg("json")
        .arg("-show_format")
        .arg(DECRYPTED_FILE)
        .output()?;

    if ffprobe_output.status.success() {
        let format_info: serde_json::Value = serde_json::from_slice(&ffprobe_output.stdout)?;
        if let Some(duration) = format_info["format"]["duration"].as_str() {
            let secs: f64 = duration.parse()?;
            let hours = (secs / 3600.0) as u32;
            let mins = ((secs % 3600.0) / 60.0) as u32;
            println!("   Duration: {}h {}m ({:.0}s)", hours, mins, secs);
        }
        if let Some(title) = format_info["format"]["tags"]["title"].as_str() {
            println!("   Title: {}", title);
        }
        if let Some(artist) = format_info["format"]["tags"]["artist"].as_str() {
            println!("   Artist: {}", artist);
        }
    }

    println!("   ✅ Valid playable M4B file!");

    // Cleanup encrypted file (save space)
    let _ = tokio::fs::remove_file(ENCRYPTED_FILE).await;

    println!("\n═══════════════════════════════════════════════════════════");
    println!("  🎉 Pipeline Complete!");
    println!("═══════════════════════════════════════════════════════════");
    println!("\n✅ Downloaded, decrypted, and verified audiobook");
    println!("📂 Output: {}", DECRYPTED_FILE);
    println!("🎵 Play with: mpv {}", DECRYPTED_FILE);

    Ok(())
}
