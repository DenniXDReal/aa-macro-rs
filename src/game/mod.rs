//! Live game context: what a client's screen shows right now, as probes the
//! input arbiter can wait on. Shared by the Movement tester and the game loop.

pub mod adaptation;
pub mod buttonmem;
pub mod cardlog;
pub mod cards;
pub mod drops;
pub mod dump;
pub mod hotbar;
pub mod loot;
pub mod minimap;
pub mod placing;
pub mod resume;
pub mod run;
#[cfg(test)]
pub mod sim;
pub mod steps;
pub mod textbuttons;
pub mod trace;

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
        assert_eq!(super::end_text("V I C T O R Y  Wave 25"), Some(true));
        // vote box: green Yes left of a red No
        let mut f = crate::capture::Frame::solid(800, 599, 20, 20, 20);
        for y in 92..114 {
            for x in 302..402 {
                let i = ((y * 800 + x) * 4) as usize;
                f.bgra[i..i + 3].copy_from_slice(&[10, 210, 70]);
            }
            for x in 405..495 {
                let i = ((y * 800 + x) * 4) as usize;
                f.bgra[i..i + 3].copy_from_slice(&[20, 20, 220]);
            }
        }
        let p = super::vote_yes(&f).unwrap();
        assert!((p.x - 351).abs() <= 2 && (p.y - 102).abs() <= 2, "{p:?}");
        assert!(super::vote_yes(&crate::capture::Frame::solid(800, 599, 20, 20, 20)).is_none());
        assert_eq!(super::end_text("DEFEAT"), Some(false));
        assert_eq!(super::end_text("Wave 3 Vote Start"), None);
        let r = crate::geom::Rect::new(100, 200, 30, 12);
        let lines = vec![
            vec![("Sell".to_string(), r)],
            vec![("UPG".to_string(), r), ("[0]".to_string(), r)],
        ];
        assert_eq!(super::upg_line(&lines).unwrap().0, "UPG [0]");
        assert_eq!(
            super::upg_line(&lines[..1]).unwrap().0,
            "Sell",
            "panel's Sell"
        );
        assert!(super::upg_line(&[vec![("Wave".to_string(), r)]]).is_none());
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

/// VICTORY (true) / DEFEAT (false) in OCR text, tolerant of OCR slips.
pub fn end_text(t: &str) -> Option<bool> {
    let flat: String = t
        .chars()
        .filter(|c| c.is_alphabetic())
        .flat_map(|c| c.to_lowercase())
        .collect();
    let win = [
        "victory",
        "victorv",
        "vict0ry",
        "stagecleared",
        "stagecomplete",
    ];
    let loss = ["defeat", "defeal", "deteat", "gameover", "stagefailed"];
    if win.iter().any(|w| flat.contains(w)) {
        Some(true)
    } else if loss.iter().any(|w| flat.contains(w)) {
        Some(false)
    } else {
        None
    }
}

/// The "Vote start" box's green Yes button (next to the red No), by colour:
/// same place on every map (around 352,103), no picture / OCR needed.
pub fn vote_yes(f: &crate::capture::Frame) -> Option<Point> {
    let (mut n, mut sx, mut sy, mut red) = (0i64, 0i64, 0i64, 0i64);
    for y in 80..125 {
        for x in 280..500 {
            let Some((r, g, b)) = f.rgb(x, y) else {
                continue;
            };
            let (r, g, b) = (r as i32, g as i32, b as i32);
            if g > 150 && r < 130 && b < 90 && g - r > 60 {
                n += 1;
                sx += x as i64;
                sy += y as i64;
            } else if r > 170 && g < 70 && b < 70 {
                red += 1;
            }
        }
    }
    (n >= 600 && red >= 400).then(|| Point::new((sx / n) as i32, (sy / n) as i32))
}

