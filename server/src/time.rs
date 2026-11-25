//! Timestamp helpers.
//!
//! Server-side timestamps are stored as fixed-width RFC 3339 UTC strings
//! (`2026-11-25T21:51:06.123Z`) so that SQL string comparison orders them correctly.
//! The core DB uses SQLite's `CURRENT_TIMESTAMP` form (`2026-11-25 21:51:06`, UTC).

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc};

pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// Canonical stored/API form.
pub fn fmt(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn now_str() -> String {
    fmt(now())
}

/// Parse a stored timestamp. Accepts RFC 3339 and the core DB's `YYYY-MM-DD HH:MM:SS`
/// (UTC). Anything else is `None`.
pub fn parse(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    for f in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, f) {
            return Some(Utc.from_utc_datetime(&n));
        }
    }
    None
}

/// Normalise a core DB timestamp or date for the API: full timestamps become RFC 3339,
/// bare dates stay `YYYY-MM-DD`, and anything unrecognised is passed through.
pub fn api_from_core(s: &str) -> String {
    if let Some(dt) = parse(s) {
        return fmt(dt);
    }
    if NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() {
        return s.to_string();
    }
    s.to_string()
}

/// The core DB's `CURRENT_TIMESTAMP` form, for comparing against `Books.created_at`.
pub fn core_fmt(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Short local-time label for a moment, as shown in waiting reasons: `14:20` today,
/// `Thu 14:20` within a week, otherwise the date.
pub fn local_label(dt: DateTime<Utc>) -> String {
    let local = dt.with_timezone(&Local);
    let today = Local::now().date_naive();
    let days = (local.date_naive() - today).num_days();
    if days == 0 {
        local.format("%H:%M").to_string()
    } else if (1..7).contains(&days) {
        local.format("%a %H:%M").to_string()
    } else {
        local.format("%Y-%m-%d %H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_form_is_fixed_width_and_round_trips() {
        let dt = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        let s = fmt(dt);
        assert_eq!(s, "2026-01-02T03:04:05.000Z");
        assert_eq!(parse(&s), Some(dt));
    }

    #[test]
    fn parses_core_timestamps_as_utc() {
        let dt = parse("2026-11-25 21:51:06").unwrap();
        assert_eq!(fmt(dt), "2026-11-25T21:51:06.000Z");
        assert_eq!(api_from_core("2026-11-25"), "2026-11-25");
    }
}
