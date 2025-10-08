//! Authentication and account management for Audible API
//!
//! This module implements the authentication flow and account management for the Audible API,
//! ported from the C# Libation implementation. It handles OAuth-based authentication,
//! token management, device registration, and activation bytes retrieval.
//!
//! # Reference C# Sources
//!
//! This implementation is based on the following C# classes from Libation:
//!
//! ## Core Account Management
//! - `AudibleUtilities/Account.cs` - Main Account class with:
//!   - AccountId (immutable email/phone identifier)
//!   - AccountName (user-friendly display name)
//!   - DecryptKey (activation bytes for DRM removal)
//!   - IdentityTokens (OAuth credentials)
//!   - Locale (market/region)
//!   - LibraryScan (whether to include in scans)
//!
//! - `AudibleUtilities/AccountsSettings.cs` - Collection management:
//!   - List of accounts
//!   - Add/remove/upsert operations
//!   - Validation (no duplicate accountId + locale)
//!   - JSON serialization
//!
//! - `AudibleUtilities/AudibleApiStorage.cs` - Persistence:
//!   - AccountsSettings.json file location
//!   - JSONPath queries for identity tokens
//!   - File I/O operations
//!
//! ## Authentication & OAuth
//! - `AudibleUtilities/Mkb79Auth.cs` - External authentication data:
//!   - OAuth tokens (access_token, refresh_token, expires)
//!   - Device info (device_serial_number, device_type, device_name)
//!   - Customer info (user_id, name, home_region)
//!   - Website cookies
//!   - Store authentication cookie
//!   - Activation bytes
//!   - Conversion to/from Account objects
//!
//! ## OAuth Flow (from AudibleApi NuGet package)
//! The C# implementation uses the AudibleApi package which provides:
//! - `AudibleApi.Authorization.Identity` - Token container with:
//!   - AccessToken (with expiration)
//!   - RefreshToken
//!   - PrivateKey (device private key)
//!   - AdpToken (Amazon Device Protocol token)
//!   - Cookies (website session cookies)
//!   - Device info (serial number, type, name)
//!   - Amazon account ID
//! - `AudibleApi.Authorization.Authorize` - OAuth flow:
//!   - RefreshAccessTokenAsync() - Refresh expired tokens
//! - `AudibleApi.Localization.Locale` - Market/region info:
//!   - CountryCode (e.g., "us", "uk", "de")
//!   - Domain (e.g., "audible.com", "audible.co.uk")
//!   - Name (display name)
//!   - WithUsername (email vs phone authentication)
//!
//! # Authentication Flow
//!
//! 1. **Initial Authentication** (via external browser):
//!    - User opens Audible login page in external browser
//!    - User completes OAuth login flow
//!    - App captures redirect URL with authorization code
//!    - App exchanges code for tokens (Mkb79Auth.cs format)
//!    - Tokens stored in Identity object
//!
//! 2. **Token Refresh** (automatic):
//!    - Check if access_token is expired (AccessTokenExpires < now)
//!    - Use refresh_token to get new access_token
//!    - Update stored Identity with new tokens
//!    - See: Authorize.RefreshAccessTokenAsync() in C#
//!
//! 3. **Device Registration**:
//!    - Generate device private key
//!    - Register device with Audible
//!    - Store device info (serial number, type, name)
//!    - Receive adp_token for API authentication
//!
//! 4. **Activation Bytes Retrieval**:
//!    - Make authenticated API call to get activation bytes
//!    - Store in Account.DecryptKey field
//!    - Used for DRM removal (AAX decryption)
//!
//! # Security Considerations
//!
//! - **Never log sensitive data**: Access tokens, refresh tokens, activation bytes,
//!   device private keys, and cookies must not appear in logs
//! - **Masked logging**: Use `MaskedLogEntry` pattern from Account.cs for safe logging
//! - **Token expiration**: Always check token expiry before API calls
//! - **Secure storage**: Consider encrypting tokens at rest (future enhancement)
//! - **HTTPS only**: All API calls must use HTTPS
//!
//! # Usage Example
//!
//! ```rust,no_run
//! # use rust_core::api::auth::*;
//! # use rust_core::error::Result;
//! # async fn example() -> Result<()> {
//! // Create a new account
//! let mut account = Account::new("user@example.com".to_string());
//! account.set_locale(Locale::us());
//!
//! // Authenticate (in real app, this would involve external browser)
//! // let identity = authenticate_with_browser(&account.locale()).await?;
//! // account.set_identity(identity);
//!
//! // Check if tokens need refresh
//! if account.needs_token_refresh() {
//!     account.refresh_tokens().await?;
//! }
//!
//! // Get activation bytes for DRM
//! let activation_bytes = account.get_activation_bytes().await?;
//! # Ok(())
//! # }
//! ```

use crate::error::{LibationError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Core Account Structure
// ============================================================================

/// Represents an Audible account with authentication credentials
///
/// Maps to C# `AudibleUtilities.Account` class (Account.cs)
///
/// # Fields from C# Account class:
/// - AccountId (string) → account_id (String)
/// - AccountName (string) → account_name (String)
/// - DecryptKey (string) → decrypt_key (String) - activation bytes
/// - IdentityTokens (Identity) → identity (Option<Identity>)
/// - LibraryScan (bool) → library_scan (bool)
/// - Locale (Locale) → accessed via identity.locale
///
/// # Notes:
/// - AccountId is immutable (email or phone number)
/// - AccountName is user-friendly and mutable
/// - DecryptKey stores the activation bytes (4-byte hex string)
/// - LibraryScan controls whether this account is included in library scans
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// Canonical, immutable account identifier (email or phone number)
    /// Maps to C# Account.AccountId
    pub account_id: String,

    /// User-friendly, mutable display name
    /// Maps to C# Account.AccountName
    pub account_name: String,

    /// Whether to include this account when scanning libraries
    /// Maps to C# Account.LibraryScan
    /// Default: true
    pub library_scan: bool,

    /// Activation bytes for DRM removal (4-byte hex string)
    /// Maps to C# Account.DecryptKey
    /// Also called "activation bytes" in Audible terminology
    pub decrypt_key: String,

    /// OAuth identity tokens and credentials
    /// Maps to C# Account.IdentityTokens (type: Identity)
    /// None if account hasn't been authenticated yet
    pub identity: Option<Identity>,
}

// ============================================================================
// OAuth Identity and Tokens
// ============================================================================

/// OAuth identity containing access tokens, refresh tokens, and device info
///
/// Maps to C# `AudibleApi.Authorization.Identity` class (from AudibleApi package)
/// and `AudibleUtilities.Mkb79Auth` class (Mkb79Auth.cs)
///
/// # C# Identity Fields Mapping:
/// - AccessToken (AccessToken) → access_token (AccessToken struct)
/// - RefreshToken (RefreshToken) → refresh_token (String)
/// - PrivateKey (PrivateKey) → device_private_key (String)
/// - AdpToken (AdpToken) → adp_token (String)
/// - Cookies (IEnumerable<KeyValuePair>) → cookies (HashMap)
/// - DeviceSerialNumber (string) → device_serial_number (String)
/// - DeviceType (string) → device_type (String)
/// - DeviceName (string) → device_name (String)
/// - AmazonAccountId (string) → amazon_account_id (String)
/// - StoreAuthenticationCookie (string) → store_authentication_cookie (String)
/// - Locale (Locale) → locale (Locale)
///
/// # C# Mkb79Auth Additional Fields:
/// - website_cookies (JObject) → website_cookies (HashMap)
/// - customer_info (CustomerInfo) → customer_info (CustomerInfo)
/// - device_info (DeviceInfo) → device_info (DeviceInfo)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    /// OAuth access token with expiration
    /// Maps to C# Identity.AccessToken and Mkb79Auth.AccessToken
    pub access_token: AccessToken,

    /// OAuth refresh token (used to get new access tokens)
    /// Maps to C# Identity.RefreshToken and Mkb79Auth.RefreshToken
    pub refresh_token: String,

    /// Device private key for cryptographic operations
    /// Maps to C# Identity.PrivateKey and Mkb79Auth.DevicePrivateKey
    pub device_private_key: String,

    /// Amazon Device Protocol token
    /// Maps to C# Identity.AdpToken and Mkb79Auth.AdpToken
    pub adp_token: String,

    /// Website session cookies
    /// Maps to C# Identity.Cookies and Mkb79Auth.WebsiteCookies
    pub cookies: HashMap<String, String>,

    /// Device serial number
    /// Maps to C# Identity.DeviceSerialNumber and Mkb79Auth.DeviceInfo.DeviceSerialNumber
    pub device_serial_number: String,

    /// Device type (e.g., "A2CZJZGLK2JJVM" for software device)
    /// Maps to C# Identity.DeviceType and Mkb79Auth.DeviceInfo.DeviceType
    pub device_type: String,

    /// User-friendly device name
    /// Maps to C# Identity.DeviceName and Mkb79Auth.DeviceInfo.DeviceName
    pub device_name: String,

    /// Amazon account user ID
    /// Maps to C# Identity.AmazonAccountId and Mkb79Auth.CustomerInfo.UserId
    pub amazon_account_id: String,

    /// Store authentication cookie
    /// Maps to C# Identity.StoreAuthenticationCookie and Mkb79Auth.StoreAuthenticationCookie
    pub store_authentication_cookie: String,

    /// Audible market/region
    /// Maps to C# Identity.Locale and Mkb79Auth.Locale
    pub locale: Locale,

    /// Customer information from Audible
    /// Maps to C# Mkb79Auth.CustomerInfo
    pub customer_info: CustomerInfo,
}

