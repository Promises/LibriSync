//! Test naming patterns with real library data
//!
//! This example:
//! - Loads full library from test fixture
//! - Tests all three naming patterns
//! - Shows examples of generated file paths
//!
//! Usage:
//! ```bash
//! cargo run --example test_naming_patterns
//! ```

use rust_core::{
    api::library::LibraryResponse,
    audio::metadata::{AudioMetadata, SeriesInfo},
    file::paths::{build_file_path, NamingPattern},
};
use std::fs;

const LIBRARY_JSON: &str = "test_fixtures/full_library.json";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n═══════════════════════════════════════════════════════════");
    println!("  Naming Pattern Test");
    println!("═══════════════════════════════════════════════════════════\n");

    // Load library
    println!("📝 Loading library from: {}", LIBRARY_JSON);
    let library_json = fs::read_to_string(LIBRARY_JSON)?;
    let library: LibraryResponse = serde_json::from_str(&library_json)?;
    println!("   ✅ Loaded {} books\n", library.items.len());

    // Test patterns
    let patterns = vec![
        ("Flat File", NamingPattern::FlatFile),
        ("Author/Book Folder", NamingPattern::AuthorBookFolder),
        ("Author/Series+Book", NamingPattern::AuthorSeriesBook),
    ];

    println!("═══════════════════════════════════════════════════════════");
    println!("  Testing Naming Patterns");
    println!("═══════════════════════════════════════════════════════════\n");

    // Test with various book types
    let test_books = vec![
        // Book with series
        library.items.iter().find(|b| b.title.contains("Bobiverse") || b.title.contains("All These Worlds")),
        // Book with series and colon in title
        library.items.iter().find(|b| b.title.contains("Cirque") && b.title.contains(":")),
        // Book without series
        library.items.iter().find(|b| b.title == "The Martian"),
        // Book where title equals series name
        library.items.iter().find(|b| b.title == "I, Starship"),
        // Book with long title
        library.items.iter().find(|b| b.title.len() > 50),
    ];

    for (i, book_opt) in test_books.iter().enumerate() {
        if let Some(book) = book_opt {
            println!("{}. {}", i + 1, book.title);
            if let Some(series_list) = &book.series {
                if let Some(series) = series_list.first() {
                    println!("   Series: {} #{}",
                        series.title.as_deref().unwrap_or("?"),
                        series.sequence.as_deref().unwrap_or("?")
                    );
                }
            }
            println!("   Author: {}", book.authors.first().map(|a| a.name.as_str()).unwrap_or("Unknown"));

            let metadata = book_to_metadata(book);

            println!();
            for (pattern_name, pattern) in &patterns {
                match build_file_path(&metadata, *pattern, "m4b") {
                    Ok(path) => println!("   {} -> {}", pattern_name, path),
                    Err(e) => println!("   {} -> ERROR: {}", pattern_name, e),
                }
            }
            println!();
        }
    }

    // Show statistics
    println!("\n═══════════════════════════════════════════════════════════");
    println!("  Special Character Handling");
    println!("═══════════════════════════════════════════════════════════\n");

    // Test problematic titles
    let problematic = library.items.iter()
        .filter(|b| b.title.contains(':'))
        .take(5);

    for book in problematic {
        let metadata = book_to_metadata(book);
        println!("Original: {}", book.title);
        match build_file_path(&metadata, NamingPattern::FlatFile, "m4b") {
            Ok(path) => println!("Sanitized: {}\n", path),
            Err(e) => println!("Error: {}\n", e),
        }
    }

    println!("═══════════════════════════════════════════════════════════\n");

    Ok(())
}

fn book_to_metadata(book: &rust_core::api::library::LibraryItem) -> AudioMetadata {
    let series = book.series.as_ref().and_then(|list| list.first()).map(|s| SeriesInfo {
        name: s.title.clone().unwrap_or_default(),
        position: s.sequence.clone(),
    });

    AudioMetadata {
        title: book.title.clone(),
        authors: book.authors.iter().map(|a| a.name.clone()).collect(),
        narrators: book.narrators.iter().map(|n| n.name.clone()).collect(),
        publisher: book.publisher.clone(),
        publication_date: book.release_date.map(|d| d.to_string()),
        language: book.language.clone(),
        series,
        description: book.description.clone(),
        genres: vec![],
        runtime_minutes: book.length_in_minutes,
        asin: Some(book.asin.clone()),
        cover_art_url: None,
    }
}
