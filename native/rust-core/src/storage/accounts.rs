// LibriSync - Audible Library Sync for Mobile
// Copyright (C) 2025 Henning Berge
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//! Account storage operations
//!
//! Functions for saving and retrieving account data from SQLite.
//! Accounts are stored as JSON in the database for flexibility.

use crate::error::{LibationError, Result};
use sqlx::SqlitePool;

/// Save or update account in database
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `account_id` - Account identifier (email or username)
/// * `account_json` - Complete account JSON (includes identity, locale, etc.)
///
/// # Returns
/// Success status
pub async fn save_account(pool: &SqlitePool, account_id: &str, account_json: &str) -> Result<()> {
    // Parse JSON to extract key fields
    let account: serde_json::Value = serde_json::from_str(account_json)
        .map_err(|e| LibationError::InvalidInput(format!("Invalid account JSON: {}", e)))?;

    let account_name = account["account_name"].as_str().unwrap_or(account_id);

    // Provider tag (multi-provider). Defaults to 'audible' for legacy accounts.
    let provider = account["provider"].as_str().unwrap_or("audible");

    // Locale is optional: Audible accounts carry one; DRM-free providers (Libro.fm)
    // don't. Fall back to "us" so a locale-less account doesn't fail to save.
    let locale_code = account
        .get("locale")
        .or_else(|| account.get("identity").and_then(|identity| identity.get("locale")))
        .and_then(|locale| locale["country_code"].as_str())
        .unwrap_or("us");

    // Extract identity JSON
    let identity_json = account["identity"].to_string();

    // Extract token expiry if available
    let token_expires_at = account["identity"]["access_token"]["expires_at"].as_str();

    let decrypt_key = account["decrypt_key"].as_str();

    // Insert or replace account
    sqlx::query(
        r#"
        INSERT INTO Accounts (
            account_id,
            account_name,
            locale_code,
            identity_json,
            token_expires_at,
            decrypt_key,
            provider
        ) VALUES (?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(account_id) DO UPDATE SET
            account_name = excluded.account_name,
            locale_code = excluded.locale_code,
            identity_json = excluded.identity_json,
            token_expires_at = excluded.token_expires_at,
            decrypt_key = excluded.decrypt_key,
            provider = excluded.provider,
            updated_at = CURRENT_TIMESTAMP
        "#,
    )
    .bind(account_id)
    .bind(account_name)
    .bind(locale_code)
    .bind(&identity_json)
    .bind(token_expires_at)
    .bind(decrypt_key)
    .bind(provider)
    .execute(pool)
    .await?;

    Ok(())
}

/// Get account from database by account_id
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `account_id` - Account identifier
///
/// # Returns
/// Complete account JSON or None if not found
pub async fn get_account(pool: &SqlitePool, account_id: &str) -> Result<Option<String>> {
    let row: Option<(String, String, String, String, Option<String>, String)> = sqlx::query_as(
        r#"
        SELECT
            account_id,
            account_name,
            locale_code,
            identity_json,
            decrypt_key,
            COALESCE(provider, 'audible')
        FROM Accounts
        WHERE account_id = ?
        "#,
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?;

    if let Some((acc_id, acc_name, locale_code, identity_json, decrypt_key, provider)) = row {
        // Parse identity JSON from database
        let identity: serde_json::Value = serde_json::from_str(&identity_json).map_err(|e| {
            LibationError::InvalidState(format!("Corrupt identity JSON in database: {}", e))
        })?;

        let locale = identity.get("locale").cloned().unwrap_or_else(|| {
            serde_json::json!({
                "country_code": locale_code
            })
        });

        // Reconstruct account using serde_json (proper serialization). `provider` must
        // round-trip: without it every account reads back as Audible and the UI would
        // hand a Libro.fm account to the Audible sync/refresh path.
        let mut account = serde_json::json!({
            "account_id": acc_id,
            "account_name": acc_name,
            "provider": provider,
            "locale": locale,
            "identity": identity,
            "library_scan": true
        });

        // Add decrypt_key if present
        if let Some(key) = decrypt_key {
            account["decrypt_key"] = serde_json::Value::String(key);
        }

        // Serialize to string (serde handles escaping correctly)
        Ok(Some(serde_json::to_string(&account)?))
    } else {
        Ok(None)
    }
}

/// Get primary account (first account in database)
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `provider` - Scope to one provider (`"audible"`, `"librofm"`, …), or `None`
///   for the first account of any provider. Callers on a provider-specific path
///   must scope, or "the first account" can be another provider's.
///
/// # Returns
/// Complete account JSON or None if no accounts exist
pub async fn get_primary_account(
    pool: &SqlitePool,
    provider: Option<&str>,
) -> Result<Option<String>> {
    let row: Option<(String,)> = match provider {
        Some(provider) => {
            sqlx::query_as(
                r#"
                SELECT account_id
                FROM Accounts
                WHERE COALESCE(provider, 'audible') = ?
                ORDER BY created_at ASC
                LIMIT 1
                "#,
            )
            .bind(provider)
            .fetch_optional(pool)
            .await?
        }
        None => {
            sqlx::query_as(
                r#"
                SELECT account_id
                FROM Accounts
                ORDER BY created_at ASC
                LIMIT 1
                "#,
            )
            .fetch_optional(pool)
            .await?
        }
    };

    if let Some((account_id,)) = row {
        get_account(pool, &account_id).await
    } else {
        Ok(None)
    }
}

/// Get all accounts in creation order.
///
/// `provider` scopes the result to one provider (`"audible"`, `"librofm"`, …);
/// `None` returns every account. Callers that drive a provider-specific screen
/// must scope, or they will hand another provider's accounts to a sync/refresh
/// path that can't handle them. Rows written before migration 7 default to
/// `audible`, so the COALESCE keeps legacy accounts visible.
pub async fn get_all_accounts(
    pool: &SqlitePool,
    provider: Option<&str>,
) -> Result<Vec<String>> {
    let account_ids: Vec<String> = match provider {
        Some(provider) => {
            sqlx::query_scalar(
                r#"
                SELECT account_id
                FROM Accounts
                WHERE COALESCE(provider, 'audible') = ?
                ORDER BY created_at ASC
                "#,
            )
            .bind(provider)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_scalar(
                r#"
                SELECT account_id
                FROM Accounts
                ORDER BY created_at ASC
                "#,
            )
            .fetch_all(pool)
            .await?
        }
    };

    let mut accounts = Vec::with_capacity(account_ids.len());
    for account_id in account_ids {
        if let Some(account) = get_account(pool, &account_id).await? {
            accounts.push(account);
        }
    }

    Ok(accounts)
}

/// Update token expiry timestamp
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `account_id` - Account identifier
/// * `expires_at` - ISO 8601 timestamp
pub async fn update_token_expiry(
    pool: &SqlitePool,
    account_id: &str,
    expires_at: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE Accounts
        SET token_expires_at = ?,
            last_token_refresh = CURRENT_TIMESTAMP
        WHERE account_id = ?
        "#,
    )
    .bind(expires_at)
    .bind(account_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Update last library sync timestamp
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `account_id` - Account identifier
pub async fn update_last_sync(pool: &SqlitePool, account_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE Accounts
        SET last_library_sync = CURRENT_TIMESTAMP
        WHERE account_id = ?
        "#,
    )
    .bind(account_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Delete account from database
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `account_id` - Account identifier
pub async fn delete_account(pool: &SqlitePool, account_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM BookAccounts WHERE account = ?")
        .bind(account_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM LibraryBooks WHERE account = ?")
        .bind(account_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM Accounts WHERE account_id = ?")
        .bind(account_id)
        .execute(pool)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::database::Database;

    #[tokio::test]
    async fn test_save_and_get_account() {
        let db = Database::new_in_memory().await.unwrap();

        let account_json = r#"{
            "account_id": "test@example.com",
            "account_name": "Test Account",
            "locale": {"country_code": "us", "name": "United States", "domain": "audible.com", "with_username": true},
            "identity": {
                "access_token": {"token": "abc123", "expires_at": "2025-01-01T00:00:00Z"},
                "refresh_token": "xyz789",
                "device_serial_number": "ABC123",
                "locale": {"country_code": "us", "name": "United States", "domain": "audible.com", "with_username": true}
            },
            "decrypt_key": "12345678"
        }"#;

        // Save account
        save_account(db.pool(), "test@example.com", account_json)
            .await
            .unwrap();

        // Get account back
        let retrieved = get_account(db.pool(), "test@example.com")
            .await
            .unwrap()
            .expect("Account not found");

        // Verify it contains expected fields
        let retrieved_json: serde_json::Value = serde_json::from_str(&retrieved).unwrap();
        assert_eq!(retrieved_json["account_id"], "test@example.com");
        assert_eq!(retrieved_json["locale"]["country_code"], "us");
        assert_eq!(retrieved_json["locale"]["domain"], "audible.com");
        assert_eq!(retrieved_json["locale"]["with_username"], true);
    }

    #[tokio::test]
    async fn test_get_primary_account() {
        let db = Database::new_in_memory().await.unwrap();

        let account1 = r#"{"account_id": "first@example.com", "account_name": "First", "locale": {"country_code": "us"}, "identity": {"access_token": {"token": "a"},"refresh_token": "b","device_serial_number": "c"}}"#;
        let account2 = r#"{"account_id": "second@example.com", "account_name": "Second", "locale": {"country_code": "uk"}, "identity": {"access_token": {"token": "d"},"refresh_token": "e","device_serial_number": "f"}}"#;

        save_account(db.pool(), "first@example.com", account1)
            .await
            .unwrap();
        save_account(db.pool(), "second@example.com", account2)
            .await
            .unwrap();

        // Primary should be first one created
        let primary = get_primary_account(db.pool(), None).await.unwrap().unwrap();
        let primary_json: serde_json::Value = serde_json::from_str(&primary).unwrap();
        assert_eq!(primary_json["account_id"], "first@example.com");
    }

    #[tokio::test]
    async fn get_all_accounts_scopes_by_provider() {
        let db = Database::new_in_memory().await.unwrap();

        // A pre-multi-provider account: no `provider` field, must read as audible.
        let audible = r#"{"account_id": "a@example.com", "account_name": "Audible",
            "locale": {"country_code": "us"},
            "identity": {"access_token": {"token": "a"},"refresh_token": "b","device_serial_number": "c"}}"#;
        // Libro.fm: provider-tagged, and deliberately locale-less.
        let librofm = r#"{"account_id": "l@example.com", "account_name": "Libro",
            "provider": "librofm", "identity": {"access_token": "tok", "username": "l@example.com"}}"#;

        save_account(db.pool(), "a@example.com", audible).await.unwrap();
        save_account(db.pool(), "l@example.com", librofm).await.unwrap();

        let ids = |accounts: Vec<String>| -> Vec<String> {
            accounts
                .iter()
                .map(|a| {
                    serde_json::from_str::<serde_json::Value>(a).unwrap()["account_id"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .collect()
        };

        assert_eq!(
            ids(get_all_accounts(db.pool(), Some("audible")).await.unwrap()),
            vec!["a@example.com"]
        );
        assert_eq!(
            ids(get_all_accounts(db.pool(), Some("librofm")).await.unwrap()),
            vec!["l@example.com"]
        );
        assert_eq!(get_all_accounts(db.pool(), None).await.unwrap().len(), 2);

        assert_eq!(
            get_primary_account(db.pool(), Some("librofm"))
                .await
                .unwrap()
                .map(|a| serde_json::from_str::<serde_json::Value>(&a).unwrap()["account_id"]
                    .as_str()
                    .unwrap()
                    .to_string()),
            Some("l@example.com".to_string())
        );
    }

    #[tokio::test]
    async fn provider_round_trips_through_get_account() {
        let db = Database::new_in_memory().await.unwrap();

        let librofm = r#"{"account_id": "l@example.com", "account_name": "Libro",
            "provider": "librofm", "identity": {"access_token": "tok"}}"#;
        save_account(db.pool(), "l@example.com", librofm).await.unwrap();

        let back: serde_json::Value =
            serde_json::from_str(&get_account(db.pool(), "l@example.com").await.unwrap().unwrap())
                .unwrap();
        assert_eq!(back["provider"], "librofm");

        // Legacy row with no provider column value still reads as audible.
        let audible = r#"{"account_id": "a@example.com", "account_name": "A",
            "locale": {"country_code": "us"}, "identity": {"access_token": {"token": "t"}}}"#;
        save_account(db.pool(), "a@example.com", audible).await.unwrap();
        let back: serde_json::Value =
            serde_json::from_str(&get_account(db.pool(), "a@example.com").await.unwrap().unwrap())
                .unwrap();
        assert_eq!(back["provider"], "audible");
    }
}
