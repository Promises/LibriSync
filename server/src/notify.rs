//! Notifications ("Connect"): Sonarr-style webhooks, Apprise, and Audiobookshelf
//! library scans.
//!
//! Deliveries run in the background; a failure is recorded on the notification
//! (`lastResult`) and surfaces as a health warning, never as an error in the action
//! that triggered it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration as StdDuration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::error::ApiError;
use crate::history::{HistoryItem, EVENTS};
use crate::state::App;
use crate::time;

pub const MASK: &str = "********";
const ABS_SCAN_INTERVAL: StdDuration = StdDuration::from_secs(60);
const TIMEOUT: StdDuration = StdDuration::from_secs(15);

/// Events a notification may subscribe to: history events plus health changes.
pub fn is_known_event(e: &str) -> bool {
    EVENTS.contains(&e) || matches!(e, "health" | "healthRestored")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Webhook,
    Apprise,
    Audiobookshelf,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Webhook => "webhook",
            Kind::Apprise => "apprise",
            Kind::Audiobookshelf => "audiobookshelf",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "webhook" => Some(Kind::Webhook),
            "apprise" => Some(Kind::Apprise),
            "audiobookshelf" => Some(Kind::Audiobookshelf),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastResult {
    pub ok: bool,
    pub time: String,
    pub message: Option<String>,
}

/// `Notification` in API.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    #[serde(default)]
    pub id: i64,
    pub kind: Kind,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub events: Option<Vec<String>>,
    pub config: Value,
    #[serde(default, skip_deserializing)]
    pub last_result: Option<LastResult>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookConfig {
    pub url: String,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub headers: Vec<Header>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

fn default_method() -> String {
    "POST".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppriseConfig {
    pub server_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stateless_urls: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbsConfig {
    pub url: String,
    pub api_key: String,
    pub library_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
}

/// Header names whose values are treated as secrets.
fn is_secret_header(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    ["auth", "token", "key", "secret", "cookie", "password"].iter().any(|s| k.contains(s))
}

/// Replace secret values with [`MASK`] for API responses.
pub fn mask(kind: Kind, config: &mut Value) {
    let set = |v: &mut Value, field: &str| {
        if let Some(f) = v.get_mut(field) {
            if f.as_str().is_some_and(|s| !s.is_empty()) {
                *f = json!(MASK);
            }
        }
    };
    match kind {
        Kind::Webhook => {
            set(config, "password");
            if let Some(headers) = config.get_mut("headers").and_then(Value::as_array_mut) {
                for h in headers {
                    let secret = h.get("key").and_then(Value::as_str).is_some_and(is_secret_header);
                    if secret {
                        set(h, "value");
                    }
                }
            }
        }
        Kind::Apprise => set(config, "statelessUrls"),
        Kind::Audiobookshelf => set(config, "apiKey"),
    }
}

/// Where the client sent [`MASK`] back, keep the stored secret.
pub fn unmask(kind: Kind, config: &mut Value, stored: &Value) {
    let keep = |v: &mut Value, old: &Value, field: &str| {
        if v.get(field).and_then(Value::as_str) == Some(MASK) {
            v[field] = old.get(field).cloned().unwrap_or(Value::Null);
        }
    };
    match kind {
        Kind::Webhook => {
            keep(config, stored, "password");
            let old_headers: HashMap<String, Value> = stored
                .get("headers")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|h| Some((h.get("key")?.as_str()?.to_string(), h.get("value")?.clone())))
                        .collect()
                })
                .unwrap_or_default();
            if let Some(headers) = config.get_mut("headers").and_then(Value::as_array_mut) {
                for h in headers {
                    if h.get("value").and_then(Value::as_str) == Some(MASK) {
                        let key = h.get("key").and_then(Value::as_str).unwrap_or_default().to_string();
                        h["value"] = old_headers.get(&key).cloned().unwrap_or(Value::Null);
                    }
                }
            }
        }
        Kind::Apprise => keep(config, stored, "statelessUrls"),
        Kind::Audiobookshelf => keep(config, stored, "apiKey"),
    }
}

fn check_url(field: &str, url: &str) -> Result<(), ApiError> {
    match url::Url::parse(url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") && u.host().is_some() => Ok(()),
        _ => Err(ApiError::invalid_field(field, "must be an http(s) URL")),
    }
}

/// Validate a notification and normalise its config to the typed shape.
pub fn validate(n: &mut Notification) -> Result<(), ApiError> {
    if n.name.trim().is_empty() {
        return Err(ApiError::invalid_field("name", "must not be empty"));
    }
    if let Some(events) = &n.events {
        if let Some(bad) = events.iter().find(|e| !is_known_event(e)) {
            return Err(ApiError::invalid_field("events", format!("unknown event {bad}")));
        }
    }
    let bad_config = |e: serde_json::Error| ApiError::invalid_field("config", e.to_string());
    n.config = match n.kind {
        Kind::Webhook => {
            let mut c: WebhookConfig = serde_json::from_value(n.config.clone()).map_err(bad_config)?;
            check_url("config.url", &c.url)?;
            c.method = c.method.to_ascii_uppercase();
            if !matches!(c.method.as_str(), "POST" | "PUT") {
                return Err(ApiError::invalid_field("config.method", "must be POST or PUT"));
            }
            if c.headers.iter().any(|h| h.key.trim().is_empty()) {
                return Err(ApiError::invalid_field("config.headers", "header names must not be empty"));
            }
            serde_json::to_value(c).map_err(bad_config)?
        }
        Kind::Apprise => {
            let c: AppriseConfig = serde_json::from_value(n.config.clone()).map_err(bad_config)?;
            check_url("config.serverUrl", &c.server_url)?;
            let has_key = c.config_key.as_deref().is_some_and(|k| !k.trim().is_empty());
            let has_urls = c.stateless_urls.as_deref().is_some_and(|k| !k.trim().is_empty());
            if !has_key && !has_urls {
                return Err(ApiError::invalid_field("config.configKey", "set a configuration key or stateless URLs"));
            }
            serde_json::to_value(c).map_err(bad_config)?
        }
        Kind::Audiobookshelf => {
            let c: AbsConfig = serde_json::from_value(n.config.clone()).map_err(bad_config)?;
            check_url("config.url", &c.url)?;
            if c.api_key.trim().is_empty() {
                return Err(ApiError::invalid_field("config.apiKey", "must not be empty"));
            }
            if c.library_id.trim().is_empty() {
                return Err(ApiError::invalid_field("config.libraryId", "must not be empty"));
            }
            n.events = None;
            serde_json::to_value(c).map_err(bad_config)?
        }
    };
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    kind: String,
    name: String,
    enabled: bool,
    events: Option<String>,
    config: String,
    last_ok: Option<bool>,
    last_time: Option<String>,
    last_message: Option<String>,
}

impl Row {
    fn into_notification(self) -> Option<Notification> {
        Some(Notification {
            id: self.id,
            kind: Kind::parse(&self.kind)?,
            name: self.name,
            enabled: self.enabled,
            events: self.events.and_then(|e| serde_json::from_str(&e).ok()),
            config: serde_json::from_str(&self.config).unwrap_or(Value::Null),
            last_result: match (self.last_ok, self.last_time) {
                (Some(ok), Some(time)) => Some(LastResult { ok, time, message: self.last_message }),
                _ => None,
            },
        })
    }
}

const COLS: &str = "id, kind, name, enabled, events, config, last_ok, last_time, last_message";

pub async fn list(pool: &SqlitePool) -> sqlx::Result<Vec<Notification>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("SELECT {COLS} FROM notifications ORDER BY id"))
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().filter_map(Row::into_notification).collect())
}