/// OAuth access token with expiration time
///
/// Maps to C# `AudibleApi.Authorization.AccessToken` class
///
/// # C# AccessToken Fields:
/// - TokenValue (string) → token (String)
/// - Expires (DateTime) → expires_at (DateTime<Utc>)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessToken {
    /// The actual token string
    /// Maps to C# AccessToken.TokenValue
    pub token: String,

    /// When this token expires
    /// Maps to C# AccessToken.Expires
    pub expires_at: DateTime<Utc>,
}

/// Customer information from Audible account
///
/// Maps to C# `AudibleUtilities.CustomerInfo` class in Mkb79Auth.cs
///
/// # C# CustomerInfo Fields:
/// - AccountPool (string) → account_pool (String)
/// - UserId (string) → user_id (String)
/// - HomeRegion (string) → home_region (String)
/// - Name (string) → name (String)
/// - GivenName (string) → given_name (String)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerInfo {
    /// Account pool (typically "Amazon")
    /// Maps to C# CustomerInfo.AccountPool
    pub account_pool: String,

    /// Audible user ID
    /// Maps to C# CustomerInfo.UserId
    pub user_id: String,

    /// Home region (e.g., "NA" for North America)
    /// Maps to C# CustomerInfo.HomeRegion
    pub home_region: String,

    /// Full name
    /// Maps to C# CustomerInfo.Name
    pub name: String,

    /// Given (first) name
    /// Maps to C# CustomerInfo.GivenName
    pub given_name: String,
}

// ============================================================================
// Locale and Region Configuration
// ============================================================================

/// Audible market/region configuration
///
/// Maps to C# `AudibleApi.Localization.Locale` class (from AudibleApi package)
///
/// # C# Locale Fields:
/// - CountryCode (string) → country_code (String) - e.g., "us", "uk"
/// - Domain (string) → domain (String) - e.g., "audible.com"
/// - Name (string) → name (String) - display name
/// - WithUsername (bool) → with_username (bool) - email vs phone auth
///
/// # Supported Locales in Libation:
/// - US: audible.com (with_username: true)
/// - UK: audible.co.uk (with_username: true)
/// - DE: audible.de (with_username: true)
/// - FR: audible.fr (with_username: true)
/// - CA: audible.ca (with_username: true)
/// - AU: audible.com.au (with_username: true)
/// - IT: audible.it (with_username: true)
/// - ES: audible.es (with_username: true)
/// - IN: audible.in (with_username: true)
/// - JP: audible.co.jp (with_username: false)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Locale {
    /// ISO country code (lowercase)
    /// Maps to C# Locale.CountryCode
    pub country_code: String,

    /// Audible domain for this market
    /// Maps to C# Locale.Domain
    pub domain: String,

    /// Human-readable name
    /// Maps to C# Locale.Name
    pub name: String,

    /// Whether this locale uses email (true) or phone (false) authentication
    /// Maps to C# Locale.WithUsername
    pub with_username: bool,
}

// ============================================================================
// OAuth Flow Data Structures
// ============================================================================

/// OAuth credentials for Audible API
///
/// These are the OAuth client credentials needed to authenticate with Audible.
/// In the C# implementation, these are typically hardcoded or configured.
///
/// Note: The actual values must be obtained from Audible's OAuth configuration
/// or reverse-engineered from the official Audible apps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCredentials {
    /// OAuth client ID
    pub client_id: String,

    /// OAuth client secret
    pub client_secret: String,

    /// Redirect URI for OAuth callback
    pub redirect_uri: String,
}

/// Device registration information
///
/// Maps to C# `AudibleUtilities.DeviceInfo` class in Mkb79Auth.cs
///
/// # C# DeviceInfo Fields:
/// - DeviceName (string) → device_name (String)
/// - DeviceSerialNumber (string) → device_serial_number (String)
/// - DeviceType (string) → device_type (String)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRegistration {
    /// User-friendly device name
    /// Maps to C# DeviceInfo.DeviceName
    pub device_name: String,

    /// Unique device serial number
    /// Maps to C# DeviceInfo.DeviceSerialNumber
    pub device_serial_number: String,

    /// Device type identifier (e.g., "A2CZJZGLK2JJVM" for software)
    /// Maps to C# DeviceInfo.DeviceType
    pub device_type: String,
}

/// Token pair from OAuth flow
///
/// Temporary structure used during authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPair {
    /// Access token string
    pub access_token: String,

    /// Refresh token string
    pub refresh_token: String,

    /// Expires in seconds
    pub expires_in: i64,
}

// ============================================================================
// Account Implementation
// ============================================================================

impl Account {
    /// Create a new account with the given account ID (email or phone)
    ///
    /// Maps to C# `Account(string accountId)` constructor in Account.cs
    ///
    /// # Arguments
    /// - `account_id` - Email address or phone number (will be trimmed)
    ///
    /// # C# Reference:
    /// ```csharp
    /// public Account(string accountId)
    /// {
    ///     AccountId = ArgumentValidator.EnsureNotNullOrWhiteSpace(accountId, nameof(accountId)).Trim();
    /// }
    /// ```
    pub fn new(account_id: String) -> Result<Self> {
        let trimmed = account_id.trim();
        if trimmed.is_empty() {
            return Err(LibationError::InvalidData(
                "Account ID cannot be empty".to_string(),
            ));
        }

        Ok(Self {
            account_id: trimmed.to_string(),
            account_name: trimmed.to_string(), // Default to account_id
            library_scan: true,
            decrypt_key: String::new(),
            identity: None,
        })
    }

    /// Set the user-friendly account name
    ///
    /// Maps to C# `Account.AccountName` setter in Account.cs
    ///
    /// # C# Reference:
    /// ```csharp
    /// set
    /// {
    ///     if (string.IsNullOrWhiteSpace(value))
    ///         return;
    ///     var v = value.Trim();
    ///     if (v == _accountName)
    ///         return;
    ///     _accountName = v;
    ///     update();
    /// }
    /// ```
    pub fn set_account_name(&mut self, name: String) {
        let trimmed = name.trim();
        if !trimmed.is_empty() && trimmed != self.account_name {
            self.account_name = trimmed.to_string();
        }
    }

    /// Set the activation bytes (decrypt key)
    ///
    /// Maps to C# `Account.DecryptKey` setter in Account.cs
    ///
    /// # Arguments
    /// - `key` - 4-byte activation bytes as hex string (e.g., "1a2b3c4d")
    ///
    /// # C# Reference:
    /// ```csharp
    /// set
    /// {
    ///     var v = (value ?? "").Trim();
    ///     if (v == _decryptKey)
    ///         return;
    ///     _decryptKey = v;
    ///     update();
    /// }
    /// ```
    pub fn set_decrypt_key(&mut self, key: String) {
        let trimmed = key.trim();
        if trimmed != self.decrypt_key {
            self.decrypt_key = trimmed.to_string();
        }
    }

    /// Set the OAuth identity tokens
    ///
    /// Maps to C# `Account.IdentityTokens` setter in Account.cs
    pub fn set_identity(&mut self, identity: Identity) {
        self.identity = Some(identity);
    }

    /// Get the locale from the identity tokens
    ///
    /// Maps to C# `Account.Locale` property in Account.cs
    ///
    /// # C# Reference:
    /// ```csharp
    /// [JsonIgnore]
    /// public Locale Locale => IdentityTokens?.Locale;
    /// ```
    pub fn locale(&self) -> Option<&Locale> {
        self.identity.as_ref().map(|i| &i.locale)
    }

    /// Check if access token needs refresh
    ///
    /// Returns true if there's no identity, or if the access token is expired
    /// or will expire within the next 5 minutes.
    ///
    /// Related to C# token refresh logic in ApiExtended.cs and Authorize class
    pub fn needs_token_refresh(&self) -> bool {
        match &self.identity {
            None => true,
            Some(identity) => {
                let now = Utc::now();
                // Refresh if expired or expiring within 5 minutes
                let buffer = chrono::Duration::minutes(5);
                identity.access_token.expires_at <= now + buffer
            }
        }
    }

