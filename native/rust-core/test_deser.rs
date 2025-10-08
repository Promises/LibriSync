use rust_core::api::library::LibraryResponse;
use std::fs;

fn main() {
    let json = fs::read_to_string("test_fixtures/library_response_debug.json").unwrap();
    println!("JSON length: {}", json.len());
    
    match serde_json::from_str::<LibraryResponse>(&json) {
        Ok(resp) => println!("Success! Got {} items", resp.items.len()),
        Err(e) => {
            println!("Error: {}", e);
            println!("Error details: {:?}", e);
            
            // Try to see what's at the error position
            if let Some(col) = e.column() {
                let start = col.saturating_sub(100);
                let end = (col + 100).min(json.len());
                println!("\nContext around column {}:", col);
                println!("{}", &json[start..end]);
            }
        }
    }
}
