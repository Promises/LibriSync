//! Audible licence pacing (DESIGN.md "Audible licence pacing"), as pure functions.
//!
//! Nothing here touches the database or the clock: callers pass `now` and the
//! account's recorded state, which keeps every rule unit-testable.
//!
//! | Rule | Applies to |
//! |---|---|
//! | Account cooldown after `license_throttled`: base (24 h) doubling per consecutive throttle, capped at 7 days | every priority; manual only with `force` |
//! | One licence request in flight per account (so the first after a cooldown is a single probe) | every priority |
//! | Rolling 24 h cap counting every request | every priority |
//! | Random spacing between requests | back-fill only |
//! | Local-time window | back-fill only (every provider) |
//! | Title backoff after `license_denied`: 1 day doubling, capped at 30 days | back-fill and new; manual bypasses |

use chrono::{DateTime, Duration, NaiveDateTime, NaiveTime, Utc};
use rand::Rng;

use crate::queue::Priority;

pub const MAX_COOLDOWN: Duration = Duration::days(7);
pub const TITLE_BACKOFF_BASE: Duration = Duration::days(1);
pub const MAX_TITLE_BACKOFF: Duration = Duration::days(30);
pub const CAP_WINDOW: Duration = Duration::hours(24);

/// The pacing settings that matter for one decision.
#[derive(Debug, Clone)]
pub struct PacingConfig {
    pub daily_cap: u32,
    pub spacing_min_minutes: u32,
    pub spacing_max_minutes: u32,
}