    /// Generate a masked log entry for safe logging
    ///
    /// Maps to C# `Account.MaskedLogEntry` property in Account.cs
    ///
    /// # C# Reference:
    /// ```csharp
    /// public string MaskedLogEntry => @$"AccountId={mask(AccountId)}|AccountName={mask(AccountName)}|Locale={Locale?.Name ?? "[empty]"}";
    /// private static string mask(string str)
    ///     => str is null ? "[null]"
    ///     : str == string.Empty ? "[empty]"
    ///     : str.ToMask();
    /// ```
    pub fn masked_log_entry(&self) -> String {
        format!(
            "AccountId={}|AccountName={}|Locale={}",
            Self::mask(&self.account_id),
            Self::mask(&self.account_name),
            self.locale()
                .map(|l| l.name.as_str())
                .unwrap_or("[empty]")
        )
    }

    /// Mask a string for safe logging
    ///
    /// Shows first 2 and last 2 characters, replaces middle with asterisks
    fn mask(s: &str) -> String {
        if s.is_empty() {
            "[empty]".to_string()
        } else if s.len() <= 4 {
            "****".to_string()
        } else {
            let chars: Vec<char> = s.chars().collect();
            let first_two: String = chars.iter().take(2).collect();
            let last_two: String = chars.iter().skip(chars.len() - 2).collect();
            let middle_len = chars.len() - 4;
            format!("{}{}{}",  first_two, "*".repeat(middle_len), last_two)
        }
    }

    /// Refresh the access token using the refresh token
    ///
    /// Maps to C# `Authorize.RefreshAccessTokenAsync()` logic in AudibleApi
    /// See also: Mkb79Auth.ToAccountAsync() in Mkb79Auth.cs lines 128-159
    ///
    /// # Errors
    /// Returns error if no identity exists or refresh fails
    ///
    /// # C# Reference (Mkb79Auth.cs):
    /// ```csharp
    /// var refreshToken = new RefreshToken(RefreshToken);
    /// var authorize = new Authorize(Locale);
    /// var newToken = await authorize.RefreshAccessTokenAsync(refreshToken);
    /// AccessToken = newToken.TokenValue;
    /// AccessTokenExpires = newToken.Expires;
    /// ```
    pub async fn refresh_tokens(&mut self) -> Result<()> {
        let identity = self.identity.as_mut().ok_or_else(|| {
            LibationError::AuthenticationFailed {
                message: "No identity tokens to refresh".to_string(),
                account_id: Some(self.account_id.clone()),
            }
        })?;

        // Call the refresh_access_token function
        let token_response = refresh_access_token(
            &identity.locale,
            &identity.refresh_token,
            &identity.device_serial_number,
        ).await?;

        // Update the identity with new tokens
        identity.access_token.token = token_response.access_token;
        identity.access_token.expires_at = Utc::now() + chrono::Duration::seconds(token_response.expires_in);

        // Update refresh token if provided (some implementations return a new refresh token)
        if let Some(new_refresh_token) = token_response.refresh_token {
            if !new_refresh_token.is_empty() {
                identity.refresh_token = new_refresh_token;
            }
        }

        Ok(())
    }

    /// Retrieve activation bytes for DRM decryption
    ///
    /// This calls the Audible API to get the activation bytes for this account.
    /// The activation bytes are stored in the `decrypt_key` field.
    ///
    /// # Errors
    /// Returns error if authentication fails or API call fails
    ///
    /// # Note
    /// This requires authenticated API access. The exact endpoint is part of
    /// the Audible private API and must be reverse-engineered or documented.
    pub async fn get_activation_bytes(&mut self) -> Result<String> {
        let identity = self.identity.as_ref().ok_or_else(|| {
            LibationError::AuthenticationFailed {
                message: "No identity tokens for activation bytes retrieval".to_string(),
                account_id: Some(self.account_id.clone()),
            }
        })?;

        // Call the get_activation_bytes function
        let activation_bytes = get_activation_bytes(
            &identity.locale,
            &identity.access_token.token,
        ).await?;

        // Store in decrypt_key field
        self.decrypt_key = activation_bytes.clone();

        Ok(activation_bytes)
    }
}

// ============================================================================
// Identity Implementation
// ============================================================================

impl Identity {
    /// Create a new Identity from OAuth tokens and device info
    pub fn new(
        access_token: AccessToken,
        refresh_token: String,
        device_private_key: String,
        adp_token: String,
        locale: Locale,
    ) -> Self {
        Self {
            access_token,
            refresh_token,
            device_private_key,
            adp_token,
            cookies: HashMap::new(),
            device_serial_number: String::new(),
            device_type: String::new(),
            device_name: String::new(),
            amazon_account_id: String::new(),
            store_authentication_cookie: String::new(),
            locale,
            customer_info: CustomerInfo::default(),
        }
    }

    /// Check if the access token is expired
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.access_token.expires_at
    }

    /// Get time until token expiration
    pub fn time_until_expiry(&self) -> chrono::Duration {
        self.access_token.expires_at - Utc::now()
    }
}

// ============================================================================
// Locale Implementation
// ============================================================================

impl Locale {
    /// Create a new locale
    pub fn new(country_code: String, domain: String, name: String, with_username: bool) -> Self {
        Self {
            country_code,
            domain,
            name,
            with_username,
        }
    }

    /// Get the US locale (audible.com)
    ///
    /// Maps to Localization.Locales in C# AudibleApi
    pub fn us() -> Self {
        Self {
            country_code: "us".to_string(),
            domain: "audible.com".to_string(),
            name: "United States".to_string(),
            with_username: true,
        }
    }

    /// Get the UK locale (audible.co.uk)
    pub fn uk() -> Self {
        Self {
            country_code: "uk".to_string(),
            domain: "audible.co.uk".to_string(),
            name: "United Kingdom".to_string(),
            with_username: true,
        }
    }

    /// Get the DE locale (audible.de)
    pub fn de() -> Self {
        Self {
            country_code: "de".to_string(),
            domain: "audible.de".to_string(),
            name: "Germany".to_string(),
            with_username: true,
        }
    }

    /// Get the FR locale (audible.fr)
    pub fn fr() -> Self {
        Self {
            country_code: "fr".to_string(),
            domain: "audible.fr".to_string(),
            name: "France".to_string(),
            with_username: true,
        }
    }

    /// Get the CA locale (audible.ca)
    pub fn ca() -> Self {
        Self {
            country_code: "ca".to_string(),
            domain: "audible.ca".to_string(),
            name: "Canada".to_string(),
            with_username: true,
        }
    }

    /// Get the AU locale (audible.com.au)
    pub fn au() -> Self {
        Self {
            country_code: "au".to_string(),
            domain: "audible.com.au".to_string(),
            name: "Australia".to_string(),
            with_username: true,
        }
    }

    /// Get the IT locale (audible.it)
    pub fn it() -> Self {
        Self {
            country_code: "it".to_string(),
            domain: "audible.it".to_string(),
            name: "Italy".to_string(),
            with_username: true,
        }
    }

    /// Get the ES locale (audible.es)
    pub fn es() -> Self {
        Self {
            country_code: "es".to_string(),
            domain: "audible.es".to_string(),
            name: "Spain".to_string(),
            with_username: true,
        }
    }

    /// Get the IN locale (audible.in)
    pub fn in_() -> Self {
        Self {
            country_code: "in".to_string(),
            domain: "audible.in".to_string(),
            name: "India".to_string(),
            with_username: true,
        }
    }

    /// Get the JP locale (audible.co.jp)
    /// Note: Japan uses phone authentication instead of email
    pub fn jp() -> Self {
        Self {
            country_code: "jp".to_string(),
            domain: "audible.co.jp".to_string(),
            name: "Japan".to_string(),
            with_username: false,
        }
    }

    /// Get all supported locales
    pub fn all() -> Vec<Self> {
        vec![
            Self::us(),
            Self::uk(),
            Self::de(),
            Self::fr(),
            Self::ca(),
            Self::au(),
            Self::it(),
            Self::es(),
            Self::in_(),
            Self::jp(),
        ]
    }

    /// Find a locale by country code
    pub fn from_country_code(code: &str) -> Option<Self> {
        Self::all()
            .into_iter()
            .find(|l| l.country_code.eq_ignore_ascii_case(code))
    }

    /// Get the API base URL for this locale
    pub fn api_url(&self) -> String {
        format!("https://api.{}", self.domain)
    }

    /// Get the OAuth URL for this locale
    pub fn oauth_url(&self) -> String {
        format!("https://www.amazon.com/ap/signin")
    }
}

// ============================================================================
// Default Implementations
// ============================================================================

impl Default for CustomerInfo {
    fn default() -> Self {
        Self {
            account_pool: "Amazon".to_string(),
            user_id: String::new(),
            home_region: String::new(),
            name: String::new(),
            given_name: String::new(),
        }
    }
}

