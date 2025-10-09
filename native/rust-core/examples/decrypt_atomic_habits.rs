//! Decrypt the downloaded AAX file
//!
//! This example:
//! - Loads account from test fixture
//! - Gets activation bytes from Audible API
//! - Decrypts the AAX file to playable M4B
//!
//! Usage:
//! ```bash
//! cargo run --example decrypt_atomic_habits
//! ```

use rust_core::api::{
    auth::{Locale, Account, get_activation_bytes},
    registration::RegistrationResponse,
};
use rust_core::crypto::activation::ActivationBytes;
use std::path::PathBuf;
use std::fs;
use std::process::Command;

const TEST_FIXTURE_PATH: &str = "test_fixtures/registration_response.json";
const INPUT_FILE: &str = "/tmp/atomic_habits.aax";
const OUTPUT_FILE: &str = "/tmp/atomic_habits_decrypted.m4b";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("═══════════════════════════════════════════════════════════");
    println!("  Decrypt Atomic Habits AAX → M4B");
    println!("═══════════════════════════════════════════════════════════\n");

    // Step 1: Load account
    println!("📝 Step 1: Loading account from fixture...");
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(TEST_FIXTURE_PATH);
    let registration_json = fs::read_to_string(&fixture_path)?;
    let response = RegistrationResponse::from_json(&registration_json)?;

    let locale = Locale::us();
    let identity = response.to_identity(locale.clone())?;

    let mut account = Account::new(identity.amazon_account_id.clone())?;
    account.set_account_name(identity.customer_info.name.clone());
    account.set_identity(identity);
    println!("   ✅ Account: {}\n", account.account_name);

    // Step 2: Get activation bytes
    println!("🔓 Step 2: Retrieving activation bytes from Audible...");

    let activation_bytes_result = get_activation_bytes(
        &locale,
        &account.identity.as_ref().unwrap().access_token.token
    ).await;

    let activation_bytes_hex = match activation_bytes_result {
        Ok(bytes) => {
            println!("   ✅ Activation bytes: {}", bytes);
            bytes
        }
        Err(e) => {
            eprintln!("   ❌ Failed to get activation bytes: {:?}", e);
            println!("\n💡 Trying to parse from license_response instead...");

            // Alternative: parse from license response
            let license_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("test_fixtures/download_license_b07t2f8vjm.json");

            if license_path.exists() {
                println!("   📄 Found license file, but license_response parsing not implemented yet");
                println!("   ⚠️  Cannot decrypt without activation bytes");
                return Err("Activation bytes unavailable".into());
            } else {
                return Err(format!("Failed to get activation bytes: {:?}", e).into());
            }
        }
    };

    // Step 3: Decrypt with ffmpeg
    println!("\n🔐 Step 3: Decrypting AAX file...");
    println!("   Input: {}", INPUT_FILE);
    println!("   Output: {}", OUTPUT_FILE);
    println!("   Activation bytes: {}", activation_bytes_hex);

    // Reference: crypto/aax.rs - build_ffmpeg_command()
    let activation_bytes = ActivationBytes::from_hex(&activation_bytes_hex)?;

    let ffmpeg_status = Command::new("ffmpeg")
        .arg("-y")  // Overwrite output
        .arg("-activation_bytes")
        .arg(activation_bytes.to_hex())
        .arg("-i")
        .arg(INPUT_FILE)
        .arg("-c")
        .arg("copy")
        .arg("-vn")  // No video
        .arg(OUTPUT_FILE)
        .status()?;

    if ffmpeg_status.success() {
        println!("   ✅ Decryption successful!\n");
    } else {
        println!("   ❌ FFmpeg failed with status: {:?}\n", ffmpeg_status.code());
        return Err("FFmpeg decryption failed".into());
    }

    // Step 4: Verify output
    println!("✓ Step 4: Verifying output file...");
    let metadata = tokio::fs::metadata(OUTPUT_FILE).await?;
    println!("   File size: {:.2} MB", metadata.len() as f64 / (1024.0 * 1024.0));

    // Check with ffprobe
    let ffprobe_output = Command::new("ffprobe")
        .arg("-v")
        .arg("quiet")
        .arg("-print_format")
        .arg("json")
        .arg("-show_format")
        .arg(OUTPUT_FILE)
        .output()?;

    if ffprobe_output.status.success() {
        let format_info: serde_json::Value = serde_json::from_slice(&ffprobe_output.stdout)?;
        if let Some(duration) = format_info["format"]["duration"].as_str() {
            let duration_secs: f64 = duration.parse()?;
            let hours = duration_secs / 3600.0;
            println!("   Duration: {:.2} hours ({:.0} seconds)", hours, duration_secs);
        }
        println!("   ✅ Valid audio file!");
    }

    println!("\n═══════════════════════════════════════════════════════════");
    println!("  Decryption Complete!");
    println!("═══════════════════════════════════════════════════════════");
    println!("\n📂 Decrypted file: {}", OUTPUT_FILE);
    println!("\n💡 You can now play the file:");
    println!("   mpv {}", OUTPUT_FILE);
    println!("   or");
    println!("   ffplay {}", OUTPUT_FILE);

    Ok(())
}