/// What is recorded about an Audible account's licence requests.
#[derive(Debug, Clone, Default)]
pub struct AccountPacing {
    pub cooldown_until: Option<DateTime<Utc>>,
    /// Consecutive throttles so far (0 = none). Reset by a granted licence.
    pub cooldown_level: u32,
    /// Earliest next back-fill request (spacing).
    pub next_licence_at: Option<DateTime<Utc>>,
    /// Request times within the last [`CAP_WINDOW`] (any order).
    pub recent_requests: Vec<DateTime<Utc>>,
    /// A licence request for this account is running right now.
    pub in_flight: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Gate {
    Go,
    Wait { until: Option<DateTime<Utc>>, reason: WaitReason },
}

#[derive(Debug, Clone, PartialEq)]
pub enum WaitReason {
    Cooldown(DateTime<Utc>),
    InFlight { probing: bool },
    Budget(DateTime<Utc>),
    Spacing(DateTime<Utc>),
}

impl WaitReason {
    /// The `waitingReason` string shown in the UI.
    pub fn describe(&self) -> String {
        use crate::time::local_label;
        match self {
            WaitReason::Cooldown(t) => format!("Account cooldown until {}", local_label(*t)),
            WaitReason::InFlight { probing: true } => {
                "Probing Audible after a cooldown; waiting for the result".into()
            }
            WaitReason::InFlight { probing: false } => {
                "Waiting for this account's current licence request".into()
            }
            WaitReason::Budget(t) => format!("Licence budget: next slot {}", local_label(*t)),
            WaitReason::Spacing(t) => format!("Licence spacing: next request {}", local_label(*t)),
        }
    }
}

/// May an item of `priority` make a licence request for this account now?
pub fn licence_gate(
    cfg: &PacingConfig,
    acct: &AccountPacing,
    priority: Priority,
    force: bool,
    now: DateTime<Utc>,
) -> Gate {
    if let Some(until) = acct.cooldown_until.filter(|u| *u > now) {
        if !(priority == Priority::Manual && force) {
            return Gate::Wait { until: Some(until), reason: WaitReason::Cooldown(until) };
        }
    }
    if acct.in_flight {
        return Gate::Wait {
            until: None,
            reason: WaitReason::InFlight { probing: acct.cooldown_level > 0 },
        };
    }
    if let Some(slot) = cap_next_slot(&acct.recent_requests, cfg.daily_cap, now) {
        return Gate::Wait { until: Some(slot), reason: WaitReason::Budget(slot) };
    }
    if priority == Priority::Backfill {
        if let Some(next) = acct.next_licence_at.filter(|n| *n > now) {
            return Gate::Wait { until: Some(next), reason: WaitReason::Spacing(next) };
        }
    }
    Gate::Go
}

/// When the rolling 24 h cap is full, the moment a slot frees up: the time at which
/// enough of the window's oldest requests have aged out. `None` = a slot is free now.
pub fn cap_next_slot(requests: &[DateTime<Utc>], cap: u32, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let window_start = now - CAP_WINDOW;
    let mut recent: Vec<DateTime<Utc>> = requests.iter().copied().filter(|t| *t > window_start).collect();
    let cap = cap as usize;
    if recent.len() < cap {
        return None;
    }
    recent.sort();
    // With n requests in the window and room for `cap`, n - cap + 1 must expire; the
    // last of those is at index n - cap.
    Some(recent[recent.len() - cap] + CAP_WINDOW)
}

/// Cooldown for the `level`-th consecutive throttle (0-based): base, doubling, capped.
pub fn cooldown_duration(base_hours: u32, level: u32) -> Duration {
    let base = Duration::hours(base_hours.max(1) as i64);
    doubling(base, level, MAX_COOLDOWN)
}

/// Apply a `license_throttled` refusal: returns `(cooldown_until, new_level)`.
pub fn after_throttle(base_hours: u32, level_before: u32, now: DateTime<Utc>) -> (DateTime<Utc>, u32) {
    (now + cooldown_duration(base_hours, level_before), level_before.saturating_add(1))
}

/// Title backoff for the `level`-th consecutive denial (0-based): 1 day, doubling, 30 days max.
pub fn title_backoff_duration(level: u32) -> Duration {
    doubling(TITLE_BACKOFF_BASE, level, MAX_TITLE_BACKOFF)
}

fn doubling(base: Duration, level: u32, cap: Duration) -> Duration {
    // 2^5 already exceeds every cap we use; avoid overflow for silly levels.
    let factor = 1i64 << level.min(16);
    let d = base * factor as i32;
    if d > cap {
        cap
    } else {
        d
    }
}

/// A random gap before the next back-fill licence request, uniform in [min, max] minutes.
pub fn pick_spacing<R: Rng>(rng: &mut R, min_minutes: u32, max_minutes: u32) -> Duration {
    let lo = min_minutes.min(max_minutes) as i64 * 60;
    let hi = max_minutes.max(min_minutes) as i64 * 60;
    Duration::seconds(rng.gen_range(lo..=hi))
}

/// Is local time `t` inside the window? A window whose end is before its start
/// crosses midnight (`22:00–06:00`).
pub fn in_window(start: NaiveTime, end: NaiveTime, t: NaiveTime) -> bool {
    if start < end {
        start <= t && t < end
    } else {
        t >= start || t < end
    }
}

/// The next local moment the window opens after `now` (which is outside it).
pub fn next_window_open(start: NaiveTime, now: NaiveDateTime) -> NaiveDateTime {
    let today = now.date().and_time(start);
    if today > now {
        today
    } else {
        today + Duration::days(1)
    }
}

/// Minutes per day the window is open (1440 without a window).
pub fn window_minutes(window: Option<(NaiveTime, NaiveTime)>) -> i64 {
    match window {
        None => 1440,
        Some((s, e)) => {
            let m = (e - s).num_minutes();
            if m > 0 {
                m
            } else {
                m + 1440
            }
        }
    }
}

/// Expected Audible back-fill licences per day for one account: the spacing allows
/// `open minutes / mean spacing`, the cap allows `daily_cap`.
pub fn backfill_per_day(cfg: &PacingConfig, window: Option<(NaiveTime, NaiveTime)>) -> f64 {
    let mean = (cfg.spacing_min_minutes + cfg.spacing_max_minutes) as f64 / 2.0;
    let by_spacing = if mean > 0.0 { window_minutes(window) as f64 / mean } else { f64::INFINITY };
    by_spacing.min(cfg.daily_cap as f64)
}

/// Wait before retrying an ordinary failure (network, disk, ffmpeg), given how many
/// attempts have failed so far. Three retries (5 min, 30 min, 2 h), then give up.
pub fn retry_delay(failed_attempts: u32) -> Option<Duration> {
    match failed_attempts {
        1 => Some(Duration::minutes(5)),
        2 => Some(Duration::minutes(30)),
        3 => Some(Duration::hours(2)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn t(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 25, h, m, 0).unwrap()
    }

    fn cfg() -> PacingConfig {
        PacingConfig {
            daily_cap: 3,
            spacing_min_minutes: 20,
            spacing_max_minutes: 60,
        }
    }

    fn hm(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn free_account_goes() {
        let acct = AccountPacing::default();
        for p in [Priority::Manual, Priority::New, Priority::Backfill] {
            assert_eq!(licence_gate(&cfg(), &acct, p, false, t(12, 0)), Gate::Go);
        }
    }

    #[test]
    fn rolling_cap_counts_only_the_last_24_hours() {
        let now = t(12, 0);
        let reqs = vec![now - Duration::hours(30), now - Duration::hours(23), now - Duration::hours(2)];
        assert_eq!(cap_next_slot(&reqs, 3, now), None, "the 30 h old one is out of the window");

        let reqs = vec![now - Duration::hours(23), now - Duration::hours(5), now - Duration::hours(2)];
        assert_eq!(cap_next_slot(&reqs, 3, now), Some(now + Duration::hours(1)));

        // Over the cap (e.g. after lowering it): wait for enough to expire.
        let reqs = vec![now - Duration::hours(20), now - Duration::hours(10), now - Duration::hours(1)];
        assert_eq!(cap_next_slot(&reqs, 2, now), Some(now + Duration::hours(14)));
    }

    #[test]
    fn cap_applies_to_every_priority_including_manual() {
        let now = t(12, 0);
        let acct = AccountPacing {
            recent_requests: vec![now - Duration::hours(3); 3],
            ..Default::default()
        };
        for p in [Priority::Manual, Priority::New, Priority::Backfill] {
            match licence_gate(&cfg(), &acct, p, true, now) {
                Gate::Wait { reason: WaitReason::Budget(slot), .. } => {
                    assert_eq!(slot, now + Duration::hours(21))
                }
                other => panic!("{p:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn spacing_only_holds_backfill() {
        let now = t(12, 0);
        let acct = AccountPacing { next_licence_at: Some(now + Duration::minutes(10)), ..Default::default() };
        assert_eq!(licence_gate(&cfg(), &acct, Priority::Manual, false, now), Gate::Go);
        assert_eq!(licence_gate(&cfg(), &acct, Priority::New, false, now), Gate::Go);
        assert!(matches!(
            licence_gate(&cfg(), &acct, Priority::Backfill, false, now),
            Gate::Wait { reason: WaitReason::Spacing(_), .. }
        ));
        // Once the spacing has passed, back-fill goes.
        assert_eq!(licence_gate(&cfg(), &acct, Priority::Backfill, false, now + Duration::minutes(11)), Gate::Go);
    }

    #[test]
    fn cooldown_blocks_everything_but_forced_manual() {
        let now = t(12, 0);
        let until = now + Duration::hours(5);
        let acct = AccountPacing { cooldown_until: Some(until), cooldown_level: 1, ..Default::default() };
        for (p, force) in [(Priority::Backfill, true), (Priority::New, true), (Priority::Manual, false)] {
            assert_eq!(
                licence_gate(&cfg(), &acct, p, force, now),
                Gate::Wait { until: Some(until), reason: WaitReason::Cooldown(until) }
            );
        }
        assert_eq!(licence_gate(&cfg(), &acct, Priority::Manual, true, now), Gate::Go);
    }

    #[test]
    fn after_cooldown_only_one_probe_is_in_flight() {
        let now = t(12, 0);
        let acct = AccountPacing {
            cooldown_until: Some(now - Duration::minutes(1)),
            cooldown_level: 2,
            in_flight: true,
            ..Default::default()
        };
        assert_eq!(
            licence_gate(&cfg(), &acct, Priority::Manual, false, now),
            Gate::Wait { until: None, reason: WaitReason::InFlight { probing: true } }
        );
        let idle = AccountPacing { in_flight: false, ..acct };
        assert_eq!(licence_gate(&cfg(), &idle, Priority::New, false, now), Gate::Go);
    }

    #[test]
    fn cooldown_doubles_and_caps_at_seven_days() {
        assert_eq!(cooldown_duration(24, 0), Duration::hours(24));
        assert_eq!(cooldown_duration(24, 1), Duration::hours(48));
        assert_eq!(cooldown_duration(24, 2), Duration::hours(96));
        assert_eq!(cooldown_duration(24, 3), Duration::days(7));
        assert_eq!(cooldown_duration(24, 40), Duration::days(7));
        assert_eq!(cooldown_duration(6, 1), Duration::hours(12));

        let now = t(0, 0);
        let (until, level) = after_throttle(24, 0, now);
        assert_eq!((until, level), (now + Duration::hours(24), 1));
        let (until, level) = after_throttle(24, level, now);
        assert_eq!((until, level), (now + Duration::hours(48), 2));
    }

    #[test]
    fn title_backoff_doubles_and_caps_at_thirty_days() {
        assert_eq!(title_backoff_duration(0), Duration::days(1));
        assert_eq!(title_backoff_duration(1), Duration::days(2));
        assert_eq!(title_backoff_duration(4), Duration::days(16));
        assert_eq!(title_backoff_duration(5), Duration::days(30));
        assert_eq!(title_backoff_duration(99), Duration::days(30));
    }

    #[test]
    fn spacing_is_within_bounds() {
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..1000 {
            let d = pick_spacing(&mut rng, 20, 60);
            assert!(d >= Duration::minutes(20) && d <= Duration::minutes(60));
        }
        assert_eq!(pick_spacing(&mut rng, 30, 30), Duration::minutes(30));
    }

    #[test]
    fn windows_including_ones_crossing_midnight() {
        assert!(in_window(hm(1, 0), hm(7, 0), hm(1, 0)));
        assert!(in_window(hm(1, 0), hm(7, 0), hm(6, 59)));
        assert!(!in_window(hm(1, 0), hm(7, 0), hm(7, 0)));
        assert!(!in_window(hm(1, 0), hm(7, 0), hm(0, 30)));

        assert!(in_window(hm(22, 0), hm(6, 0), hm(23, 0)));
        assert!(in_window(hm(22, 0), hm(6, 0), hm(2, 0)));
        assert!(!in_window(hm(22, 0), hm(6, 0), hm(12, 0)));
        assert!(!in_window(hm(22, 0), hm(6, 0), hm(6, 0)));

        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();
        assert_eq!(next_window_open(hm(22, 0), day.and_time(hm(12, 0))), day.and_time(hm(22, 0)));
        assert_eq!(
            next_window_open(hm(1, 0), day.and_time(hm(12, 0))),
            day.succ_opt().unwrap().and_time(hm(1, 0))
        );

        assert_eq!(window_minutes(None), 1440);
        assert_eq!(window_minutes(Some((hm(1, 0), hm(7, 0)))), 360);
        assert_eq!(window_minutes(Some((hm(22, 0), hm(6, 0)))), 480);
    }

    #[test]
    fn backfill_rate_is_the_tighter_of_cap_and_spacing() {
        let mut c = cfg();
        c.daily_cap = 20;
        // Mean spacing 40 min → 36/day by spacing; cap 20 wins.
        assert_eq!(backfill_per_day(&c, None), 20.0);
        // A 6 h window allows 9 by spacing.
        assert_eq!(backfill_per_day(&c, Some((hm(1, 0), hm(7, 0)))), 9.0);
    }

    #[test]
    fn retries_three_times_then_fails() {
        assert_eq!(retry_delay(1), Some(Duration::minutes(5)));
        assert_eq!(retry_delay(2), Some(Duration::minutes(30)));
        assert_eq!(retry_delay(3), Some(Duration::hours(2)));
        assert_eq!(retry_delay(4), None);
    }
}