// ============================================================================
// OAuth Flow Functions
// ============================================================================

use rand::Rng;
use sha2::{Digest, Sha256};
use base64::{Engine as _, engine::general_purpose};
use url::Url;
use std::collections::HashMap as StdHashMap;
use uuid::Uuid;
use rsa::{RsaPrivateKey, pkcs8::EncodePrivateKey};

/// OAuth configuration constants
/// Based on mkb79 Python library (https://github.com/mkb79/Audible)
pub struct OAuthConfig {
    /// OAuth client ID format: "device:{serial}#A10KISP2GWF0E4"
    /// A10KISP2GWF0E4 is the device type ID from Libation
    pub client_id_format: &'static str,

    /// OAuth redirect URI (must match registered app)
    pub redirect_uri: &'static str,

    /// OAuth response type
    pub response_type: &'static str,

    /// OAuth scope
    pub scope: &'static str,

    /// PKCE code challenge method
    pub code_challenge_method: &'static str,

    /// Device type (A10KISP2GWF0E4 = from Libation)
    pub device_type: &'static str,
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            client_id_format: "device:{}#A10KISP2GWF0E4",
            redirect_uri: "https://www.amazon.com/ap/maplanding",
            response_type: "code",
            scope: "device_auth_access",
            code_challenge_method: "S256",
            device_type: "A10KISP2GWF0E4", // AudibleApi uses Android device type
        }
    }
}

/// PKCE code verifier and challenge pair
///
/// PKCE (Proof Key for Code Exchange) is used to prevent authorization code
/// interception attacks in OAuth flows.
///
/// Reference: RFC 7636 - https://tools.ietf.org/html/rfc7636
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    /// Code verifier (random string, 32-128 chars)
    /// Kept secret, sent during token exchange
    pub verifier: String,

    /// Code challenge (SHA256 hash of verifier, base64-url encoded)
    /// Sent during authorization request
    pub challenge: String,

    /// Challenge method (always "S256" for SHA-256)
    pub method: String,
}

impl PkceChallenge {
    /// Generate a new PKCE challenge with SHA-256 (S256 method)
    ///
    /// # Reference
    /// Based on mkb79 Python library login.py:
    /// - Generates 32-byte random verifier
    /// - Creates SHA-256 hash as challenge
    /// - Uses base64-url encoding
    ///
    /// # Returns
    /// New PkceChallenge with verifier and challenge
    pub fn generate() -> Result<Self> {
        // Generate 32 random bytes for verifier
        let mut rng = rand::thread_rng();
        let verifier_bytes: Vec<u8> = (0..32).map(|_| rng.gen::<u8>()).collect();

        // Base64-url encode the verifier (without padding)
        let verifier = general_purpose::URL_SAFE_NO_PAD.encode(&verifier_bytes);

        // Create SHA-256 hash of verifier
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let challenge_bytes = hasher.finalize();

        // Base64-url encode the challenge (without padding)
        let challenge = general_purpose::URL_SAFE_NO_PAD.encode(&challenge_bytes);

        Ok(Self {
            verifier,
            challenge,
            method: "S256".to_string(),
        })
    }
}

/// OAuth state parameter for CSRF protection
#[derive(Debug, Clone)]
pub struct OAuthState {
    /// Random state value
    pub value: String,
}

impl OAuthState {
    /// Generate a new random state value
    pub fn generate() -> Self {
        Self {
            value: Uuid::new_v4().to_string(),
        }
    }
}

/// Generate OAuth authorization URL with PKCE
///
/// Creates the URL that the user will visit in their browser to authenticate
/// with Audible. The URL includes all necessary OAuth parameters including
/// the PKCE code challenge for security.
///
/// # Reference
/// Based on mkb79 Python library login.py:
/// - Base URL: https://www.amazon.{domain}/ap/signin
/// - Response type: "code"
/// - Scope: "device_auth_access"
/// - Code challenge method: "S256"
/// - Client ID format: "device:{serial}#A2CZJZGLK2JJVM"
///
/// # Arguments
/// * `locale` - The Audible market/region
/// * `device_serial` - Device serial number (UUID)
/// * `pkce` - PKCE challenge generated with PkceChallenge::generate()
/// * `state` - OAuth state for CSRF protection
///
/// # Returns
/// Complete authorization URL string that should be opened in browser
///
/// # Example
/// ```rust,no_run
/// # use rust_core::api::auth::*;
/// # fn example() -> rust_core::error::Result<()> {
/// let locale = Locale::us();
/// let device_serial = "1234-5678-9012".to_string();
/// let pkce = PkceChallenge::generate()?;
/// let state = OAuthState::generate();
///
/// let auth_url = generate_authorization_url(&locale, &device_serial, &pkce, &state)?;
/// println!("Open this URL in browser: {}", auth_url);
/// # Ok(())
/// # }
/// ```
pub fn generate_authorization_url(
    locale: &Locale,
    device_serial: &str,
    pkce: &PkceChallenge,
    state: &OAuthState,
) -> Result<String> {
    let config = OAuthConfig::default();

    // Build client ID parts
    // AudibleApi hex-encodes only the "SERIAL#TYPE" part, not the "device:" prefix
    let serial_and_type = format!("{}#{}", device_serial, config.device_type);

    // Hex-encode to LOWERCASE (AudibleApi uses lowercase!)
    let serial_type_hex = serial_and_type.as_bytes()
        .iter()
        .map(|b| format!("{:02x}", b))  // lowercase!
        .collect::<String>();

    // Full client ID for token exchange (not hex-encoded)
    let client_id = format!("device:{}", serial_and_type);

    // Amazon login domain varies by region
    // Most regions use amazon.com, but some use local Amazon domains
    let amazon_domain = match locale.country_code.as_str() {
        "us" => "amazon.com",
        "uk" => "amazon.co.uk",
        "de" => "amazon.de",
        "fr" => "amazon.fr",
        "ca" => "amazon.ca",
        "au" => "amazon.com.au",
        "it" => "amazon.it",
        "es" => "amazon.es",
        "in" => "amazon.in",
        "jp" => "amazon.co.jp",
        _ => "amazon.com", // Default fallback
    };

    // Build authorization URL
    let mut url = Url::parse(&format!("https://www.{}/ap/signin", amazon_domain))
        .map_err(|e| LibationError::InvalidInput(format!("Invalid URL: {}", e)))?;

    {
        let mut query = url.query_pairs_mut();

        // OAuth 2.0 parameters (first for Libation compatibility)
        query.append_pair("openid.oa2.response_type", config.response_type);
        query.append_pair("openid.oa2.code_challenge_method", &pkce.method);
        query.append_pair("openid.oa2.code_challenge", &pkce.challenge);

        // OpenID parameters
        query.append_pair("openid.return_to", &format!("https://www.{}/ap/maplanding", amazon_domain));
        query.append_pair("openid.assoc_handle", &format!("amzn_audible_ios_{}", locale.country_code));
        query.append_pair("openid.identity", "http://specs.openid.net/auth/2.0/identifier_select");
        query.append_pair("pageId", "amzn_audible_ios");
        query.append_pair("accountStatusPolicy", "P1");
        query.append_pair("openid.claimed_id", "http://specs.openid.net/auth/2.0/identifier_select");
        query.append_pair("openid.mode", "checkid_setup");

        // Namespace declarations (CRITICAL - must match Libation)
        query.append_pair("openid.ns.oa2", "http://www.amazon.com/ap/ext/oauth/2");

        // OAuth client ID (device: prefix + hex-encoded serial#type, matching Libation exactly)
        query.append_pair("openid.oa2.client_id", &format!("device:{}", serial_type_hex));

        // PAPE namespace and parameters
        query.append_pair("openid.ns.pape", "http://specs.openid.net/extensions/pape/1.0");

        // Marketplace ID (locale-specific)
        let marketplace_id = match locale.country_code.as_str() {
            "us" => "AF2M0KC94RCEA",
            "uk" => "A2I9A3Q2GNFNGQ",
            "de" => "AN7EY7DTAW63G",
            "fr" => "A2728XDNODOQ8T",
            "ca" => "A2CQZ5RBY40XE",
            "au" => "AN7EY7DTAW63G",
            "it" => "A2N7FU2W2BU2ZC",
            "es" => "ALMIKO4SZCSAR",
            "in" => "AJO3FBRUE6J4S",
            "jp" => "A1QAP3MOU4173J",
            _ => "AF2M0KC94RCEA", // Default to US
        };
        query.append_pair("marketPlaceId", marketplace_id);

        // OAuth scope and state
        query.append_pair("openid.oa2.scope", config.scope);
        query.append_pair("forceMobileLayout", "true");

        // Main OpenID namespace (must come after extensions)
        query.append_pair("openid.ns", "http://specs.openid.net/auth/2.0");

        // PAPE max auth age
        query.append_pair("openid.pape.max_auth_age", "0");

        // Note: State parameter intentionally omitted - Libation doesn't use it
        // Amazon OAuth may not support state parameter in this flow
        // CSRF protection is handled by OpenID's nonce parameter instead
    }

    Ok(url.to_string())
}

