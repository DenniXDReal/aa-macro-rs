//! Challenge every 30 min: Anime Adventures' Challenge resets at :00 and
//! :30 of your PC's local time. With the toggle on, the round that ends
//! after a reset goes back to the lobby, one Challenge run is played, and
//! the farm carries on. Once per reset window; never mid-stage.

use chrono::{DateTime, Local, TimeZone, Timelike};
use std::sync::Mutex;

/// The reset window `t` is in: half hours of local time since 1970.
pub fn window<Tz: TimeZone>(t: &DateTime<Tz>) -> i64 {
    let local_minutes = t.naive_local().and_utc().timestamp() / 60;
    local_minutes.div_euclid(30)
}

/// Minutes until the next reset (:00 / :30) after `t`.
pub fn minutes_to_next<Tz: TimeZone>(t: &DateTime<Tz>) -> u32 {
    let m = t.minute() % 30;
    if m == 0 && t.second() == 0 {
        30
    } else {
        30 - m
    }
}

/// Last window a Challenge was played in (None = none yet this run).
static DONE: Mutex<Option<i64>> = Mutex::new(None);

/// A Challenge is owed: a reset happened since the last one we played (or
/// since the start).
pub fn due_at(now_window: i64, done: Option<i64>) -> bool {
    done.is_none_or(|d| now_window > d)
}

pub fn due() -> bool {
    due_at(
        window(&Local::now()),
        *DONE.lock().unwrap_or_else(|e| e.into_inner()),
    )
}

/// This window's Challenge is taken care of (played, or tried).
pub fn mark_done() {
    *DONE.lock().unwrap_or_else(|e| e.into_inner()) = Some(window(&Local::now()));
}

/// A new start: the first Challenge is at the next reset (:00 / :30),
/// not right away.
pub fn reset() {
    mark_done();
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at(h: u32, m: u32, s: u32) -> DateTime<FixedOffset> {
        // Singapore time
        FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 11, 7, h, m, s)
            .unwrap()
    }

    #[test]
    fn windows_turn_over_at_00_and_30_local() {
        assert_eq!(window(&at(4, 0, 0)), window(&at(4, 29, 59)));
        assert_eq!(window(&at(4, 30, 0)), window(&at(4, 0, 0)) + 1);
        assert_eq!(window(&at(5, 0, 0)), window(&at(4, 30, 0)) + 1);
        // local time, not UTC: 04:30 SGT is 20:30 UTC
        let utc = chrono::Utc
            .with_ymd_and_hms(2026, 11, 6, 20, 30, 0)
            .unwrap();
        assert_ne!(
            window(&utc),
            window(&at(4, 30, 0)),
            "uses the clock you see"
        );
        assert_eq!(minutes_to_next(&at(4, 12, 0)), 18);
        assert_eq!(minutes_to_next(&at(4, 30, 0)), 30);
        assert_eq!(minutes_to_next(&at(4, 59, 10)), 1);
    }

    #[test]
    fn once_per_window() {
        let w = window(&at(4, 10, 0));
        assert!(due_at(w, None), "owed at the start");
        assert!(!due_at(w, Some(w)), "played this window");
        assert!(due_at(w + 1, Some(w)), "next reset");
        assert!(due_at(w + 5, Some(w)), "several missed = one owed");
    }
}
