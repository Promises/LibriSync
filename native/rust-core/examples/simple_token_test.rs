//! Simple token exchange test using /auth/token instead of /auth/register

use std::collections::HashMap;

#[tokio::main]
async fn main() {
    let auth_code = "ANuhgzTLXnxSgnGjpAOhTieN";
    let device_serial = "test123";
    let code_verifier = "test_verifier";

    let client_id = format!("device:{}#A2CZJZGLK2JJVM", device_serial);

    let mut form_data = HashMap::new();
    form_data.insert("grant_type", "authorization_code");
    form_data.insert("code", auth_code);
    form_data.insert("client_id", &client_id);
    form_data.insert("code_verifier", code_verifier);
    form_data.insert("redirect_uri", "https://www.amazon.com/ap/maplanding");

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.amazon.com/auth/token")
        .form(&form_data)
        .send()
        .await
        .unwrap();

    println!("Status: {}", response.status());
    println!("Response: {}", response.text().await.unwrap());
}
