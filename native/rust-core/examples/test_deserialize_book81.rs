//! Try to deserialize the saved book #81 JSON
//!
//! Usage:
//! ```bash
//! cargo run --example test_deserialize_book81
//! ```

use rust_core::api::library::LibraryResponse;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Reading /tmp/book81_response.json...");

    let json_str = fs::read_to_string("/tmp/book81_response.json")?;
    println!("JSON length: {} bytes", json_str.len());

    println!("\nAttempting to deserialize...");

    match serde_json::from_str::<LibraryResponse>(&json_str) {
        Ok(response) => {
            println!("✅ SUCCESS!");
            println!("   Items: {}", response.items.len());
            if let Some(book) = response.items.first() {
                println!("   Title: {}", book.title);
                println!("   ASIN: {}", book.asin);
            }
        }
        Err(e) => {
            println!("❌ DESERIALIZATION FAILED!");
            println!("\nError: {}", e);
            println!("Line: {}", e.line());
            println!("Column: {}", e.column());

            // Show context
            let col = e.column();
            if col > 50 && col < json_str.len() {
                let start = col.saturating_sub(50);
                let end = (col + 100).min(json_str.len());
                println!("\nContext:");
                println!("{}", &json_str[start..end]);
                println!("{}^", " ".repeat(50));
            }
        }
    }

    Ok(())
}
