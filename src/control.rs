//! Stop / pause shared by every worker and the input arbiter.
//!
//! Every loop calls [`Control::checkpoint`] or [`Control::sleep`]. That is what
//! makes Stop immediate and Pause safe. Deadlines ignore time spent paused.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopped;

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("stopped")
    }
}
impl std::error::Error for Stopped {}

#[derive(Default)]
struct Inner {
    stop: AtomicBool,
    paused: AtomicBool,
}

/// Cheap to clone; all clones share the same flags.
#[derive(Clone, Default)]
pub struct Control(Arc<Inner>);

const SLICE: Duration = Duration::from_millis(10);

impl Control {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn stop(&self) {
        self.0.stop.store(true, Ordering::SeqCst);
    }
    pub fn reset(&self) {
        self.0.stop.store(false, Ordering::SeqCst);
        self.0.paused.store(false, Ordering::SeqCst);
    }
    pub fn is_stopped(&self) -> bool {
        self.0.stop.load(Ordering::SeqCst)
    }
    pub fn set_paused(&self, p: bool) {
        self.0.paused.store(p, Ordering::SeqCst);
    }
    pub fn toggle_pause(&self) -> bool {
        let now = !self.is_paused();
        self.set_paused(now);
        now
    }
    pub fn is_paused(&self) -> bool {
        self.0.paused.load(Ordering::SeqCst)
    }

    /// Returns `Err(Stopped)` on stop; blocks while paused.
    pub fn checkpoint(&self) -> Result<(), Stopped> {
        loop {
            if self.is_stopped() {
                return Err(Stopped);
            }
            if !self.is_paused() {
                return Ok(());
            }
            std::thread::sleep(SLICE);
        }
    }

    /// Sleep `ms` of *unpaused* time, in small slices.
    pub fn sleep(&self, ms: u64) -> Result<(), Stopped> {
        let mut left = Duration::from_millis(ms);
        while !left.is_zero() {
            self.checkpoint()?;
            let step = left.min(SLICE);
            let t = Instant::now();
            std::thread::sleep(step);
            if !self.is_paused() {
                left = left.saturating_sub(t.elapsed().min(step));
            }
        }
        self.checkpoint()
    }

    pub fn deadline(&self, seconds: f64) -> Deadline {
        Deadline {
            ctl: self.clone(),
            budget: Duration::from_secs_f64(seconds.max(0.0)),
            spent: Duration::ZERO,
            last: Instant::now(),
            never: seconds <= 0.0,
        }
    }
}

/// A timeout that ignores time spent paused. `seconds <= 0` never expires.
pub struct Deadline {
    ctl: Control,
    budget: Duration,
    spent: Duration,
    last: Instant,
    never: bool,
}

impl Deadline {
    pub fn expired(&mut self) -> bool {
        let now = Instant::now();
        if !self.ctl.is_paused() {
            self.spent += now - self.last;
        }
        self.last = now;
        !self.never && self.spent >= self.budget
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_interrupts_sleep() {
        let c = Control::new();
        let c2 = c.clone();
        let h = std::thread::spawn(move || c2.sleep(5_000));
        std::thread::sleep(Duration::from_millis(30));
        c.stop();
        assert_eq!(h.join().unwrap(), Err(Stopped));
    }

    #[test]
    fn pause_extends_sleep() {
        let c = Control::new();
        c.set_paused(true);
        let c2 = c.clone();
        let t = Instant::now();
        let h = std::thread::spawn(move || c2.sleep(40));
        std::thread::sleep(Duration::from_millis(80));
        c.set_paused(false);
        h.join().unwrap().unwrap();
        assert!(t.elapsed() >= Duration::from_millis(115));
    }

    #[test]
    fn zero_deadline_never_expires() {
        let c = Control::new();
        let mut d = c.deadline(0.0);
        std::thread::sleep(Duration::from_millis(5));
        assert!(!d.expired());
        let mut d2 = c.deadline(0.001);
        std::thread::sleep(Duration::from_millis(5));
        assert!(d2.expired());
    }
}
