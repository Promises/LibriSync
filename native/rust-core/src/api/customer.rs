//! Customer information API
//!
//! This module implements customer/account information retrieval from Audible API.
//!
//! # Reference C# Sources
//! - **`AudibleApi/Api.Customer.cs`** - Customer information endpoint
//!
//! # API Endpoint
//! `GET https://api.audible.{domain}/1.0/customer/information`

use crate::error::Result;
use crate::api::client::AudibleClient;
use serde::{Deserialize, Serialize};

/// Customer information response from Audible API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerInformation {
    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub given_name: Option<String>,

    #[serde(default)]
    pub email: Option<String>,
}

impl AudibleClient {
    /// Get customer information
    ///
    /// # Reference
    /// Based on `Api.GetCustomerInformationAsync()` - AudibleApi/Api.Customer.cs:67-75
    ///
    /// Makes a GET request to `/1.0/customer/information` endpoint
    ///
    /// # Returns
    /// Customer information (name, email if available)
    ///
    /// # Errors
    /// Returns error if API call fails or response cannot be parsed
    pub async fn get_customer_information(&self) -> Result<CustomerInformation> {
        #[derive(Serialize)]
        struct CustomerQuery {
            response_groups: String,
        }

        // Request with minimal response groups
        let query = CustomerQuery {
            response_groups: "migration_details".to_string(),
        };

        let response: serde_json::Value = self.get_with_query("/1.0/customer/information", &query).await?;

        // Try to extract name from various possible locations in response
        let name = response.get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let given_name = response.get("given_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let email = response.get("email")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(CustomerInformation {
            name,
            given_name,
            email,
        })
    }
}
