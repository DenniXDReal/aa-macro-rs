//! Recovery watchdog (Phase 1): a background thread that looks at every
//! client every few seconds for Roblox's disconnect box and for a captcha,
//! and posts what it sees here. It never clicks.
//!
//! - Disconnect: `RunCtx::check()` / the step machine's abort check see it
//!   and stop the step; the loop then runs the reconnect recovery
//!   (`RunCtx::recover_disconnect`): Reconnect > back in the lobby > party >
//!   next round. The lost stage is an aborted run (no Discord report).
//! - Captcha: the watchdog pauses the macro, pings Discord and shows it on
//!   the card; once the captcha is gone (you typed it) it un-pauses. The
//!   macro never solves it.

use crate::geom::Hwnd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How often every client is looked at.
pub const EVERY: Duration = Duration::from_secs(5);
/// OCR (the slower check, for when there's no picture) every Nth look.
pub const OCR_EVERY_NTH: u32 = 2;
/// How long the reconnect may take to get a client back to the lobby.
pub const RECONNECT_WAIT_SECS: f64 = 90.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Disconnected,
    Captcha,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Disconnected => "DISCONNECTED",
            Kind::Captcha => "CAPTCHA",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Incident {
    pub hwnd: Hwnd,
    /// 1-based client number.
    pub client: usize,
    pub kind: Kind,
    pub since: Instant,
}

/// Captcha by its words (when you haven't captured a `captcha` picture).
pub fn captcha_text(t: &str) -> bool {
    let t = t.to_lowercase();
    // the code itself is random: only the dialog's fixed words count
    t.contains("captcha")
        || t.contains("verify")
        || t.contains("verification")
        || t.contains("are you a robot")
}

/// What one look at a client found: the disconnect box beats a captcha.
pub fn classify(disconnect_pic: bool, captcha_pic: bool, ocr: Option<&str>) -> Option<Kind> {
    if disconnect_pic || ocr.is_some_and(super::run::disconnect_text) {
        Some(Kind::Disconnected)
    } else if captcha_pic || ocr.is_some_and(captcha_text) {
        Some(Kind::Captcha)
    } else {
        None
    }
}

/// Shared between the watchdog thread and the loop.
#[derive(Default, Debug)]
pub struct Watch {
    now: Mutex<Vec<Incident>>,
    /// The watchdog paused the macro for a captcha (and un-pauses it).
    pub captcha_paused: AtomicBool,
    /// A reconnect is running: `check()` doesn't stop it for the very
    /// disconnect it's fixing.
    pub recovering: AtomicBool,
    /// When reconnects happened (for the per-hour limit).
    reconnects: Mutex<Vec<Instant>>,
}

impl Watch {
    pub fn incidents(&self) -> Vec<Incident> {
        self.now.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    /// Replace what client `hwnd` shows (None = all clear), keeping `since`
    /// for an incident that was already there. Returns true when it's new.
    pub fn set(&self, hwnd: Hwnd, client: usize, kind: Option<Kind>) -> bool {
        let mut v = self.now.lock().unwrap_or_else(|e| e.into_inner());
        let old = v.iter().find(|i| i.hwnd == hwnd).cloned();
        v.retain(|i| i.hwnd != hwnd);
        match kind {
            Some(k) => {
                let since = old
                    .as_ref()
                    .filter(|o| o.kind == k)
                    .map(|o| o.since)
                    .unwrap_or_else(Instant::now);
                v.push(Incident {
                    hwnd,
                    client,
                    kind: k,
                    since,
                });
                old.is_none_or(|o| o.kind != k)
            }
            None => false,
        }
    }
    pub fn clear(&self) {
        self.now.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    pub fn first(&self, kind: Kind) -> Option<Incident> {
        self.incidents().into_iter().find(|i| i.kind == kind)
    }
    /// A disconnect that should stop the step (not while reconnecting).
    pub fn stopping_disconnect(&self) -> Option<Incident> {
        if self.recovering.load(Ordering::Relaxed) {
            None
        } else {
            self.first(Kind::Disconnected)
        }
    }
    /// Count a reconnect; false = already `max` in the last hour.
    pub fn allow_reconnect(&self, max: u32) -> bool {
        let mut v = self.reconnects.lock().unwrap_or_else(|e| e.into_inner());
        v.retain(|t| t.elapsed() < Duration::from_secs(3600));
        if v.len() as u32 >= max {
            return false;
        }
        v.push(Instant::now());
        true
    }
}

/// While alive, `check()` lets a reconnect run.
pub struct Recovering<'a>(&'a Watch);
impl<'a> Recovering<'a> {
    pub fn new(w: &'a Watch) -> Self {
        w.recovering.store(true, Ordering::Relaxed);
        Recovering(w)
    }
}
impl Drop for Recovering<'_> {
    fn drop(&mut self) {
        self.0.recovering.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_look_finds() {
        assert_eq!(classify(true, false, None), Some(Kind::Disconnected));
        assert_eq!(classify(false, true, None), Some(Kind::Captcha));
        assert_eq!(classify(true, true, None), Some(Kind::Disconnected));
        assert_eq!(
            classify(
                false,
                false,
                Some("Disconnected\nLost connection to the game server\nReconnect Leave")
            ),
            Some(Kind::Disconnected)
        );
        assert_eq!(
            classify(false, false, Some("Error Code: 277")),
            Some(Kind::Disconnected)
        );
        assert_eq!(
            classify(false, false, Some("Please complete the captcha")),
            Some(Kind::Captcha)
        );
        assert_eq!(
            classify(false, false, Some("Verify  X7kQ2  Submit")),
            Some(Kind::Captcha)
        );
        assert_eq!(classify(false, false, Some("VICTORY  +7291")), None);
        assert_eq!(classify(false, false, Some("Wave 17 Navy Bay")), None);
        assert_eq!(classify(false, false, None), None);
    }

    #[test]
    fn incidents_per_client() {
        let w = Watch::default();
        let (a, b) = (Hwnd(1), Hwnd(2));
        assert!(w.set(a, 1, Some(Kind::Captcha)), "new");
        assert!(!w.set(a, 1, Some(Kind::Captcha)), "same one again");
        assert!(w.set(b, 2, Some(Kind::Disconnected)));
        assert_eq!(w.first(Kind::Disconnected).map(|i| i.client), Some(2));
        assert!(w.stopping_disconnect().is_some());
        {
            let _r = Recovering::new(&w);
            assert!(w.stopping_disconnect().is_none(), "not while reconnecting");
        }
        assert!(w.stopping_disconnect().is_some());
        w.set(b, 2, None);
        assert!(w.first(Kind::Disconnected).is_none());
        assert_eq!(w.incidents().len(), 1);
    }

    #[test]
    fn reconnects_per_hour() {
        let w = Watch::default();
        assert!(w.allow_reconnect(2));
        assert!(w.allow_reconnect(2));
        assert!(!w.allow_reconnect(2));
    }
}
