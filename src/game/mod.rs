//! Live game context: what a client's screen shows right now, as probes the
//! input arbiter can wait on. Shared by the Movement tester and the game loop.

pub mod cards;
pub mod placing;
pub mod run;
#[cfg(test)]
pub mod sim;

#[cfg(test)]
mod level_tests {
    use super::parse_level;

    #[test]
    fn reads_levels() {
        assert_eq!(parse_level("UPG [3]"), Some(Ok(3)));
        assert_eq!(parse_level("UPG[0]"), Some(Ok(0)));
        assert_eq!(parse_level("upg [O]"), Some(Ok(0)));
        assert_eq!(parse_level("UPG 13]"), Some(Ok(3)));
        assert_eq!(parse_level("UPG [10]"), Some(Ok(10)));
        assert_eq!(parse_level("UPG 5"), Some(Ok(5)));
        assert_eq!(parse_level("MAX"), Some(Err(())));
        assert_eq!(parse_level("UPG [MAX]"), Some(Err(())));
        assert_eq!(parse_level(""), None);
    }
}

use crate::geom::{Hwnd, Point, Rect};
use crate::input::Probe;
use crate::movement::RouteEnv;
use crate::platform::Platform;
use crate::vision::{key, Library, Pattern};
use std::collections::HashMap;
use std::sync::Arc;

/// "UPG [3]" -> Ok(3), "MAX" -> Err(()), junk -> None. Tolerates the
/// usual OCR slips ("[" read as "1", "O" for 0, missing brackets).
pub fn parse_level(text: &str) -> Option<Result<u32, ()>> {
    let t = text.to_uppercase();
    if t.contains("MAX") {
        return Some(Err(()));
    }
    let t: String = t
        .chars()
        .map(|c| match c {
            'O' | 'Q' | 'D' => '0',
            'I' | 'L' | '|' | '!' => '1',
            'S' => '5',
            'B' => '8',
            c => c,
        })
        .collect();
    // text after "UPG" (the letters U, P, G survive the mapping above)
    let after = t.find("UPG").map(|i| &t[i + 3..]).unwrap_or(&t);
    let num = |s: &str| -> Option<u32> {
        let d: String = s
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        d.parse().ok()
    };
    if let Some(i) = after.find('[') {
        return num(&after[i + 1..]).map(Ok);
    }
    let n = num(after)?;
    // "[3]" read as "13]": no '[' but a ']' - the leading 1 was the bracket
    let digits = n.to_string();
    if after.contains(']') && digits.len() >= 2 && digits.starts_with('1') {
        return digits[1..].parse().ok().map(Ok);
    }
    Some(Ok(n))
}

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

    /// Where the unit panel's level text sits: around your `unit_placed`
    /// capture, else the AHK macro's spot.
    pub fn level_area(&self) -> Rect {
        match self.pats.get(&key("unit_placed")) {
            Some(p) => {
                let c = p.captured_at;
                Rect::new(c.x - 30, c.y - 6, c.w + 60, c.h + 12)
            }
            None => Rect::new(170, 228, 142, 24),
        }
    }

    /// OCR of the unit panel's level: Some(Ok(n)) = "UPG [n]",
    /// Some(Err(())) = MAX, None = couldn't read it.
    pub fn ocr_level(&self, frame: &crate::capture::Frame) -> Option<Result<u32, ()>> {
        let text = self.platform.ocr(frame, self.level_area())?;
        parse_level(&text)
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

    /// Like `find`, but over the whole picture (the button may sit somewhere
    /// else than where you captured it, e.g. Next on a DEFEAT vs VICTORY screen).
    pub fn find_wide(&self, names: &[String]) -> Option<Point> {
        let frame = self.platform.capture_client(self.hwnd).ok()?;
        let all = Rect::new(0, 0, frame.width as i32, frame.height as i32);
        names.iter().find_map(|n| {
            self.pats.get(&key(n)).and_then(|p| {
                p.find_in(&frame, None)
                    .or_else(|| p.find_in(&frame, Some(all)))
                    .map(|h| h.center())
            })
        })
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
