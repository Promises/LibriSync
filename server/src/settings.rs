//! User settings (`GET/PUT /api/v1/settings`), stored as one JSON document in
//! `server.db`. `general.apiKey` and `media.libraryDir` are read-only and filled in
//! from their real sources on read.

use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::template;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    M4b,
    Mp3,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::M4b => "m4b",
            Format::Mp3 => "mp3",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillOrder {
    NewestFirst,
    OldestFirst,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct General {
    pub instance_name: String,
    #[serde(default)]
    pub api_key: String,
    pub log_level: LogLevel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    #[serde(default)]
    pub library_dir: String,
    pub folder_template: String,
    pub no_series_folder_template: String,
    pub format: Format,
    pub write_sidecars: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSettings {
    pub interval_hours: u32,
    pub auto_download_new: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backfill {
    pub enabled: bool,
    pub order: BackfillOrder,
    pub window_start: Option<String>,
    pub window_end: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudibleSettings {
    pub daily_licence_cap: u32,
    pub spacing_min_minutes: u32,
    pub spacing_max_minutes: u32,
    pub throttle_cooldown_hours: u32,
    /// Download Audible Plus catalogue titles (`Books.is_ayce`) automatically. They can
    /// only be licensed while the customer has a Plus membership, so the default leaves
    /// them out of back-fill and new-purchase downloads; a manual download still works.
    /// Missing from settings saved before it existed, which therefore read as `false`.
    #[serde(default)]
    pub include_plus_catalog: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Downloads {
    pub concurrency: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub general: General,
    pub media: Media,
    pub sync: SyncSettings,
    pub backfill: Backfill,
    pub audible: AudibleSettings,
    pub downloads: Downloads,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            general: General {
                instance_name: "LibriSync".into(),
                api_key: String::new(),
                log_level: LogLevel::Info,
            },
            media: Media {
                library_dir: String::new(),
                folder_template: template::DEFAULT_SERIES_TEMPLATE.into(),
                no_series_folder_template: template::DEFAULT_TEMPLATE.into(),
                format: Format::M4b,
                write_sidecars: true,
            },
            sync: SyncSettings { interval_hours: 24, auto_download_new: true },
            backfill: Backfill {
                enabled: true,
                order: BackfillOrder::NewestFirst,
                window_start: None,
                window_end: None,
            },
            audible: AudibleSettings {
                daily_licence_cap: 20,
                spacing_min_minutes: 20,
                spacing_max_minutes: 60,
                throttle_cooldown_hours: 24,
                include_plus_catalog: false,
            },
            downloads: Downloads { concurrency: 1 },
        }
    }
}

/// A validation failure naming the offending field (`media.folderTemplate`).
#[derive(Debug, PartialEq)]
pub struct FieldError {
    pub field: &'static str,
    pub message: String,
}

fn err(field: &'static str, message: impl Into<String>) -> Result<(), FieldError> {
    Err(FieldError { field, message: message.into() })
}

/// Parse `HH:MM` (24 h).
pub fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    if s.len() != 5 {
        return None;
    }
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

impl Settings {
    pub fn validate(&self) -> Result<(), FieldError> {
        let name = self.general.instance_name.trim();
        if name.is_empty() || name.chars().count() > 100 {
            return err("general.instanceName", "must be 1–100 characters");
        }
        if let Err(e) = template::validate(&self.media.folder_template) {
            return err("media.folderTemplate", e);
        }
        if let Err(e) = template::validate(&self.media.no_series_folder_template) {
            return err("media.noSeriesFolderTemplate", e);
        }
        if !(1..=168).contains(&self.sync.interval_hours) {
            return err("sync.intervalHours", "must be between 1 and 168");
        }
        match (&self.backfill.window_start, &self.backfill.window_end) {
            (None, None) => {}
            (Some(s), Some(e)) => {
                let start = parse_hhmm(s);
                let end = parse_hhmm(e);
                if start.is_none() {
                    return err("backfill.windowStart", "must be HH:MM");
                }
                if end.is_none() {
                    return err("backfill.windowEnd", "must be HH:MM");
                }
                if start == end {
                    return err("backfill.windowEnd", "must differ from windowStart");
                }
            }
            (Some(_), None) => return err("backfill.windowEnd", "set both window times or neither"),
            (None, Some(_)) => return err("backfill.windowStart", "set both window times or neither"),
        }
        if !(1..=200).contains(&self.audible.daily_licence_cap) {
            return err("audible.dailyLicenceCap", "must be between 1 and 200");
        }
        if !(1..=1440).contains(&self.audible.spacing_min_minutes) {
            return err("audible.spacingMinMinutes", "must be between 1 and 1440");
        }
        if !(1..=1440).contains(&self.audible.spacing_max_minutes) {
            return err("audible.spacingMaxMinutes", "must be between 1 and 1440");
        }
        if self.audible.spacing_max_minutes < self.audible.spacing_min_minutes {
            return err("audible.spacingMaxMinutes", "must be at least spacingMinMinutes");
        }
        if !(1..=168).contains(&self.audible.throttle_cooldown_hours) {
            return err("audible.throttleCooldownHours", "must be between 1 and 168");
        }
        if !(1..=4).contains(&self.downloads.concurrency) {
            return err("downloads.concurrency", "must be between 1 and 4");
        }
        Ok(())
    }

    /// The back-fill window as times, when one is configured and valid.
    pub fn backfill_window(&self) -> Option<(NaiveTime, NaiveTime)> {
        let s = parse_hhmm(self.backfill.window_start.as_deref()?)?;
        let e = parse_hhmm(self.backfill.window_end.as_deref()?)?;
        Some((s, e))
    }
}

const SETTINGS_KEY: &str = "settings";
const API_KEY_KEY: &str = "api_key";

/// Load settings; a missing or unreadable document yields the defaults (logged).
pub async fn load(pool: &SqlitePool) -> sqlx::Result<Settings> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(SETTINGS_KEY)
        .fetch_optional(pool)
        .await?;
    Ok(match row {
        None => Settings::default(),
        Some((json,)) => match serde_json::from_str::<Settings>(&json) {
            Ok(s) if s.validate().is_ok() => s,
            Ok(_) | Err(_) => {
                tracing::warn!("Stored settings are invalid; using defaults");
                Settings::default()
            }
        },
    })
}

pub async fn save(pool: &SqlitePool, settings: &Settings) -> sqlx::Result<()> {
    let mut stored = settings.clone();
    // Read-only fields have their own sources; don't persist copies that could go stale.
    stored.general.api_key.clear();
    stored.media.library_dir.clear();
    let json = serde_json::to_string(&stored).map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
    sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(SETTINGS_KEY)
        .bind(json)
        .execute(pool)
        .await?;
    Ok(())
}

pub fn generate_api_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// The API key, generated and stored on first start.
pub async fn load_or_create_api_key(pool: &SqlitePool) -> sqlx::Result<String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(API_KEY_KEY)
        .fetch_optional(pool)
        .await?;
    if let Some((key,)) = row {
        return Ok(key);
    }
    let key = generate_api_key();
    store_api_key(pool, &key).await?;
    Ok(key)
}

pub async fn store_api_key(pool: &SqlitePool, key: &str) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(API_KEY_KEY)
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert_eq!(Settings::default().validate(), Ok(()));
    }

    fn field_of(s: &Settings) -> &'static str {
        s.validate().unwrap_err().field
    }

    #[test]
    fn rejects_out_of_range_numbers_naming_the_field() {
        let mut s = Settings::default();
        s.sync.interval_hours = 0;
        assert_eq!(field_of(&s), "sync.intervalHours");

        let mut s = Settings::default();
        s.downloads.concurrency = 5;
        assert_eq!(field_of(&s), "downloads.concurrency");

        let mut s = Settings::default();
        s.audible.daily_licence_cap = 201;
        assert_eq!(field_of(&s), "audible.dailyLicenceCap");

        let mut s = Settings::default();
        s.audible.spacing_min_minutes = 30;
        s.audible.spacing_max_minutes = 10;
        assert_eq!(field_of(&s), "audible.spacingMaxMinutes");
    }

    #[test]
    fn window_needs_both_ends_in_hhmm() {
        let mut s = Settings::default();
        s.backfill.window_start = Some("01:00".into());
        assert_eq!(field_of(&s), "backfill.windowEnd");

        s.backfill.window_end = Some("7:00".into());
        assert_eq!(field_of(&s), "backfill.windowEnd");

        s.backfill.window_end = Some("07:00".into());
        assert_eq!(s.validate(), Ok(()));
        assert!(s.backfill_window().is_some());

        s.backfill.window_end = Some("01:00".into());
        assert_eq!(field_of(&s), "backfill.windowEnd");
    }

    #[test]
    fn rejects_bad_templates() {
        let mut s = Settings::default();
        s.media.folder_template = "{author}/{nope}".into();
        assert_eq!(field_of(&s), "media.folderTemplate");
        let mut s = Settings::default();
        s.media.no_series_folder_template = "../{title}".into();
        assert_eq!(field_of(&s), "media.noSeriesFolderTemplate");
    }

    #[test]
    fn json_shape_is_camel_case_and_rejects_missing_fields() {
        let json = serde_json::to_value(Settings::default()).unwrap();
        assert!(json["media"]["folderTemplate"].is_string());
        assert_eq!(json["backfill"]["order"], "newest_first");
        assert_eq!(json["media"]["format"], "m4b");

        let mut bad = json.clone();
        bad["downloads"].as_object_mut().unwrap().remove("concurrency");
        assert!(serde_json::from_value::<Settings>(bad).is_err());
    }

    /// Settings saved before the Audible Plus switch existed load with it off, and the
    /// switch round-trips under its API name.
    #[test]
    fn plus_catalog_defaults_off_for_existing_settings() {
        assert!(!Settings::default().audible.include_plus_catalog);
        let mut stored = serde_json::to_value(Settings::default()).unwrap();
        stored["audible"].as_object_mut().unwrap().remove("includePlusCatalog");
        let loaded: Settings = serde_json::from_value(stored).unwrap();
        assert!(!loaded.audible.include_plus_catalog);

        let mut on = Settings::default();
        on.audible.include_plus_catalog = true;
        let json = serde_json::to_value(&on).unwrap();
        assert_eq!(json["audible"]["includePlusCatalog"], true);
        assert_eq!(serde_json::from_value::<Settings>(json).unwrap(), on);
    }

    #[tokio::test]
    async fn stored_settings_without_the_plus_switch_still_load() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)").execute(&pool).await.unwrap();
        let mut stored = serde_json::to_value(Settings::default()).unwrap();
        stored["audible"].as_object_mut().unwrap().remove("includePlusCatalog");
        stored["audible"]["dailyLicenceCap"] = 7.into();
        sqlx::query("INSERT INTO settings (key, value) VALUES ('settings', ?)")
            .bind(stored.to_string())
            .execute(&pool)
            .await
            .unwrap();
        let loaded = load(&pool).await.unwrap();
        assert_eq!(loaded.audible.daily_licence_cap, 7, "the stored document was used, not the defaults");
        assert!(!loaded.audible.include_plus_catalog);
    }
}
