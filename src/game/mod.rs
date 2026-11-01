//! Live game context: what a client's screen shows right now, as probes the
//! input arbiter can wait on. Shared by the Movement tester and the game loop.

pub mod cards;
pub mod placing;
pub mod run;
#[cfg(test)]
pub mod sim;

use crate::geom::{Hwnd, Point, Rect};
use crate::input::Probe;
use crate::movement::RouteEnv;
use crate::platform::Platform;
use crate::vision::{key, Library, Pattern};
use std::collections::HashMap;
use std::sync::Arc;

/// One client's screen + a snapshot of your patterns.
#[derive(Clone)]
pub struct LiveScreen {
    platform: Arc<dyn Platform>,
    hwnd: Hwnd,
    pats: Arc<HashMap<String, Pattern>>,
}

impl LiveScreen {
    pub fn new(platform: Arc<dyn Platform>, hwnd: Hwnd, lib: &Library) -> Self {
        let pats = lib.patterns().map(|p| (key(&p.name), p.clone())).collect();
        Self {
            platform,
            hwnd,
            pats: Arc::new(pats),
        }
    }

    /// Centre of the first of `names` found now (inside `region` if given).
    pub fn find(&self, names: &[String], region: Option<Rect>) -> Option<Point> {
        let frame = self.platform.capture_client(self.hwnd).ok()?;
        names.iter().find_map(|n| {
            self.pats
                .get(&key(n))
                .and_then(|p| p.find_in(&frame, region))
                .map(|h| h.center())
        })
    }

    /// Is `name` in an already captured picture (no new capture)?
    pub fn find_in_frame(&self, frame: &crate::capture::Frame, name: &str) -> bool {
        self.pats
            .get(&key(name))
            .and_then(|p| p.find_in(frame, None))
            .is_some()
    }

    /// The client's picture right now.
    pub fn frame(&self) -> Option<crate::capture::Frame> {
        self.platform.capture_client(self.hwnd).ok()
    }

    /// Captured pattern names (normalised) starting with `prefix`.
    pub fn with_prefix(&self, prefix: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .pats
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        v.sort();
        v
    }

    pub fn has(&self, name: &str) -> bool {
        self.pats.contains_key(&key(name))
    }

    /// Names in `names` that you haven't captured yet.
    pub fn missing(&self, names: &[String]) -> Vec<String> {
        names
            .iter()
            .filter(|n| !self.pats.contains_key(&key(n)))
            .cloned()
            .collect()
    }
}

impl RouteEnv for LiveScreen {
    fn probe(&self, patterns: &[String], region: Option<Rect>) -> Probe {
        let me = self.clone();
        let names = patterns.to_vec();
        Probe::new(patterns.join(" | "), move || me.find(&names, region))
    }
}

/// Never sees anything: used to check a route's syntax.
pub struct Blind;

impl RouteEnv for Blind {
    fn probe(&self, patterns: &[String], _region: Option<Rect>) -> Probe {
        Probe::new(patterns.join(" | "), || None)
    }
}