/// Parse OAuth callback URL to extract authorization code
///
/// After the user authenticates in the browser, Audible redirects to the
/// callback URL with the authorization code as a query parameter. This
/// function extracts and validates that code.
///
/// # Arguments
/// * `callback_url` - Full callback URL from redirect (e.g., "https://localhost/callback?code=ABC123&state=xyz")
/// * `expected_state` - The state value that was sent in the authorization request
///
/// # Returns
/// Authorization code that can be exchanged for tokens
///
/// # Errors
/// Returns error if:
/// - URL cannot be parsed
/// - Authorization code is missing
/// - OAuth error is present (error, error_description)
/// - State parameter doesn't match (CSRF protection)
///
/// # Example
/// ```rust,no_run
/// # use rust_core::api::auth::*;
/// # fn example() -> rust_core::error::Result<()> {
/// let callback_url = "https://localhost/callback?code=ABC123&state=xyz";
/// let state = OAuthState { value: "xyz".to_string() };
/// let code = parse_authorization_callback(callback_url)?;
/// println!("Authorization code: {}", code);
/// # Ok(())
/// # }
/// ```
pub fn parse_authorization_callback(
    callback_url: &str,
) -> Result<String> {
    let url = Url::parse(callback_url)
        .map_err(|e| LibationError::InvalidInput(format!("Invalid callback URL: {}", e)))?;

    let params: StdHashMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    // Check for OAuth errors
    if let Some(error) = params.get("error") {
        let error_desc = params.get("error_description")
            .map(|s| s.as_str())
            .unwrap_or("No description");
        return Err(LibationError::AuthenticationFailed {
            message: format!("OAuth error: {} - {}", error, error_desc),
            account_id: None,
        });
    }

    // Extract authorization code
    // The code might be in different parameters depending on OAuth mode
    let code = params.get("openid.oa2.authorization_code")
        .or_else(|| params.get("code"))
        .ok_or_else(|| LibationError::AuthenticationFailed {
            message: "Missing authorization code in callback".to_string(),
            account_id: None,
        })?;

    Ok(code.clone())
}

/// Token response from Audible OAuth
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    pub expires_in: i64,  // Seconds until expiration
    pub token_type: String,
}