/// The OCR line with the unit panel's "UPG [n]": (its text, where "UPG" is).
pub fn upg_line(lines: &[Vec<(String, Rect)>]) -> Option<(String, Point)> {
    // the unit panel's own words (as AA Spiral v17 checks): Sell, and the
    // targeting button (First / Last / Strongest / ...)
    let panel_word = |w: &str| {
        let f: String = w
            .chars()
            .filter(|c| c.is_alphabetic())
            .flat_map(|c| c.to_lowercase())
            .collect();
        matches!(
            f.as_str(),
            "sell"
                | "first"
                | "last"
                | "strongest"
                | "weakest"
                | "closest"
                | "farthest"
                | "furthest"
        )
    };
    let mut word_hit = None;
    for l in lines {
        for (i, (w, r)) in l.iter().enumerate() {
            if word_hit.is_none() && panel_word(w) {
                word_hit = Some((w.clone(), Point::new(r.x + r.w / 2, r.y + r.h / 2)));
            }
            let f: String = w
                .chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(|c| c.to_uppercase())
                .collect();
            // "UPG", "UPG[0]", OCR slips "UP6" / "UPC"
            if f.starts_with("UPG") || f.starts_with("UP6") || f.starts_with("UPC") {
                let rest: Vec<&str> = l[i..].iter().take(3).map(|(t, _)| t.as_str()).collect();
                let text = rest.join(" ").replace("UP6", "UPG").replace("UPC", "UPG");
                if parse_level(&text).is_some() {
                    return Some((text, Point::new(r.x + r.w / 2, r.y + r.h / 2)));
                }
            }
        }
    }
    word_hit
}

/// One client's screen + a snapshot of your patterns.
#[derive(Clone)]
pub struct LiveScreen {
    platform: Arc<dyn Platform>,
    hwnd: Hwnd,
    /// Swapped by hot-reload when the patterns folder changes.
    pats: crate::hotreload::LivePatterns,
}

impl LiveScreen {
    pub fn new(platform: Arc<dyn Platform>, hwnd: Hwnd, lib: &Library) -> Self {
        Self {
            platform,
            hwnd,
            pats: crate::hotreload::live_patterns(crate::hotreload::pattern_map(lib)),
        }
    }

    /// Centre of the first of `names` found now (inside `region` if given).
    pub fn find(&self, names: &[String], region: Option<Rect>) -> Option<Point> {
        let pats = self.pats.load_full();
        let frame = self.platform.capture_client(self.hwnd).ok()?;
        let hit = names.iter().find_map(|n| {
            let p = pats.get(&key(n))?;
            match p.find_in(&frame, region) {
                Some(h) => {
                    crate::debugbus::hit(
                        self.hwnd,
                        Rect::new(h.at.x, h.at.y, h.w, h.h),
                        n,
                        h.score,
                    );
                    buttonmem::remember(&key(n), h.center());
                    Some(h.center())
                }
                None => {
                    crate::debugbus::miss(self.hwnd, region.unwrap_or(p.search), n);
                    None
                }
            }
        });
        self.seen(names, hit.is_some());
        hit.or_else(|| self.find_text(&frame, names, region).into_iter().next())
    }

    /// Where one of `labels` is written right now (OCR, first label first).
    pub fn find_words_now(&self, labels: &[&str]) -> Option<Point> {
        let frame = self.frame()?;
        let lines = self.ocr_lines(&frame)?;
        labels
            .iter()
            .find_map(|l| textbuttons::find_all(&lines, &[l], None).into_iter().next())
    }

    /// The vote box's Yes button right now (by colour).
    pub fn find_vote_yes(&self) -> Option<Point> {
        self.frame().and_then(|f| vote_yes(&f))
    }

    pub fn hwnd(&self) -> Hwnd {
        self.hwnd
    }

    /// A captured pattern by name.
    pub fn pattern(&self, name: &str) -> Option<Pattern> {
        self.pats.load().get(&key(name)).cloned()
    }

    /// Read button labels right now (no waiting for the picture first).
    pub fn find_label_now(&self, names: &[String]) -> Option<Point> {
        let frame = self.frame()?;
        let lines = self.ocr_lines(&frame)?;
        names
            .iter()
            .filter_map(|n| textbuttons::labels_for(&key(n)))
            .flat_map(|l| textbuttons::find_all(&lines, l, None))
            .next()
    }

    /// Track how long `names` have been missing: the OCR backup only runs for
    /// a button that hasn't been seen for a while (OCR is slow).
    fn seen(&self, names: &[String], found: bool) {
        use std::time::Instant;
        let k = names.join("|");
        let mut m = MISSING.lock().unwrap_or_else(|e| e.into_inner());
        if found {
            m.retain(|(h, n, _)| !(*h == self.hwnd && *n == k));
        } else if !m.iter().any(|(h, n, _)| *h == self.hwnd && *n == k) {
            if m.len() > 200 {
                m.clear();
            }
            m.push((self.hwnd, k, Instant::now()));
        }
    }

