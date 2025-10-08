use serde_json;
use std::fs;

fn main() {
    let json = fs::read_to_string("test_fixtures/library_response_debug.json").unwrap();
    
    // Try parsing as generic Value first
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    
    // Check second item (The Martian)
    let items = value["items"].as_array().unwrap();
    let second = &items[1];
    
    // Find all array fields
    for (key, val) in second.as_object().unwrap() {
        if val.is_array() {
            println!("{}: array with {} items", key, val.as_array().unwrap().len());
        } else if val.is_null() && key.contains("review") {
            println!("{}: null (might expect array)", key);
        }
    }
}
