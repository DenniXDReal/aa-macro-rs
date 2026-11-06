//! Live macro state for the dashboard: the current step and per-client
//! retry attempts. Written by the macro threads, read by the UI every frame
//! through `ArcSwap` (last value wins; the UI never blocks the macro).
//! Per-client input generations come from `input::generation`.

use crate::geom::Hwnd;
use arc_swap::ArcSwap;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Attempt {
    pub hwnd: Hwnd,
    /// 1-based try number.
    pub n: u32,
    pub of: u32,
    pub at: Instant,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    /// Step of the round loop ("lobby", "place", "upgrade", ...); empty
    /// before the first run.
    pub step: String,
    pub step_since: Option<Instant>,
    /// The latest retry per client (cleared when it went through).
    pub attempts: Vec<Attempt>,
    /// Watchdog findings per client ("CAPTCHA", "DISCONNECTED").
    pub incidents: Vec<(Hwnd, &'static str)>,
}

impl Status {
    pub fn attempt(&self, h: Hwnd) -> Option<&Attempt> {
        self.attempts.iter().find(|a| a.hwnd == h)
    }
    pub fn incident(&self, h: Hwnd) -> Option<&'static str> {
        self.incidents
            .iter()
            .find(|(c, _)| *c == h)
            .map(|(_, w)| *w)
    }
    fn with_step(&self, name: &str) -> Status {
        Status {
            step: name.to_string(),
            step_since: Some(Instant::now()),
            // a new step starts with a clean slate
            attempts: vec![],
            incidents: self.incidents.clone(),
        }
    }
    fn with_attempt(&self, h: Hwnd, n: u32, of: u32) -> Status {
        let mut s = self.clone();
        s.attempts.retain(|a| a.hwnd != h);
        s.attempts.push(Attempt {
            hwnd: h,
            n,
            of,
            at: Instant::now(),
        });
        s
    }
    fn without_attempt(&self, h: Hwnd) -> Status {
        let mut s = self.clone();
        s.attempts.retain(|a| a.hwnd != h);
        s
    }
}

static NOW: LazyLock<ArcSwap<Status>> = LazyLock::new(|| ArcSwap::from_pointee(Status::default()));

pub fn snapshot() -> Arc<Status> {
    NOW.load_full()
}

/// The round loop moved on to `name` (called by `game::trace::step`).
pub fn set_step(name: &str) {
    NOW.rcu(|s| s.with_step(name));
}

/// A new run: no step yet, no attempts (called on Start).
pub fn reset() {
    NOW.store(Arc::new(Status::default()));
}

/// What the watchdog sees on client `h` (None = all clear).
pub fn set_incident(h: Hwnd, what: Option<&'static str>) {
    if snapshot().incident(h) == what {
        return;
    }
    NOW.rcu(|s| {
        let mut s = (**s).clone();
        s.incidents.retain(|(c, _)| *c != h);
        if let Some(w) = what {
            s.incidents.push((h, w));
        }
        s
    });
}

/// Client `h` is on try `n` of `of` of something (n > 1 = retrying).
pub fn attempt(h: Hwnd, n: u32, of: u32) {
    NOW.rcu(|s| s.with_attempt(h, n, of));
}

/// What client `h` was retrying went through.
pub fn clear_attempt(h: Hwnd) {
    if snapshot().attempt(h).is_none() {
        return;
    }
    NOW.rcu(|s| s.without_attempt(h));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempts_per_client_and_reset_by_step() {
        let (a, b) = (Hwnd(1), Hwnd(2));
        let s = Status::default()
            .with_attempt(a, 2, 4)
            .with_attempt(b, 1, 4)
            .with_attempt(a, 3, 4);
        assert_eq!(s.attempt(a).map(|x| x.n), Some(3));
        assert_eq!(s.attempt(b).map(|x| x.n), Some(1));
        let s = s.without_attempt(b);
        assert!(s.attempt(b).is_none());
        let s = s.with_step("upgrade");
        assert_eq!(s.step, "upgrade");
        assert!(s.attempt(a).is_none());
    }
}
