//! Live game context: what a client's screen shows right now, as probes the
//! input arbiter can wait on. Shared by the Movement tester and the game loop.

pub mod cardlog;
pub mod cards;
pub mod minimap;
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
        use super::card_screen_text;
        assert_eq!(
            card_screen_text("CHOOSE A CARD Weather: Rainy Tier 1"),
            Some("Rainy".into())
        );
        assert_eq!(card_screen_text("Choose a Card"), Some(String::new()));
        assert_eq!(card_screen_text("Wave 23 DEFEAT"), None);
        use super::weather_shift_text;
        assert_eq!(
            weather_shift_text("WEATHER SHIFT Sunny"),
            Some("sunny".into())
        );
        assert_eq!(
            weather_shift_text("Weather changed to Rainy!"),
            Some("rainy".into())
        );
        assert_eq!(weather_shift_text("Weather: Rainy"), None);
    }
}

use crate::geom::{Hwnd, Point, Rect};
use crate::input::Probe;
use crate::movement::RouteEnv;
use crate::platform::Platform;
use crate::vision::{key, Library, Pattern};
use std::collections::HashMap;
use std::sync::Arc;

/// OCR text of a screen -> Some(weather) if it's the card pick screen.
pub fn card_screen_text(text: &str) -> Option<String> {
    let t = text.to_lowercase();
    let flat: String = t
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ':')
        .collect();
    if !(flat.contains("chooseacard")
        || flat.contains("choosecard")
        || (t.contains("choose") && t.contains("card")))
    {
        return None;
    }
    // "Weather: Rainy" -> "Rainy"
    let weather = t
        .find("weather")
        .map(|i| {
            text[i + 7..]
                .trim_start_matches([':', ' '])
                .split(|c: char| !c.is_alphanumeric() && c != ' ')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default();
    Some(weather)
}

/// OCR text -> Some(weather) when a weather shift banner is up
/// ("Weather Shift", "Weather changed to ...").
pub fn weather_shift_text(text: &str) -> Option<String> {
    let t = text.to_lowercase();
    let i = t.find("weather")?;
    let rest = &t[i + 7..];
    if !(rest.contains("shift") || rest.contains("chang") || t[..i].contains("shift")) {
        return None;
    }
    let w = rest
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| {
            !w.is_empty()
                && ![
                    "shift", "shifted", "changed", "change", "to", "is", "now", "the",
                ]
                .contains(w)
        })
        .next()
        .unwrap_or("")
        .to_string();
    Some(w)
}

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

    /// Card pick screen by OCR: Some(weather) when "Choose a card" is on
    /// screen (weather "" when it isn't readable).
    pub fn ocr_card_screen(&self, frame: &crate::capture::Frame) -> Option<String> {
        let all = Rect::new(0, 0, frame.width as i32, frame.height as i32);
        let text = self.platform.ocr(frame, all)?;
        card_screen_text(&text)
    }

    /// The stage title bar at the top middle (map + mode name).
    pub fn ocr_title(&self, frame: &crate::capture::Frame) -> Option<String> {
        self.platform.ocr(frame, Rect::new(150, 0, 500, 48))
    }

    /// Everything readable on the screen (Windows OCR).
    pub fn ocr_all(&self, frame: &crate::capture::Frame) -> Option<String> {
        let all = Rect::new(0, 0, frame.width as i32, frame.height as i32);
        self.platform.ocr(frame, all)
    }

    /// VICTORY (true) / DEFEAT (false) read with OCR from the upper middle.
    pub fn ocr_end(&self, frame: &crate::capture::Frame) -> Option<bool> {
        let area = Rect::new(150, 60, 500, 220);
        let t = self.platform.ocr(frame, area)?.to_lowercase();
        let flat: String = t.chars().filter(|c| c.is_alphabetic()).collect();
        if flat.contains("victory") {
            Some(true)
        } else if flat.contains("defeat") {
            Some(false)
        } else {
            None
        }
    }

    /// One OCR read of the whole screen: (card screen weather, weather shift).
    pub fn ocr_events(&self, frame: &crate::capture::Frame) -> (Option<String>, Option<String>) {
        let all = Rect::new(0, 0, frame.width as i32, frame.height as i32);
        match self.platform.ocr(frame, all) {
            Some(t) => (card_screen_text(&t), weather_shift_text(&t)),
            None => (None, None),
        }
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

    /// Every place `name` shows on the whole picture (e.g. one Invite button
    /// per alt in the party list), top to bottom.
    pub fn find_all_wide(&self, name: &str) -> Vec<Point> {
        let Some(frame) = self.platform.capture_client(self.hwnd).ok() else {
            return vec![];
        };
        let Some(p) = self.pats.get(&key(name)) else {
            return vec![];
        };
        let mut out = vec![];
        let mut todo = vec![Rect::new(0, 0, frame.width as i32, frame.height as i32)];
        while let Some(r) = todo.pop() {
            if out.len() >= 12 || r.h < 4 {
                continue;
            }
            if let Some(h) = p.find_in(&frame, Some(r)) {
                out.push(h.center());
                // look above and below this one (list rows)
                let above = Rect::new(r.x, r.y, r.w, (h.at.y - r.y).max(0));
                let below_y = h.at.y + h.h;
                let below = Rect::new(r.x, below_y, r.w, (r.y + r.h - below_y).max(0));
                todo.push(above);
                todo.push(below);
            }
        }
        out.sort_by_key(|p| (p.y, p.x));
        out
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
