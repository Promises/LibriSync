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

        // Signed, not bearer: this endpoint answers a bearer token with
        // "Request could not be authenticated". Reference: AudibleApi/Api.Customer.cs:72
        // (AdHocAuthenticatedGetAsync).
        let response: serde_json::Value =
            self.get_signed("/1.0/customer/information", &query).await?;

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