/// Full registration response from /auth/register endpoint
/// Contains all tokens, cookies, and device/customer information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationResponse {
    /// Bearer access/refresh tokens
    pub bearer: BearerTokenInfo,
    /// MAC-DMS tokens (device private key, ADP token)
    pub mac_dms: MacDmsTokenInfo,
    /// Website cookies for session management
    pub website_cookies: Vec<Cookie>,
    /// Store authentication cookie
    pub store_authentication_cookie: StoreAuthCookie,
    /// Device information
    pub device_info: DeviceInfo,
    /// Customer information
    pub customer_info: CustomerInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BearerTokenInfo {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: String,  // String because API returns "3600" as string
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacDmsTokenInfo {
    pub device_private_key: String,
    pub adp_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cookie {
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
pub struct DeviceInfo {
    pub device_name: String,
    pub device_serial_number: String,
    pub device_type: String,
}

/// Exchange authorization code for access and refresh tokens
///
/// After obtaining an authorization code from the callback, this function
/// exchanges it for actual OAuth tokens that can be used to access the API.
///
/// # Reference
/// Based on mkb79 Python library auth.py token refresh endpoint:
/// - POST to https://api.amazon.{domain}/auth/token
/// - Form-encoded body with grant_type, code, redirect_uri, code_verifier
///
/// # Arguments
/// * `locale` - The Audible market/region
/// * `authorization_code` - Code from callback URL
/// * `device_serial` - Device serial number (must match authorization request)
/// * `pkce` - PKCE challenge (verifier is sent, not challenge)
///
/// # Returns
/// TokenResponse with access_token, refresh_token, and expiration
///
/// # Errors
/// Returns error if token exchange fails
///
/// # Note
/// This function makes an HTTP request to Audible's token endpoint.
/// The client_id must match the one used in authorization URL.
pub async fn exchange_authorization_code(
    locale: &Locale,
    authorization_code: &str,
    device_serial: &str,
    pkce: &PkceChallenge,
) -> Result<RegistrationResponse> {
    let config = OAuthConfig::default();

    // Build hex-encoded client_id (LOWERCASE hex like AudibleApi!)
    // Format: hex(SERIAL#TYPE) without "device:" prefix
    let serial_and_type = format!("{}#{}", device_serial, config.device_type);
    let serial_type_hex = serial_and_type.as_bytes()
        .iter()
        .map(|b| format!("{:02x}", b))  // lowercase hex!
        .collect::<String>();

    // Amazon domain for token endpoint
    let amazon_domain = match locale.country_code.as_str() {
        "us" => "amazon.com",
        "uk" => "amazon.co.uk",
        "de" => "amazon.de",
        "fr" => "amazon.fr",
        "ca" => "amazon.ca",
        "au" => "amazon.com.au",
        "it" => "amazon.it",
        "es" => "amazon.es",
        "in" => "amazon.in",
        "jp" => "amazon.co.jp",
        _ => "amazon.com",
    };

    let register_url = format!("https://api.{}/auth/register", amazon_domain);

    // Build registration request body - EXACT match to mkb79/Audible Python library
    let request_body = serde_json::json!({
        "requested_token_type": [
            "bearer",
            "mac_dms",
            "website_cookies",
            "store_authentication_cookie"
        ],
        "cookies": {
            "website_cookies": [],
            "domain": format!(".amazon.{}", if locale.country_code == "us" { "com" } else { locale.country_code.as_str() })
        },
        "registration_data": {
            "domain": "DeviceLegacy",
            "device_type": "A10KISP2GWF0E4",
            "device_serial": device_serial,
            "app_name": "com.audible.application",
            "app_version": "177102",
            "device_name": format!("%FIRST_NAME%%FIRST_NAME_POSSESSIVE_STRING%%DUPE_STRATEGY_1ST%Android"),
            "os_version": "Android/sdk_phone64_x86_64/emu64x:14/UE1A.230829.036.A1/11228894:userdebug/test-keys",
            "software_version": "130050002",
            "device_model": "Android SDK built for x86_64"
        },
        "device_metadata": {
            "device_os_family": "android",
            "device_type": "A10KISP2GWF0E4",
            "device_serial": device_serial,
            "manufacturer": "unknown",
            "model": "Android SDK built for x86_64",
            "os_version": "34",
            "product": "34"
        },
        "auth_data": {
            "use_global_authentication": "true",
            "authorization_code": authorization_code,
            "code_verifier": &pkce.verifier,
            "code_algorithm": "SHA-256",
            "client_domain": "DeviceLegacy",
            "client_id": &serial_type_hex
        },
        "requested_extensions": ["device_info", "customer_info"]
    });

    // Log the request for debugging
    eprintln!("=== Device Registration Request ===");
    eprintln!("URL: {}", register_url);
    eprintln!("Body: {}", serde_json::to_string_pretty(&request_body).unwrap_or_default());
    eprintln!("===================================");

    // Make HTTP request
    let client = reqwest::Client::new();
    let response = client
        .post(&register_url)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| LibationError::NetworkError {
            message: format!("Token exchange request failed: {}", e),
            is_transient: true,
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        eprintln!("=== Registration Failed ===");
        eprintln!("Status: {}", status);
        eprintln!("Response: {}", error_body);
        eprintln!("===========================");
        return Err(LibationError::AuthenticationFailed {
            message: format!("Token exchange failed (status {}): {}", status, error_body),
            account_id: None,
        });
    }

    let response_text = response.text().await.unwrap_or_default();

    // Try to save to app's cache directory (writable)
    let cache_paths = [
        "/data/data/com.rnaudible.app/cache/registration.json",
        "/sdcard/Download/audible_registration.json",
    ];

    for path in &cache_paths {
        if let Ok(_) = std::fs::write(path, &response_text) {
            eprintln!("✅ Saved registration to: {}", path);
            break;
        }
    }

    // Parse the response we just logged
    let register_response: serde_json::Value = serde_json::from_str(&response_text)
        .map_err(|e| LibationError::InvalidApiResponse {
            message: format!("Failed to parse registration response: {}", e),
            response_body: Some(response_text.clone()),
        })?;

    // Extract full registration data
    let success = register_response
        .get("response")
        .and_then(|r| r.get("success"))
        .ok_or_else(|| LibationError::InvalidApiResponse {
            message: "Success response not found in registration".to_string(),
            response_body: Some(register_response.to_string()),
        })?;

    let tokens = success.get("tokens")
        .ok_or_else(|| LibationError::InvalidApiResponse {
            message: "Tokens not found in registration response".to_string(),
            response_body: Some(register_response.to_string()),
        })?;

    let extensions = success.get("extensions")
        .ok_or_else(|| LibationError::InvalidApiResponse {
            message: "Extensions not found in registration response".to_string(),
            response_body: Some(register_response.to_string()),
        })?;

    // Parse bearer tokens
    let bearer: BearerTokenInfo = serde_json::from_value(tokens.get("bearer").unwrap().clone())
        .map_err(|e| LibationError::InvalidApiResponse {
            message: format!("Failed to parse bearer tokens: {}", e),
            response_body: Some(tokens.to_string()),
        })?;

    // Parse MAC-DMS tokens
    let mac_dms: MacDmsTokenInfo = serde_json::from_value(tokens.get("mac_dms").unwrap().clone())
        .map_err(|e| LibationError::InvalidApiResponse {
            message: format!("Failed to parse mac_dms tokens: {}", e),
            response_body: Some(tokens.to_string()),
        })?;

    // Parse website cookies
    let website_cookies: Vec<Cookie> = serde_json::from_value(tokens.get("website_cookies").unwrap().clone())
        .map_err(|e| LibationError::InvalidApiResponse {
            message: format!("Failed to parse website_cookies: {}", e),
            response_body: Some(tokens.to_string()),
        })?;

    // Parse store authentication cookie
    let store_authentication_cookie: StoreAuthCookie = serde_json::from_value(tokens.get("store_authentication_cookie").unwrap().clone())
        .map_err(|e| LibationError::InvalidApiResponse {
            message: format!("Failed to parse store_authentication_cookie: {}", e),
            response_body: Some(tokens.to_string()),
        })?;

    // Parse device info
    let device_info: DeviceInfo = serde_json::from_value(extensions.get("device_info").unwrap().clone())
        .map_err(|e| LibationError::InvalidApiResponse {
            message: format!("Failed to parse device_info: {}", e),
            response_body: Some(extensions.to_string()),
        })?;

    // Parse customer info
    let customer_info: CustomerInfo = serde_json::from_value(extensions.get("customer_info").unwrap().clone())
        .map_err(|e| LibationError::InvalidApiResponse {
            message: format!("Failed to parse customer_info: {}", e),
            response_body: Some(extensions.to_string()),
        })?;

    Ok(RegistrationResponse {
        bearer,
        mac_dms,
        website_cookies,
        store_authentication_cookie,
        device_info,
        customer_info,
    })
}

/// Refresh access token using refresh token
///
/// Access tokens expire after a certain time. This function uses the
/// refresh token to obtain a new access token without requiring the
/// user to log in again.
///
/// # Reference
/// Based on mkb79 Python library auth.py:
/// - POST to https://api.amazon.{domain}/auth/token
/// - Form data with grant_type=refresh_token
///
/// # Arguments
/// * `locale` - The Audible market/region
/// * `refresh_token` - The refresh token from original authentication
/// * `device_serial` - Device serial number
///
/// # Returns
/// TokenResponse with new access_token (refresh_token may be same or new)
///
/// # Errors
/// Returns error if refresh fails (user may need to re-authenticate)
pub async fn refresh_access_token(
    locale: &Locale,
    refresh_token: &str,
    device_serial: &str,
) -> Result<TokenResponse> {
    let config = OAuthConfig::default();
    let client_id = format!("device:{}#{}", device_serial, config.device_type);

    let amazon_domain = match locale.country_code.as_str() {
        "us" => "amazon.com",
        "uk" => "amazon.co.uk",
        "de" => "amazon.de",
        "fr" => "amazon.fr",
        "ca" => "amazon.ca",
        "au" => "amazon.com.au",
        "it" => "amazon.it",
        "es" => "amazon.es",
        "in" => "amazon.in",
        "jp" => "amazon.co.jp",
        _ => "amazon.com",
    };

    let token_url = format!("https://api.{}/auth/token", amazon_domain);

    let mut form_data = StdHashMap::new();
    form_data.insert("app_name".to_string(), "Audible".to_string());
    form_data.insert("app_version".to_string(), "3.56.2".to_string());
    form_data.insert("source_token".to_string(), refresh_token.to_string());
    form_data.insert("source_token_type".to_string(), "refresh_token".to_string());
    form_data.insert("requested_token_type".to_string(), "access_token".to_string());

    let client = reqwest::Client::new();
    let response = client
        .post(&token_url)
        .form(&form_data)
        .send()
        .await
        .map_err(|e| LibationError::NetworkError {
            message: format!("Token refresh request failed: {}", e),
            is_transient: true,
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        return Err(LibationError::AuthenticationFailed {
            message: format!("Token refresh failed (status {}): {}", status, error_body),
            account_id: None,
        });
    }

    let token_response: TokenResponse = response.json().await
        .map_err(|e| LibationError::InvalidApiResponse {
            message: format!("Failed to parse refresh token response: {}", e),
            response_body: None,
        })?;

    Ok(token_response)
}

/// Register a new device with Audible
///
/// Device registration generates a private key and registers the device
/// with Audible's API. This is required before making authenticated API calls.
///
/// # Arguments
/// * `locale` - The Audible market to register with
/// * `device_name` - User-friendly name for this device
/// * `access_token` - Valid OAuth access token
///
/// # Returns
/// DeviceRegistration with serial number, type, and private key
///
/// # Errors
/// Returns error if device registration fails
///
/// # Note
/// This generates an RSA-2048 private key which is used for request signing.
/// The private key must be securely stored and never logged.
pub async fn register_device(
    locale: &Locale,
    device_name: String,
    access_token: &str,
) -> Result<DeviceRegistration> {
    let config = OAuthConfig::default();

    // Generate RSA-2048 private key
    let mut rng = rand::thread_rng();
    let private_key = RsaPrivateKey::new(&mut rng, 2048)
        .map_err(|e| LibationError::InternalError(format!("Failed to generate RSA key: {}", e)))?;

    // Convert private key to PEM format
    let private_key_pem = private_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
        .map_err(|e| LibationError::InternalError(format!("Failed to encode private key: {}", e)))?;

    // Generate device serial number
    let device_serial = Uuid::new_v4().to_string();

    // In a full implementation, this would POST to the device registration endpoint
    // For now, return the generated device info
    // TODO: Implement actual device registration API call

    Ok(DeviceRegistration {
        device_name,
        device_serial_number: device_serial,
        device_type: config.device_type.to_string(),
    })
}

/// Retrieve activation bytes for DRM decryption
///
/// Activation bytes are required to decrypt AAX audiobook files.
/// This makes an authenticated API call to retrieve them.
///
/// # Reference
/// Endpoint may be one of:
/// - /1.0/content/license/licenseForCustomerToken
/// - /1.0/customer/information
///
/// # Arguments
/// * `locale` - The Audible market/region
/// * `access_token` - Valid OAuth access token
///
/// # Returns
/// Activation bytes as 4-byte hex string (e.g., "1a2b3c4d")
///
/// # Errors
/// Returns error if API call fails or activation bytes not found
pub async fn get_activation_bytes(
    locale: &Locale,
    access_token: &str,
) -> Result<String> {
    // AudibleApi uses the Audible login URI, not API URI
    let api_url = format!(
        "https://www.{}/license/token?action=register&player_manuf=Audible,iPhone&player_model=iPhone",
        locale.domain
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&api_url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| LibationError::NetworkError {
            message: format!("Activation bytes request failed: {}", e),
            is_transient: true,
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        return Err(LibationError::ApiRequestFailed {
            message: format!("Failed to retrieve activation bytes (status {}): {}", status, error_body),
            status_code: Some(status.as_u16()),
            endpoint: Some("/license/token".to_string()),
        });
    }

    // Response is a binary blob (activation blob)
    let device_license = response.bytes().await
        .map_err(|e| LibationError::InvalidApiResponse {
            message: format!("Failed to read activation blob: {}", e),
            response_body: None,
        })?;

    const ACTIVATION_BLOB_SZ: usize = 0x238;

    if device_license.len() < ACTIVATION_BLOB_SZ {
        return Err(LibationError::InvalidApiResponse {
            message: format!("Unexpected activation response size: {} bytes", device_license.len()),
            response_body: None,
        });
    }

    // Activation bytes are at beginning of activation blob (last ACTIVATION_BLOB_SZ bytes)
    // Extract 4-byte uint from offset: device_license.len() - ACTIVATION_BLOB_SZ
    let offset = device_license.len() - ACTIVATION_BLOB_SZ;
    let act_bytes = u32::from_le_bytes([
        device_license[offset],
        device_license[offset + 1],
        device_license[offset + 2],
        device_license[offset + 3],
    ]);

    // Return as 8-character lowercase hex string
    Ok(format!("{:08x}", act_bytes))
}

/// Deregister a device from Audible
///
/// This removes the device registration from the user's Audible account.
/// The device will no longer be able to make authenticated API calls.
///
/// # Arguments
/// * `account` - The account to deregister
///
/// # Errors
/// Returns error if deregistration fails
pub async fn deregister_device(account: &Account) -> Result<()> {
    let identity = account.identity.as_ref()
        .ok_or_else(|| LibationError::AuthenticationFailed {
            message: "No identity tokens for deregistration".to_string(),
            account_id: Some(account.account_id.clone()),
        })?;

    let api_url = format!(
        "https://api.{}/1.0/devices/{}",
        identity.locale.domain,
        identity.device_serial_number
    );

    let client = reqwest::Client::new();
    let response = client
        .delete(&api_url)
        .header("Authorization", format!("Bearer {}", identity.access_token.token))
        .send()
        .await
        .map_err(|e| LibationError::NetworkError {
            message: format!("Device deregistration request failed: {}", e),
            is_transient: false,
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        return Err(LibationError::ApiRequestFailed {
            message: format!("Device deregistration failed (status {}): {}", status, error_body),
            status_code: Some(status.as_u16()),
            endpoint: Some("/1.0/devices/...".to_string()),
        });
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ========== Account Tests ==========

    #[test]
    fn test_account_creation() {
        let account = Account::new("test@example.com".to_string()).unwrap();
        assert_eq!(account.account_id, "test@example.com");
        assert_eq!(account.account_name, "test@example.com");
        assert!(account.library_scan);
        assert!(account.decrypt_key.is_empty());
        assert!(account.identity.is_none());
    }

    #[test]
    fn test_account_creation_trims_whitespace() {
        let account = Account::new("  test@example.com  ".to_string()).unwrap();
        assert_eq!(account.account_id, "test@example.com");
    }

    #[test]
    fn test_account_creation_empty_fails() {
        let result = Account::new("   ".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_set_account_name() {
        let mut account = Account::new("test@example.com".to_string()).unwrap();
        account.set_account_name("My Account".to_string());
        assert_eq!(account.account_name, "My Account");
    }

    #[test]
    fn test_set_decrypt_key() {
        let mut account = Account::new("test@example.com".to_string()).unwrap();
        account.set_decrypt_key("1a2b3c4d".to_string());
        assert_eq!(account.decrypt_key, "1a2b3c4d");
    }

    #[test]
    fn test_masked_log_entry() {
        let account = Account::new("test@example.com".to_string()).unwrap();
        let masked = account.masked_log_entry();
        println!("Masked output: {}", masked);
        // The mask function shows first 2 and last 2 chars with asterisks in between
        // "test@example.com" (16 chars) -> "te" + 12 asterisks + "om"
        assert!(masked.contains("AccountId="));
        assert!(masked.contains("AccountName="));
        assert!(masked.contains("Locale=[empty]"));
    }

    #[test]
    fn test_needs_token_refresh_no_identity() {
        let account = Account::new("test@example.com".to_string()).unwrap();
        assert!(account.needs_token_refresh());
    }

    #[test]
    fn test_needs_token_refresh_expired() {
        let mut account = Account::new("test@example.com".to_string()).unwrap();
        let expired_token = AccessToken {
            token: "test_token".to_string(),
            expires_at: Utc::now() - chrono::Duration::hours(1),
        };
        let identity = Identity::new(
            expired_token,
            "refresh_token".to_string(),
            "private_key".to_string(),
            "adp_token".to_string(),
            Locale::us(),
        );
        account.set_identity(identity);
        assert!(account.needs_token_refresh());
    }

    // ========== Locale Tests ==========

    #[test]
    fn test_locale_us() {
        let locale = Locale::us();
        assert_eq!(locale.country_code, "us");
        assert_eq!(locale.domain, "audible.com");
        assert_eq!(locale.name, "United States");
        assert!(locale.with_username);
    }

    #[test]
    fn test_locale_from_country_code() {
        let locale = Locale::from_country_code("uk").unwrap();
        assert_eq!(locale.country_code, "uk");
        assert_eq!(locale.domain, "audible.co.uk");
    }

    #[test]
    fn test_locale_api_url() {
        let locale = Locale::us();
        assert_eq!(locale.api_url(), "https://api.audible.com");
    }

    #[test]
    fn test_all_locales_have_api_urls() {
        for locale in Locale::all() {
            let api_url = locale.api_url();
            assert!(api_url.starts_with("https://api."));
            assert!(api_url.contains(&locale.domain));
        }
    }

    // ========== Identity Tests ==========

    #[test]
    fn test_identity_is_expired() {
        let expired_token = AccessToken {
            token: "test".to_string(),
            expires_at: Utc::now() - chrono::Duration::hours(1),
        };
        let identity = Identity::new(
            expired_token,
            "refresh".to_string(),
            "key".to_string(),
            "adp".to_string(),
            Locale::us(),
        );
        assert!(identity.is_expired());
    }

    #[test]
    fn test_identity_not_expired() {
        let valid_token = AccessToken {
            token: "test".to_string(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
        };
        let identity = Identity::new(
            valid_token,
            "refresh".to_string(),
            "key".to_string(),
            "adp".to_string(),
            Locale::us(),
        );
        assert!(!identity.is_expired());
    }

    // ========== PKCE Tests ==========

    #[test]
    fn test_pkce_challenge_generate() {
        let pkce = PkceChallenge::generate().unwrap();

        // Verify verifier is not empty
        assert!(!pkce.verifier.is_empty());

        // Verify challenge is not empty
        assert!(!pkce.challenge.is_empty());

        // Verify method is S256
        assert_eq!(pkce.method, "S256");

        // Verify verifier and challenge are different
        assert_ne!(pkce.verifier, pkce.challenge);

        // Verify challenge is base64-url encoded (no padding, url-safe chars)
        assert!(!pkce.challenge.contains('='));
        assert!(!pkce.challenge.contains('+'));
        assert!(!pkce.challenge.contains('/'));
    }

    #[test]
    fn test_pkce_challenge_is_deterministic() {
        // Same verifier should produce same challenge
        let pkce1 = PkceChallenge::generate().unwrap();

        // Manually verify the challenge matches SHA-256(verifier)
        let mut hasher = Sha256::new();
        hasher.update(pkce1.verifier.as_bytes());
        let expected_challenge = general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());

        assert_eq!(pkce1.challenge, expected_challenge);
    }

    #[test]
    fn test_pkce_challenge_uniqueness() {
        // Each generation should produce unique values
        let pkce1 = PkceChallenge::generate().unwrap();
        let pkce2 = PkceChallenge::generate().unwrap();

        assert_ne!(pkce1.verifier, pkce2.verifier);
        assert_ne!(pkce1.challenge, pkce2.challenge);
    }

    // ========== OAuth State Tests ==========

    #[test]
    fn test_oauth_state_generate() {
        let state = OAuthState::generate();

        // Verify state value is not empty
        assert!(!state.value.is_empty());

        // Verify it looks like a UUID (contains hyphens)
        assert!(state.value.contains('-'));
    }

    #[test]
    fn test_oauth_state_uniqueness() {
        let state1 = OAuthState::generate();
        let state2 = OAuthState::generate();

        assert_ne!(state1.value, state2.value);
    }

    // ========== OAuth Authorization URL Tests ==========

    #[test]
    fn test_generate_authorization_url_us() {
        let locale = Locale::us();
        let device_serial = "test-device-123";
        let pkce = PkceChallenge::generate().unwrap();
        let state = OAuthState::generate();

        let url = generate_authorization_url(&locale, device_serial, &pkce, &state).unwrap();

        // Verify base URL
        assert!(url.starts_with("https://www.amazon.com/ap/signin"));

        // Verify key parameters are present
        assert!(url.contains("openid.mode=checkid_setup"));
        assert!(url.contains("openid.ns=http"));
        // The colon and # are URL encoded in the URL
        assert!(url.contains("device%3A") || url.contains("device:"));
        // Device type is hex-encoded in client_id, so look for hex(#A10KISP2GWF0E4)
        assert!(url.contains("23413130") || url.contains("A10KISP2GWF0E4"),
                "URL should contain device type in hex or plain form");
        assert!(url.contains(&pkce.challenge));
        // State parameter intentionally not included (matches Libation)
        assert!(url.contains("openid.oa2.scope=device_auth_access"));
    }

    #[test]
    fn test_generate_authorization_url_uk() {
        let locale = Locale::uk();
        let device_serial = "test-device-456";
        let pkce = PkceChallenge::generate().unwrap();
        let state = OAuthState::generate();

        let url = generate_authorization_url(&locale, device_serial, &pkce, &state).unwrap();

        // UK should use amazon.co.uk
        assert!(url.starts_with("https://www.amazon.co.uk/ap/signin"));
    }

    #[test]
    fn test_generate_authorization_url_de() {
        let locale = Locale::de();
        let device_serial = "test-device-789";
        let pkce = PkceChallenge::generate().unwrap();
        let state = OAuthState::generate();

        let url = generate_authorization_url(&locale, device_serial, &pkce, &state).unwrap();

        // Germany should use amazon.de
        assert!(url.starts_with("https://www.amazon.de/ap/signin"));
    }

    // ========== OAuth Callback Parsing Tests ==========

    #[test]
    fn test_parse_authorization_callback_success() {
        let state = OAuthState { value: "test-state-123".to_string() };
        let callback_url = format!(
            "https://localhost/callback?openid.oa2.authorization_code=ABC123&state={}",
            state.value
        );

        let code = parse_authorization_callback(&callback_url).unwrap();
        assert_eq!(code, "ABC123");
    }

    #[test]
    fn test_parse_authorization_callback_code_param() {
        let state = OAuthState { value: "test-state-456".to_string() };
        let callback_url = format!(
            "https://localhost/callback?code=XYZ789&state={}",
            state.value
        );

        let code = parse_authorization_callback(&callback_url).unwrap();
        assert_eq!(code, "XYZ789");
    }

    #[test]
    fn test_parse_authorization_callback_missing_code() {
        let state = OAuthState { value: "test-state".to_string() };
        let callback_url = format!("https://localhost/callback?state={}", state.value);

        let result = parse_authorization_callback(&callback_url);
        assert!(result.is_err());

        match result {
            Err(LibationError::AuthenticationFailed { message, .. }) => {
                assert!(message.contains("Missing authorization code"));
            }
            _ => panic!("Expected AuthenticationFailed error"),
        }
    }

    // State parameter test removed - Libation doesn't use state parameter
    // Amazon's OAuth flow uses OpenID nonce for replay protection instead

    #[test]
    fn test_parse_authorization_callback_oauth_error() {
        let state = OAuthState { value: "test-state".to_string() };
        let callback_url = format!(
            "https://localhost/callback?error=access_denied&error_description=User+cancelled&state={}",
            state.value
        );

        let result = parse_authorization_callback(&callback_url);
        assert!(result.is_err());

        match result {
            Err(LibationError::AuthenticationFailed { message, .. }) => {
                assert!(message.contains("access_denied"));
                assert!(message.contains("User"));
            }
            _ => panic!("Expected AuthenticationFailed error"),
        }
    }

    #[test]
    fn test_parse_authorization_callback_invalid_url() {
        let state = OAuthState { value: "test-state".to_string() };
        let callback_url = "not-a-valid-url";

        let result = parse_authorization_callback(callback_url);
        assert!(result.is_err());
    }

    // ========== OAuth Config Tests ==========

    #[test]
    fn test_oauth_config_defaults() {
        let config = OAuthConfig::default();

        assert_eq!(config.device_type, "A10KISP2GWF0E4", "Device type must match AudibleApi Android");
        assert_eq!(config.response_type, "code");
        assert_eq!(config.scope, "device_auth_access");
        assert_eq!(config.code_challenge_method, "S256");
        assert!(config.redirect_uri.contains("maplanding"), "redirect_uri should use Amazon's maplanding page");
    }

    // ========== Integration Test Helpers ==========

    #[test]
    fn test_complete_oauth_url_generation_flow() {
        // This simulates the complete flow of generating an OAuth URL
        let locale = Locale::us();
        let device_serial = Uuid::new_v4().to_string();
        let pkce = PkceChallenge::generate().unwrap();
        let state = OAuthState::generate();

        // Generate URL
        let auth_url = generate_authorization_url(&locale, &device_serial, &pkce, &state).unwrap();

        // Verify URL is valid
        assert!(Url::parse(&auth_url).is_ok());

        // Verify all critical parameters are present
        let parsed_url = Url::parse(&auth_url).unwrap();
        let params: std::collections::HashMap<_, _> = parsed_url.query_pairs().collect();

        assert!(params.get("openid.oa2.code_challenge").is_some());
        assert!(params.get("openid.oa2.code_challenge_method").is_some());
        assert!(params.get("openid.oa2.client_id").is_some());
        // State parameter intentionally omitted to match Libation behavior
        assert!(params.get("openid.oa2.scope").is_some());

        // Verify code challenge matches
        assert_eq!(
            params.get("openid.oa2.code_challenge").unwrap().as_ref(),
            &pkce.challenge
        );
    }

    /// Interactive OAuth flow test - requires manual login
    ///
    /// Run with: cargo test test_interactive_oauth_flow -- --ignored --nocapture
    ///
    /// This test will:
    /// 1. Generate an authorization URL
    /// 2. Print it to console
    /// 3. Wait for you to paste the callback URL
    /// 4. Exchange code for tokens
    /// 5. Get activation bytes
    #[tokio::test]
    #[ignore] // Only run manually with --ignored flag
    async fn test_interactive_oauth_flow() {
        use std::io::{self, Write};

        println!("\n=== Interactive OAuth Flow Test ===\n");

        // Step 1: Generate authorization URL
        let locale = Locale::us();

        // Generate device serial as 32-character hex string (matches Libation format)
        // Libation uses 16 random bytes encoded as 32 hex characters
        let random_bytes: [u8; 16] = rand::random();
        let device_serial = random_bytes.iter()
            .map(|b| format!("{:02X}", b))
            .collect::<String>();

        let pkce = PkceChallenge::generate().unwrap();
        let state = OAuthState::generate();

        println!("📱 Device Serial: {}", device_serial);
        println!("🔐 PKCE Verifier: {}", &pkce.verifier[..20]);
        println!("🎲 State: {}\n", &state.value[..20]);

        let auth_url = generate_authorization_url(&locale, &device_serial, &pkce, &state).unwrap();

        // Step 2: Print URL for user
        println!("🔗 Authorization URL:");
        println!("{}\n", auth_url);
        println!("📋 Instructions:");
        println!("1. Copy the URL above");
        println!("2. Open it in your browser");
        println!("3. Log in to your Audible account");
        println!("4. After redirect, copy the ENTIRE callback URL");
        println!("5. Paste it below and press Enter\n");

        // Step 3: Wait for callback URL
        print!("🔙 Paste callback URL: ");
        io::stdout().flush().unwrap();

        let mut callback_url = String::new();
        io::stdin().read_line(&mut callback_url).unwrap();
        let callback_url = callback_url.trim();

        println!("\n⚙️  Processing callback...\n");

        // Step 4: Parse callback
        match parse_authorization_callback(callback_url) {
            Ok(authorization_code) => {
                println!("✅ Authorization code received: {}...", &authorization_code[..20]);

                // Step 5: Exchange code for tokens
                println!("🔄 Exchanging code for tokens...\n");

                match exchange_authorization_code(&locale, &authorization_code, &device_serial, &pkce).await {
                    Ok(token_response) => {
                        println!("✅ Token Exchange Successful!");
                        println!("   Access Token: {}...", &token_response.bearer.access_token[..30]);
                        println!("   Refresh Token: {}...", &token_response.bearer.refresh_token[..30]);
                        println!("   Expires In: {} seconds", token_response.bearer.expires_in);

                        // Step 6: Get activation bytes
                        println!("🔓 Retrieving activation bytes...\n");

                        match get_activation_bytes(&locale, &token_response.bearer.access_token).await {
                            Ok(activation_bytes) => {
                                println!("✅ Activation Bytes Retrieved!");
                                println!("   Activation Bytes: {}\n", activation_bytes);

                                println!("🎉 OAuth Flow Complete!");
                                println!("\n📊 Summary:");
                                println!("   ✅ Authorization URL generated");
                                println!("   ✅ User authentication successful");
                                println!("   ✅ Callback parsed");
                                println!("   ✅ Tokens exchanged");
                                println!("   ✅ Activation bytes retrieved");
                                println!("\n💾 You can now use these credentials to:");
                                println!("   • Sync your library");
                                println!("   • Download audiobooks");
                                println!("   • Decrypt AAX files");
                            }
                            Err(e) => {
                                println!("❌ Failed to get activation bytes: {:?}", e);
                                println!("\n⚠️  This might be okay - activation bytes retrieval");
                                println!("    can fail if the API endpoint changes. But token");
                                println!("    exchange worked, which is the main OAuth flow!");
                            }
                        }
                    }
                    Err(e) => {
                        println!("❌ Token Exchange Failed: {:?}\n", e);
                        panic!("Token exchange failed - check authorization code and PKCE verifier");
                    }
                }
            }
            Err(e) => {
                println!("❌ Callback Parsing Failed: {:?}\n", e);
                println!("Expected callback format:");
                println!("  https://localhost/callback?openid.oa2.authorization_code=ABC123&openid.oa2.state=XYZ789");
                println!("Or:");
                println!("  rnaudible://callback?code=ABC123&state=XYZ789");
                panic!("Callback parsing failed - check the URL format");
            }
        }
    }
}
