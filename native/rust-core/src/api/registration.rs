// LibriSync - Audible Library Sync for Mobile
// Copyright (C) 2025 Henning Berge
//
// This program is a Rust port of Libation (https://github.com/rmcrackan/Libation)
// Original work Copyright (C) Libation contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.


//! Device registration response parsing
//!
//! This module handles the complete registration response from Audible's
//! `/auth/register` endpoint, extracting all tokens and data needed for
//! authenticated API access.
//!
//! # Registration Response Structure
//!
//! The registration response contains:
//! - **Bearer tokens**: OAuth access + refresh tokens
//! - **MAC DMS tokens**: Device private key + ADP token (encrypted)
//! - **Website cookies**: Session cookies for web access
//! - **Store auth cookie**: Store authentication cookie
//! - **Device info**: Serial, type, name
//! - **Customer info**: User ID, name, home region
//!
//! # Reference
//! Based on mkb79/Audible Python library registration response parsing
//! and Libation's Mkb79Auth.cs

use crate::error::{LibationError, Result};
use crate::api::auth::{AccessToken, Identity, Locale, CustomerInfo};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Registration Response Structures
// ============================================================================

/// Complete registration response from Audible
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationResponse {
    pub response: ResponseWrapper,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseWrapper {
    pub success: SuccessResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub tokens: Tokens,
    pub extensions: Extensions,
    pub customer_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub bearer: BearerTokens,
    pub mac_dms: MacDmsTokens,
    pub website_cookies: Vec<WebsiteCookie>,
    pub store_authentication_cookie: StoreAuthCookie,
    pub website_cookies_ttl: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BearerTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacDmsTokens {
    pub device_private_key: String,
    pub adp_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebsiteCookie {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Value")]
    pub value: String,
    #[serde(rename = "Domain")]
    pub domain: String,
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "Expires")]
    pub expires: String,
    #[serde(rename = "Secure")]
    pub secure: String,
    #[serde(rename = "HttpOnly")]
    pub http_only: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreAuthCookie {
    pub cookie: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extensions {
    pub device_info: DeviceInfo,
    pub customer_info: CustomerInfoExtension,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub device_name: String,
    pub device_serial_number: String,
    pub device_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerInfoExtension {
    pub account_pool: String,
    pub user_id: String,
    pub home_region: String,
    pub name: String,
    pub given_name: String,
}

// ============================================================================
// Registration Data - Extracted and Ready for Use
// ============================================================================

/// Extracted registration data ready for Identity creation
#[derive(Debug, Clone)]
pub struct RegistrationData {
    pub access_token: AccessToken,
    pub refresh_token: String,
    pub device_private_key: String,
    pub adp_token: String,
    pub cookies: HashMap<String, String>,
    pub device_serial_number: String,
    pub device_type: String,
    pub device_name: String,
    pub amazon_account_id: String,
    pub store_authentication_cookie: String,
    pub customer_info: CustomerInfo,
}

// ============================================================================
// Parsing Functions
// ============================================================================

impl RegistrationResponse {
    /// Parse a registration response from JSON
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| LibationError::InvalidApiResponse {
            message: format!("Failed to parse registration response: {}", e),
            response_body: Some(json.to_string()),
        })
    }

    /// Extract registration data for Identity creation
    pub fn extract_data(&self, locale: Locale) -> Result<RegistrationData> {
        let success = &self.response.success;
        let tokens = &success.tokens;
        let extensions = &success.extensions;

        // Parse bearer tokens
        let expires_in = tokens.bearer.expires_in.parse::<i64>()
            .map_err(|_| LibationError::InvalidApiResponse {
                message: format!("Invalid expires_in value: {}", tokens.bearer.expires_in),
                response_body: None,
            })?;

        let access_token = AccessToken {
            token: tokens.bearer.access_token.clone(),
            expires_at: Utc::now() + Duration::seconds(expires_in),
        };

        // Convert website cookies to HashMap
        let mut cookies = HashMap::new();
        for cookie in &tokens.website_cookies {
            cookies.insert(cookie.name.clone(), cookie.value.clone());
        }

        // Build CustomerInfo
        let customer_info = CustomerInfo {
            account_pool: extensions.customer_info.account_pool.clone(),
            user_id: extensions.customer_info.user_id.clone(),
            home_region: extensions.customer_info.home_region.clone(),
            name: extensions.customer_info.name.clone(),
            given_name: extensions.customer_info.given_name.clone(),
        };

        Ok(RegistrationData {
            access_token,
            refresh_token: tokens.bearer.refresh_token.clone(),
            device_private_key: tokens.mac_dms.device_private_key.clone(),
            adp_token: tokens.mac_dms.adp_token.clone(),
            cookies,
            device_serial_number: extensions.device_info.device_serial_number.clone(),
            device_type: extensions.device_info.device_type.clone(),
            device_name: extensions.device_info.device_name.clone(),
            amazon_account_id: success.customer_id.clone(),
            store_authentication_cookie: tokens.store_authentication_cookie.cookie.clone(),
            customer_info,
        })
    }

    /// Create an Identity from registration data
    pub fn to_identity(&self, locale: Locale) -> Result<Identity> {
        let data = self.extract_data(locale.clone())?;

        Ok(Identity {
            access_token: data.access_token,
            refresh_token: data.refresh_token,
            device_private_key: data.device_private_key,
            adp_token: data.adp_token,
            cookies: data.cookies,
            device_serial_number: data.device_serial_number,
            device_type: data.device_type,
            device_name: data.device_name,
            amazon_account_id: data.amazon_account_id,
            store_authentication_cookie: data.store_authentication_cookie,
            locale,
            customer_info: data.customer_info,
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_FIXTURE: &str = include_str!("../../test_fixtures/registration_response.json");

    #[test]
    fn test_parse_registration_response() {
        let response = RegistrationResponse::from_json(TEST_FIXTURE).unwrap();

        // Verify basic structure
        assert_eq!(response.request_id, "8e570a18-8232-4df0-a212-c0d21860fcd5");
        assert_eq!(response.response.success.customer_id, "amzn1.account.AGMGLSGIFYVALF2MEO4F3JJQRLSA");
    }

    #[test]
    fn test_extract_bearer_tokens() {
        let response = RegistrationResponse::from_json(TEST_FIXTURE).unwrap();
        let tokens = &response.response.success.tokens.bearer;

        assert!(tokens.access_token.starts_with("Atna|"));
        assert!(tokens.refresh_token.starts_with("Atnr|"));
        // Token expiration can vary slightly (3599-3601 seconds)
        let expires_in: i64 = tokens.expires_in.parse().unwrap();
        assert!(expires_in >= 3599 && expires_in <= 3601, "expires_in should be ~3600, got {}", expires_in);
    }

    #[test]
    fn test_extract_mac_dms_tokens() {
        let response = RegistrationResponse::from_json(TEST_FIXTURE).unwrap();
        let mac_dms = &response.response.success.tokens.mac_dms;

        // Device private key should be RSA private key (PEM format)
        assert!(mac_dms.device_private_key.starts_with("MII"));
        assert!(mac_dms.device_private_key.len() > 1000);

        // ADP token should be encrypted format
        assert!(mac_dms.adp_token.contains("{enc:"));
        assert!(mac_dms.adp_token.contains("{key:"));
        assert!(mac_dms.adp_token.contains("{iv:"));
    }

    #[test]
    fn test_extract_website_cookies() {
        let response = RegistrationResponse::from_json(TEST_FIXTURE).unwrap();
        let cookies = &response.response.success.tokens.website_cookies;

        // Should have 5 cookies
        assert_eq!(cookies.len(), 5);

        // Verify key cookies
        let cookie_names: Vec<_> = cookies.iter().map(|c| c.name.as_str()).collect();
        assert!(cookie_names.contains(&"session-id"));
        assert!(cookie_names.contains(&"ubid-main"));
        assert!(cookie_names.contains(&"x-main"));
        assert!(cookie_names.contains(&"at-main"));
        assert!(cookie_names.contains(&"sess-at-main"));

        // Verify a specific cookie
        let session_id = cookies.iter().find(|c| c.name == "session-id").unwrap();
        assert_eq!(session_id.value, "139-0923163-3019105");
        assert_eq!(session_id.domain, ".amazon.com");
    }

    #[test]
    fn test_extract_device_info() {
        let response = RegistrationResponse::from_json(TEST_FIXTURE).unwrap();
        let device_info = &response.response.success.extensions.device_info;

        assert_eq!(device_info.device_name, "Henning's 7th Android");
        assert_eq!(device_info.device_serial_number, "B45EF975C33A7B7E8DAF4D96E39B8040");
        assert_eq!(device_info.device_type, "A10KISP2GWF0E4");
    }

    #[test]
    fn test_extract_customer_info() {
        let response = RegistrationResponse::from_json(TEST_FIXTURE).unwrap();
        let customer_info = &response.response.success.extensions.customer_info;

        assert_eq!(customer_info.account_pool, "Amazon");
        assert_eq!(customer_info.user_id, "amzn1.account.AGMGLSGIFYVALF2MEO4F3JJQRLSA");
        assert_eq!(customer_info.home_region, "NA");
        assert_eq!(customer_info.name, "Henning Berge");
        assert_eq!(customer_info.given_name, "Henning");
    }

    #[test]
    fn test_extract_registration_data() {
        let response = RegistrationResponse::from_json(TEST_FIXTURE).unwrap();
        let locale = Locale::us();
        let data = response.extract_data(locale).unwrap();

        // Verify access token
        assert!(data.access_token.token.starts_with("Atna|"));
        assert!(data.access_token.expires_at > Utc::now());

        // Verify refresh token
        assert!(data.refresh_token.starts_with("Atnr|"));

        // Verify device credentials
        assert!(data.device_private_key.starts_with("MII"));
        assert!(data.adp_token.contains("{enc:"));

        // Verify device info
        assert_eq!(data.device_serial_number, "B45EF975C33A7B7E8DAF4D96E39B8040");
        assert_eq!(data.device_type, "A10KISP2GWF0E4");
        assert_eq!(data.device_name, "Henning's 7th Android");

        // Verify customer info
        assert_eq!(data.amazon_account_id, "amzn1.account.AGMGLSGIFYVALF2MEO4F3JJQRLSA");
        assert_eq!(data.customer_info.name, "Henning Berge");
        assert_eq!(data.customer_info.home_region, "NA");

        // Verify cookies
        assert!(data.cookies.contains_key("session-id"));
        assert!(data.cookies.contains_key("at-main"));

        // Verify store auth cookie
        assert!(data.store_authentication_cookie.len() > 50);
    }

    #[test]
    fn test_to_identity() {
        let response = RegistrationResponse::from_json(TEST_FIXTURE).unwrap();
        let locale = Locale::us();
        let identity = response.to_identity(locale.clone()).unwrap();

        // Verify Identity fields
        assert!(identity.access_token.token.starts_with("Atna|"));
        assert!(identity.refresh_token.starts_with("Atnr|"));
        assert_eq!(identity.device_serial_number, "B45EF975C33A7B7E8DAF4D96E39B8040");
        assert_eq!(identity.device_type, "A10KISP2GWF0E4");
        assert_eq!(identity.amazon_account_id, "amzn1.account.AGMGLSGIFYVALF2MEO4F3JJQRLSA");
        assert_eq!(identity.locale, locale);
        assert_eq!(identity.customer_info.name, "Henning Berge");
    }

    #[test]
    fn test_cookie_expiration_parsing() {
        let response = RegistrationResponse::from_json(TEST_FIXTURE).unwrap();
        let cookies = &response.response.success.tokens.website_cookies;

        // Check session cookie expires far in future
        let session_id = cookies.iter().find(|c| c.name == "session-id").unwrap();
        assert_eq!(session_id.expires, "2 Oct 2045 16:26:51 GMT");

        // Check at-main expires in 1 year
        let at_main = cookies.iter().find(|c| c.name == "at-main").unwrap();
        assert_eq!(at_main.expires, "7 Oct 2026 16:26:51 GMT");
    }

    #[test]
    fn test_secure_cookies() {
        let response = RegistrationResponse::from_json(TEST_FIXTURE).unwrap();
        let cookies = &response.response.success.tokens.website_cookies;

        // All cookies should be secure
        for cookie in cookies {
            assert_eq!(cookie.secure, "true");
        }

        // Check HttpOnly flags
        let at_main = cookies.iter().find(|c| c.name == "at-main").unwrap();
        assert_eq!(at_main.http_only, "true");

        let session_id = cookies.iter().find(|c| c.name == "session-id").unwrap();
        assert_eq!(session_id.http_only, "false");
    }

    #[test]
    fn test_token_types() {
        let response = RegistrationResponse::from_json(TEST_FIXTURE).unwrap();
        let tokens = &response.response.success.tokens;

        // Bearer tokens
        assert!(tokens.bearer.access_token.starts_with("Atna|"));
        assert!(tokens.bearer.refresh_token.starts_with("Atnr|"));

        // MAC DMS tokens - device_private_key is base64-encoded (no PEM headers)
        assert!(tokens.mac_dms.device_private_key.starts_with("MII"));
        assert!(tokens.mac_dms.device_private_key.len() > 1000);

        // Store auth cookie is base64-encoded
        assert!(tokens.store_authentication_cookie.cookie.len() > 50);
    }

    #[test]
    fn test_customer_id_matches_user_id() {
        let response = RegistrationResponse::from_json(TEST_FIXTURE).unwrap();
        let customer_id = &response.response.success.customer_id;
        let user_id = &response.response.success.extensions.customer_info.user_id;

        // These should be the same
        assert_eq!(customer_id, user_id);
        assert_eq!(customer_id, "amzn1.account.AGMGLSGIFYVALF2MEO4F3JJQRLSA");
    }
}
