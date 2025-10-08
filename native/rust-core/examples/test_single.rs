use rust_core::api::library::LibraryResponse;
use std::fs;

fn main() {
    for i in 1..=5 {
        let filename = format!("test_fixtures/single_item_{}.json", i);
        let json = fs::read_to_string(&filename).unwrap();
        
        match serde_json::from_str::<LibraryResponse>(&json) {
            Ok(resp) => println!("✓ Book {} parsed successfully: {}", i, resp.items[0].title),
            Err(e) => {
                println!("✗ Book {} failed: {}", i, e);
                println!("  File: {}", filename);
                break;
            }
        }
    }
}