    fn missing_for(&self, names: &[String]) -> std::time::Duration {
        let k = names.join("|");
        MISSING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(h, n, _)| *h == self.hwnd && *n == k)
            .map(|(_, _, t)| t.elapsed())
            .unwrap_or_default()
    }

    /// Backup: the buttons' labels read with OCR (when the picture isn't
    /// found or was never captured). One OCR per client every 0.7 s at most.
    pub fn find_text(
        &self,
        frame: &crate::capture::Frame,
        names: &[String],
        within: Option<Rect>,
    ) -> Vec<Point> {
        let pats = self.pats.load_full();
        use std::sync::Mutex;
        use std::time::{Duration, Instant};
        type Lines = Vec<Vec<(String, Rect)>>;
        static CACHE: Mutex<Vec<(Hwnd, Instant, Arc<Lines>)>> = Mutex::new(Vec::new());
        let labels: Vec<&[&str]> = names
            .iter()
            .filter_map(|n| textbuttons::labels_for(&key(n)))
            .collect();
        if labels.is_empty() {
            return vec![];
        }
        // captured and only just gone: don't OCR yet (it's slow, and a
        // "wait until it's gone" loop would crawl)
        let captured = names.iter().any(|n| pats.contains_key(&key(n)));
        if captured && self.missing_for(names) < Duration::from_millis(2500) {
            return vec![];
        }
        let cached = CACHE.lock().ok().and_then(|c| {
            c.iter()
                .find(|(h, t, _)| *h == self.hwnd && t.elapsed() < Duration::from_millis(1200))
                .map(|(_, _, l)| l.clone())
        });
        let lines = match cached {
            Some(l) => l,
            None => {
                let all = Rect::new(0, 0, frame.width as i32, frame.height as i32);
                let Some(l) = self.platform.ocr_lines(frame, all) else {
                    return vec![];
                };
                let l = Arc::new(l);
                if let Ok(mut c) = CACHE.lock() {
                    c.retain(|(h, _, _)| *h != self.hwnd);
                    c.push((self.hwnd, Instant::now(), l.clone()));
                }
                l
            }
        };
        labels
            .iter()
            .flat_map(|l| textbuttons::find_all(&lines, l, within))
            .collect()
    }

    /// Where the unit panel's level text sits: around your `unit_placed`
    /// capture, else the AHK macro's spot.
    pub fn level_area(&self) -> Rect {
        let pats = self.pats.load_full();
        match pats.get(&key("unit_placed")) {
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
        let fresh = self.fresh(frame);
        let frame: &crate::capture::Frame = &fresh;
        if let Some(l) = self
            .platform
            .ocr(frame, self.level_area())
            .and_then(|t| parse_level(&t))
        {
            return Some(l);
        }
        // thin "1" / brackets lost: same box once more with the strokes
        // thickened by 1 px
        let a = self.level_area();
        if let Some(a) = a.intersect(&Rect::new(0, 0, frame.width as i32, frame.height as i32)) {
            let crop = frame.crop(a.x as u32, a.y as u32, a.w as u32, a.h as u32);
            let thick = crate::vision::dilate_h(&crop);
            let all = Rect::new(0, 0, thick.width as i32, thick.height as i32);
            if let Some(l) = self.platform.ocr(&thick, all).and_then(|t| parse_level(&t)) {
                return Some(l);
            }
        }
        // the panel may sit elsewhere: find "UPG" anywhere and read its line
        let lines = self.ocr_lines(frame)?;
        upg_line(&lines)
            .filter(|(t, _)| t.to_uppercase().contains("UPG") || t.to_uppercase().contains("MAX"))
            .and_then(|(t, _)| parse_level(&t))
    }

    pub fn has_ocr(&self) -> bool {
        self.platform.has_ocr()
    }

    /// Where the unit panel's "UPG [n]" (or MAX) text is, read with OCR over
    /// the whole screen: the unit panel is open = a unit was just placed.
    pub fn find_upg(&self, frame: &crate::capture::Frame) -> Option<Point> {
        let fresh = self.fresh(frame);
        let frame: &crate::capture::Frame = &fresh;
        // where it was last time on this client (read sharper, 4x), then the
        // usual unit panel spot, then the whole screen (2x)
        static LAST: std::sync::Mutex<Vec<(Hwnd, Rect)>> = std::sync::Mutex::new(Vec::new());
        let last = LAST
            .lock()
            .ok()
            .and_then(|v| v.iter().find(|(h, _)| *h == self.hwnd).map(|(_, r)| *r));
        let mut tries: Vec<Rect> = last.into_iter().collect();
        let la = self.level_area();
        tries.push(Rect::new(la.x - 120, la.y - 60, la.w + 240, la.h + 220));
        tries.push(Rect::new(0, 0, frame.width as i32, frame.height as i32));
        for r in tries {
            let Some(lines) = self.platform.ocr_lines(frame, r) else {
                continue;
            };
            if let Some((_, p)) = upg_line(&lines) {
                if let Ok(mut v) = LAST.lock() {
                    v.retain(|(h, _)| *h != self.hwnd);
                    v.push((self.hwnd, Rect::new(p.x - 150, p.y - 25, 300, 50)));
                }
                return Some(p);
            }
        }
        None
    }

    /// Card pick screen by OCR: Some(weather) when "Choose a card" is on
    /// screen (weather "" when it isn't readable).
    pub fn ocr_card_screen(&self, frame: &crate::capture::Frame) -> Option<String> {
        let fresh = self.fresh(frame);
        let frame: &crate::capture::Frame = &fresh;
        let all = Rect::new(0, 0, frame.width as i32, frame.height as i32);
        let text = self.platform.ocr(frame, all)?;
        card_screen_text(&text)
    }

    /// The stage title bar at the top middle (map + mode name).
    pub fn ocr_title(&self, frame: &crate::capture::Frame) -> Option<String> {
        let fresh = self.fresh(frame);
        let frame: &crate::capture::Frame = &fresh;
        self.platform.ocr(frame, Rect::new(150, 0, 500, 48))
    }

    /// OCR of one area of a picture.
    pub fn ocr_rect(&self, frame: &crate::capture::Frame, area: Rect) -> Option<String> {
        let fresh = self.fresh(frame);
        let frame: &crate::capture::Frame = &fresh;
        let t = self.platform.ocr(frame, area);
        if let Some(t) = &t {
            crate::debugbus::ocr(self.hwnd, area, t);
        }
        t
    }

    /// OCR of the whole screen with word boxes (None without OCR).
    pub fn ocr_lines(&self, frame: &crate::capture::Frame) -> Option<Vec<Vec<(String, Rect)>>> {
        let fresh = self.fresh(frame);
        let frame: &crate::capture::Frame = &fresh;
        let all = Rect::new(0, 0, frame.width as i32, frame.height as i32);
        let lines = self.platform.ocr_lines(frame, all);
        if crate::debugbus::enabled() {
            for l in lines.iter().flatten() {
                if let Some(r) = l.iter().map(|(_, r)| *r).reduce(|a, b| {
                    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
                    Rect::new(
                        x,
                        y,
                        (a.x + a.w).max(b.x + b.w) - x,
                        (a.y + a.h).max(b.y + b.h) - y,
                    )
                }) {
                    let t: Vec<&str> = l.iter().map(|(w, _)| w.as_str()).collect();
                    crate::debugbus::ocr(self.hwnd, r, &t.join(" "));
                }
            }
        }
        lines
    }

    /// The whole screen's text, quick (2x read), one line per line.
    pub fn ocr_text_fast(&self, frame: &crate::capture::Frame) -> Option<String> {
        let fresh = self.fresh(frame);
        let frame: &crate::capture::Frame = &fresh;
        let Some(lines) = self.ocr_lines(frame) else {
            return self.ocr_all(frame);
        };
        Some(
            lines
                .iter()
                .map(|w| {
                    w.iter()
                        .map(|(t, _)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// The screen's text, one string per line.
    pub fn ocr_text_lines(&self, frame: &crate::capture::Frame) -> Vec<String> {
        let fresh = self.fresh(frame);
        let frame: &crate::capture::Frame = &fresh;
        if let Some(l) = self.ocr_lines(frame) {
            return l
                .iter()
                .map(|w| {
                    w.iter()
                        .map(|(t, _)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect();
        }
        self.ocr_all(frame)
            .map(|t| t.lines().map(String::from).collect())
            .unwrap_or_default()
    }

    /// The hotbar counter of the selected unit ("1/3"), read with OCR.
    pub fn ocr_counter(&self, frame: &crate::capture::Frame) -> Option<(u32, u32)> {
        let fresh = self.fresh(frame);
        let frame: &crate::capture::Frame = &fresh;
        let area = hotbar::counter_area(frame.width as i32, frame.height as i32);
        let t = self.platform.ocr(frame, area)?;
        hotbar::parse_count(&t)
    }

    /// Everything readable on the screen (Windows OCR).
    pub fn ocr_all(&self, frame: &crate::capture::Frame) -> Option<String> {
        let fresh = self.fresh(frame);
        let frame: &crate::capture::Frame = &fresh;
        let all = Rect::new(0, 0, frame.width as i32, frame.height as i32);
        self.platform.ocr(frame, all)
    }

    /// VICTORY (true) / DEFEAT (false) read with OCR from the upper middle.
    pub fn ocr_end(&self, frame: &crate::capture::Frame) -> Option<bool> {
        let fresh = self.fresh(frame);
        let frame: &crate::capture::Frame = &fresh;
        // the upper 60% of the screen (the title sits in different places on
        // the VICTORY and DEFEAT screens)
        let area = Rect::new(0, 0, frame.width as i32, frame.height as i32 * 3 / 5);
        let t = self.platform.ocr(frame, area)?;
        end_text(&t)
    }

    /// One OCR read of the whole screen: (card screen weather, weather shift).
    pub fn ocr_events(&self, frame: &crate::capture::Frame) -> (Option<String>, Option<String>) {
        let fresh = self.fresh(frame);
        let frame: &crate::capture::Frame = &fresh;
        let all = Rect::new(0, 0, frame.width as i32, frame.height as i32);
        match self.platform.ocr(frame, all) {
            Some(t) => (card_screen_text(&t), weather_shift_text(&t)),
            None => (None, None),
        }
    }

    /// Is `name` in an already captured picture (no new capture)?
    pub fn find_in_frame(&self, frame: &crate::capture::Frame, name: &str) -> bool {
        self.pats
            .load()
            .get(&key(name))
            .and_then(|p| p.find_in(frame, None))
            .is_some()
    }

    /// The client's picture right now.
    pub fn frame(&self) -> Option<crate::capture::Frame> {
        let f = self.platform.capture_client(self.hwnd).ok()?;
        crate::debugbus::captured(self.hwnd);
        // frozen / minimized window: input landed since the last picture but
        // the picture didn't change at all (the game world always moves) -
        // look once more, and if it's still identical don't trust it
        let gen = crate::input::generation(self.hwnd);
        let hash = f.quick_hash();
        let same_after_input = {
            let mut m = LAST_SHOT.lock().unwrap_or_else(|e| e.into_inner());
            let prev = m
                .iter()
                .find(|(h, ..)| *h == self.hwnd)
                .map(|(_, ph, pg)| (*ph, *pg));
            m.retain(|(h, ..)| *h != self.hwnd);
            m.push((self.hwnd, hash, gen));
            prev.is_some_and(|(ph, pg)| ph == hash && pg < gen)
        };
        if same_after_input {
            std::thread::sleep(std::time::Duration::from_millis(120));
            let g = self.platform.capture_client(self.hwnd).ok()?;
            if g.quick_hash() == hash {
                trace::log(&format!(
                    "stale picture: client window {:?} didn't change after input (frozen / minimized?)",
                    self.hwnd
                ));
                return None;
            }
            return Some(g);
        }
        Some(f)
    }

    /// `f`, or a fresh picture when `f` is too old for this read (urgent
    /// reads: 250 ms, background: 1 s).
    pub fn fresh<'a>(
        &self,
        f: &'a crate::capture::Frame,
    ) -> std::borrow::Cow<'a, crate::capture::Frame> {
        let urgent = crate::platform::OCR_IS_URGENT.with(|c| c.get());
        let max = if urgent {
            MAX_AGE_URGENT
        } else {
            MAX_AGE_BACKGROUND
        };
        if f.age() <= max {
            return std::borrow::Cow::Borrowed(f);
        }
        match self.platform.capture_client(self.hwnd).ok() {
            Some(n) if n.width == f.width && n.height == f.height => std::borrow::Cow::Owned(n),
            _ => std::borrow::Cow::Borrowed(f),
        }
    }

    /// Captured pattern names (normalised) starting with `prefix`.
    pub fn with_prefix(&self, prefix: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .pats
            .load()
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        v.sort();
        v
    }

    pub fn has(&self, name: &str) -> bool {
        let pats = self.pats.load_full();
        pats.contains_key(&key(name))
    }

    /// Like `find`, but over the whole picture (the button may sit somewhere
    /// else than where you captured it, e.g. Next on a DEFEAT vs VICTORY screen).
    pub fn find_wide(&self, names: &[String]) -> Option<Point> {
        let pats = self.pats.load_full();
        let frame = self.platform.capture_client(self.hwnd).ok()?;
        let all = Rect::new(0, 0, frame.width as i32, frame.height as i32);
        let hit = names.iter().find_map(|n| {
            let p = pats.get(&key(n))?;
            match p
                .find_in(&frame, None)
                .or_else(|| p.find_in(&frame, Some(all)))
            {
                Some(h) => {
                    crate::debugbus::hit(
                        self.hwnd,
                        Rect::new(h.at.x, h.at.y, h.w, h.h),
                        n,
                        h.score,
                    );
                    buttonmem::remember(&key(n), h.center());
                    Some(h.center())
                }
                None => {
                    crate::debugbus::miss(self.hwnd, all, n);
                    None
                }
            }
        });
        self.seen(names, hit.is_some());
        hit.or_else(|| self.find_text(&frame, names, None).into_iter().next())
    }

    /// Every place `name` shows on the whole picture (e.g. one Invite button
    /// per alt in the party list), top to bottom.
    pub fn find_all_wide(&self, name: &str) -> Vec<Point> {
        let pats = self.pats.load_full();
        let Some(frame) = self.platform.capture_client(self.hwnd).ok() else {
            return vec![];
        };
        let Some(p) = pats.get(&key(name)) else {
            return self.find_text(&frame, &[name.to_string()], None);
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
        if out.is_empty() {
            return self.find_text(&frame, &[name.to_string()], None);
        }
        out.sort_by_key(|p| (p.y, p.x));
        out
    }

    /// Names in `names` that you haven't captured yet.
    pub fn missing(&self, names: &[String]) -> Vec<String> {
        let pats = self.pats.load_full();
        names
            .iter()
            .filter(|n| !pats.contains_key(&key(n)))
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
    fn remembered(&self, pattern: &str) -> Option<Point> {
        buttonmem::get(&key(pattern))
    }
}

/// Oldest picture an urgent read (placement check, upgrade level) may use.
pub const MAX_AGE_URGENT: std::time::Duration = std::time::Duration::from_millis(250);
/// Oldest picture a background read (cards, end screen, events) may use.
pub const MAX_AGE_BACKGROUND: std::time::Duration = std::time::Duration::from_secs(1);

/// Last picture per client: (window, quick hash, input generation then).
static LAST_SHOT: std::sync::Mutex<Vec<(Hwnd, u64, u64)>> = std::sync::Mutex::new(Vec::new());

/// (client, names, missing since) for the OCR backup's patience.
static MISSING: std::sync::Mutex<Vec<(Hwnd, String, std::time::Instant)>> =
    std::sync::Mutex::new(Vec::new());

/// Never sees anything: used to check a route's syntax.
pub struct Blind;

impl RouteEnv for Blind {
    fn probe(&self, patterns: &[String], _region: Option<Rect>) -> Probe {
        Probe::new(patterns.join(" | "), || None)
    }
}

/// Folder the macro keeps its pictures in (next to the exe). Tests get a
/// fresh folder per run so old pictures never leak between runs.
pub fn data_dir() -> std::path::PathBuf {
    #[cfg(test)]
    {
        std::env::temp_dir().join(format!("aa-macro-test-{}", std::process::id()))
    }
    #[cfg(not(test))]
    {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    }
}