pub async fn get(pool: &SqlitePool, id: i64) -> sqlx::Result<Option<Notification>> {
    let row: Option<Row> = sqlx::query_as(&format!("SELECT {COLS} FROM notifications WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(Row::into_notification))
}

pub async fn insert(pool: &SqlitePool, n: &Notification) -> sqlx::Result<i64> {
    Ok(sqlx::query("INSERT INTO notifications (kind, name, enabled, events, config) VALUES (?, ?, ?, ?, ?)")
        .bind(n.kind.as_str())
        .bind(n.name.trim())
        .bind(n.enabled)
        .bind(n.events.as_ref().map(|e| json!(e).to_string()))
        .bind(n.config.to_string())
        .execute(pool)
        .await?
        .last_insert_rowid())
}

pub async fn update(pool: &SqlitePool, n: &Notification) -> sqlx::Result<bool> {
    Ok(sqlx::query("UPDATE notifications SET kind = ?, name = ?, enabled = ?, events = ?, config = ? WHERE id = ?")
        .bind(n.kind.as_str())
        .bind(n.name.trim())
        .bind(n.enabled)
        .bind(n.events.as_ref().map(|e| json!(e).to_string()))
        .bind(n.config.to_string())
        .bind(n.id)
        .execute(pool)
        .await?
        .rows_affected()
        == 1)
}

pub async fn delete(pool: &SqlitePool, id: i64) -> sqlx::Result<bool> {
    Ok(sqlx::query("DELETE FROM notifications WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        == 1)
}

async fn record_result(pool: &SqlitePool, id: i64, result: &Result<String, String>) {
    let (ok, msg) = match result {
        Ok(m) => (true, Some(m.clone())),
        Err(e) => (false, Some(e.clone())),
    };
    if let Err(e) = sqlx::query("UPDATE notifications SET last_ok = ?, last_time = ?, last_message = ? WHERE id = ?")
        .bind(ok)
        .bind(time::now_str())
        .bind(msg)
        .bind(id)
        .execute(pool)
        .await
    {
        tracing::warn!("Could not record a notification result: {e}");
    }
}

/// What a webhook or Apprise message is about.
#[derive(Debug, Clone)]
pub struct Message {
    pub event_type: String,
    pub message: String,
    pub account: Option<Value>,
    pub book: Option<Value>,
}

pub struct Notifier {
    http: reqwest::Client,
    abs: Mutex<HashMap<i64, AbsScanState>>,
}

#[derive(Default)]
struct AbsScanState {
    last: Option<Instant>,
    pending: bool,
}

impl Notifier {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http, abs: Mutex::new(HashMap::new()) }
    }

    /// Fan a history event out to every subscribed notification (in the background).
    pub async fn dispatch(&self, app: &App, item: &HistoryItem) {
        let Ok(all) = list(&app.db).await else { return };
        let targets: Vec<Notification> = all
            .into_iter()
            .filter(|n| n.enabled)
            .filter(|n| match n.kind {
                Kind::Audiobookshelf => item.event == "download",
                _ => n.events.as_ref().is_none_or(|e| e.iter().any(|x| x == &item.event)),
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        let msg = message_for(app, item).await;
        for n in targets {
            self.spawn_send(app, n, msg.clone());
        }
    }

    /// Fan out an event that isn't a history item (health changes).
    pub async fn dispatch_raw(&self, app: &App, event_type: &str, message: &str) {
        let Ok(all) = list(&app.db).await else { return };
        let msg = Message { event_type: event_type.into(), message: message.into(), account: None, book: None };
        for n in all.into_iter().filter(|n| n.enabled && n.kind != Kind::Audiobookshelf) {
            if n.events.as_ref().is_none_or(|e| e.iter().any(|x| x == event_type)) {
                self.spawn_send(app, n, msg.clone());
            }
        }
    }

    fn spawn_send(&self, app: &App, n: Notification, msg: Message) {
        let Some(app) = app.self_arc() else { return };
        tokio::spawn(async move {
            if n.kind == Kind::Audiobookshelf {
                app.notifier.request_abs_scan(&app, n).await;
                return;
            }
            let result = app.notifier.send(&app, &n, &msg).await;
            if let Err(e) = &result {
                tracing::warn!("Notification '{}' failed: {e}", n.name);
            }
            record_result(&app.db, n.id, &result).await;
        });
    }

    /// Deliver one message now. Used for real events and for the test endpoint.
    pub async fn send(&self, app: &App, n: &Notification, msg: &Message) -> Result<String, String> {
        match n.kind {
            Kind::Webhook => {
                let c: WebhookConfig = serde_json::from_value(n.config.clone()).map_err(|e| e.to_string())?;
                let settings = app.settings();
                let mut body = json!({
                    "eventType": msg.event_type,
                    "instanceName": settings.general.instance_name,
                    "applicationUrl": app.cfg.application_url,
                    "time": time::now_str(),
                    "message": msg.message,
                });
                if let Some(a) = &msg.account {
                    body["account"] = a.clone();
                }
                if let Some(b) = &msg.book {
                    body["book"] = b.clone();
                }
                let mut req = if c.method == "PUT" { self.http.put(&c.url) } else { self.http.post(&c.url) };
                req = req.timeout(TIMEOUT).json(&body);
                for h in &c.headers {
                    req = req.header(h.key.as_str(), h.value.as_str());
                }
                if let Some(user) = c.username.as_deref().filter(|u| !u.is_empty()) {
                    req = req.basic_auth(user, c.password.as_deref());
                }
                let resp = req.send().await.map_err(|e| format!("Webhook request failed: {e}"))?;
                let status = resp.status();
                if status.is_success() {
                    Ok(format!("HTTP {}", status.as_u16()))
                } else {
                    Err(format!("Webhook answered HTTP {}", status.as_u16()))
                }
            }
            Kind::Apprise => {
                let c: AppriseConfig = serde_json::from_value(n.config.clone()).map_err(|e| e.to_string())?;
                let settings = app.settings();
                let base = c.server_url.trim_end_matches('/');
                let mut body = json!({
                    "title": format!("{}: {}", settings.general.instance_name, event_title(&msg.event_type)),
                    "body": msg.message,
                    "type": if msg.event_type.contains("Failed") || msg.event_type == "health" { "failure" } else { "info" },
                });
                let url = match c.config_key.as_deref().filter(|k| !k.trim().is_empty()) {
                    Some(key) => {
                        if let Some(tags) = c.tags.as_deref().filter(|t| !t.trim().is_empty()) {
                            body["tag"] = json!(tags);
                        }
                        format!("{base}/notify/{}", urlencoding::encode(key.trim()))
                    }
                    None => {
                        body["urls"] = json!(c.stateless_urls.unwrap_or_default());
                        format!("{base}/notify")
                    }
                };
                let resp = self
                    .http
                    .post(url)
                    .timeout(TIMEOUT)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("Apprise request failed: {e}"))?;
                let status = resp.status();
                if status.is_success() {
                    Ok(format!("HTTP {}", status.as_u16()))
                } else {
                    Err(format!("Apprise answered HTTP {}", status.as_u16()))
                }
            }
            Kind::Audiobookshelf => {
                let c: AbsConfig = serde_json::from_value(n.config.clone()).map_err(|e| e.to_string())?;
                let libs = abs_libraries(&self.http, &c.url, &c.api_key).await?;
                match libs.iter().find(|l| l.id == c.library_id) {
                    Some(l) => Ok(format!("Connected; library '{}' found", l.name)),
                    None => Err("Connected, but the library id was not found".into()),
                }
            }
        }
    }

    /// Scan an Audiobookshelf library, at most once per minute per notification: a
    /// request inside the minute schedules one scan at its end.
    async fn request_abs_scan(&self, app: &App, n: Notification) {
        let wait = {
            let Ok(mut map) = self.abs.lock() else { return };
            let st = map.entry(n.id).or_default();
            if st.pending {
                return;
            }
            let wait = st
                .last
                .map(|l| ABS_SCAN_INTERVAL.saturating_sub(l.elapsed()))
                .unwrap_or_default();
            st.pending = true;
            wait
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        if let Ok(mut map) = self.abs.lock() {
            let st = map.entry(n.id).or_default();
            st.pending = false;
            st.last = Some(Instant::now());
        }
        let result = abs_scan(&self.http, &n).await;
        if let Err(e) = &result {
            tracing::warn!("Audiobookshelf scan for '{}' failed: {e}", n.name);
        }
        record_result(&app.db, n.id, &result).await;
    }
}

fn event_title(event: &str) -> &'static str {
    match event {
        "test" => "Test",
        "bookAdded" => "New book",
        "grab" => "Downloading",
        "download" => "Downloaded",
        "downloadFailed" => "Download failed",
        "licenceDenied" => "Licence denied",
        "licenceThrottled" => "Licence throttled",
        "syncCompleted" => "Sync completed",
        "syncFailed" => "Sync failed",
        "accountAdded" => "Account added",
        "accountRemoved" => "Account removed",
        "authExpired" => "Sign-in expired",
        "health" => "Health issue",
        "healthRestored" => "Health restored",
        _ => "Event",
    }
}

async fn message_for(app: &App, item: &HistoryItem) -> Message {
    let mut account = None;
    if let Some(id) = &item.account_id {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT COALESCE(provider, 'audible'), account_name FROM core.Accounts WHERE account_id = ?")
                .bind(id)
                .fetch_optional(&app.db)
                .await
                .ok()
                .flatten();
        account = Some(match row {
            Some((provider, name)) => json!({ "id": id, "provider": provider, "name": name }),
            None => json!({ "id": id }),
        });
    }
    let mut book = None;
    if let Some(id) = &item.book_id {
        book = Some(match crate::books::get(&app.db, id).await.ok().flatten() {
            Some(b) => json!({
                "id": b.id, "title": b.title, "authors": b.authors, "series": b.series,
                "seriesSequence": b.series_sequence, "outputPath": b.output_path,
            }),
            None => json!({ "id": id, "title": item.title }),
        });
    }
    Message { event_type: item.event.clone(), message: item.message.clone(), account, book }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbsLibrary {
    pub id: String,
    pub name: String,
    pub folders: Vec<String>,
}

pub async fn abs_libraries(http: &reqwest::Client, url: &str, api_key: &str) -> Result<Vec<AbsLibrary>, String> {
    let base = url.trim_end_matches('/');
    let resp = http
        .get(format!("{base}/api/libraries"))
        .bearer_auth(api_key)
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Could not reach Audiobookshelf: {e}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err("Audiobookshelf rejected the API key".into());
    }
    if !status.is_success() {
        return Err(format!("Audiobookshelf answered HTTP {}", status.as_u16()));
    }
    let body: Value = resp.json().await.map_err(|e| format!("Unexpected Audiobookshelf response: {e}"))?;
    let libs = body.get("libraries").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(libs
        .iter()
        .filter_map(|l| {
            Some(AbsLibrary {
                id: l.get("id")?.as_str()?.to_string(),
                name: l.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                folders: l
                    .get("folders")
                    .and_then(Value::as_array)
                    .map(|f| {
                        f.iter()
                            .filter_map(|x| x.get("fullPath").and_then(Value::as_str).map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect())
}

async fn abs_scan(http: &reqwest::Client, n: &Notification) -> Result<String, String> {
    let c: AbsConfig = serde_json::from_value(n.config.clone()).map_err(|e| e.to_string())?;
    let base = c.url.trim_end_matches('/');
    let resp = http
        .post(format!("{base}/api/libraries/{}/scan", urlencoding::encode(&c.library_id)))
        .bearer_auth(&c.api_key)
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Could not reach Audiobookshelf: {e}"))?;
    let status = resp.status();
    if status.is_success() {
        Ok("Library scan started".into())
    } else if status == reqwest::StatusCode::FORBIDDEN {
        Err("Audiobookshelf refused the scan (the API key's user must be an admin)".into())
    } else {
        Err(format!("Audiobookshelf answered HTTP {}", status.as_u16()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_masked_and_restored() {
        let stored = json!({
            "url": "https://hooks.example/x", "method": "POST",
            "headers": [{"key": "Authorization", "value": "Bearer s3cret"}, {"key": "X-Env", "value": "prod"}],
            "username": "u", "password": "p"
        });
        let mut shown = stored.clone();
        mask(Kind::Webhook, &mut shown);
        assert_eq!(shown["password"], MASK);
        assert_eq!(shown["headers"][0]["value"], MASK);
        assert_eq!(shown["headers"][1]["value"], "prod");
        assert_eq!(shown["username"], "u");

        let mut back = shown.clone();
        unmask(Kind::Webhook, &mut back, &stored);
        assert_eq!(back, stored);

        let abs = json!({"url": "http://abs:80", "apiKey": "k", "libraryId": "l"});
        let mut shown = abs.clone();
        mask(Kind::Audiobookshelf, &mut shown);
        assert_eq!(shown["apiKey"], MASK);
        let mut changed = shown.clone();
        changed["apiKey"] = json!("new");
        unmask(Kind::Audiobookshelf, &mut changed, &abs);
        assert_eq!(changed["apiKey"], "new");
    }

    #[test]
    fn validation_normalises_and_rejects() {
        let mut n = Notification {
            id: 0,
            kind: Kind::Webhook,
            name: "Hook".into(),
            enabled: true,
            events: Some(vec!["download".into(), "health".into()]),
            config: json!({"url": "https://example.com/hook", "method": "put"}),
            last_result: None,
        };
        validate(&mut n).unwrap();
        assert_eq!(n.config["method"], "PUT");
        assert_eq!(n.config["headers"], json!([]));

        n.events = Some(vec!["nope".into()]);
        assert_eq!(validate(&mut n).unwrap_err().field.as_deref(), Some("events"));

        let mut n = Notification {
            kind: Kind::Apprise,
            events: None,
            config: json!({"serverUrl": "http://apprise:8000"}),
            ..n
        };
        assert!(validate(&mut n).is_err(), "needs a key or URLs");
        n.config = json!({"serverUrl": "ftp://apprise", "configKey": "k"});
        assert!(validate(&mut n).is_err());
        n.config = json!({"serverUrl": "http://apprise:8000", "configKey": "k"});
        validate(&mut n).unwrap();
    }
}
