//! Loot tracking for the End step: what one run dropped ([`RunStats`]), the
//! session totals, `runtime/run_history.json`, and the Discord loot report.
//!
//! The scanning itself (OCR / patterns on the reward screen) lives in
//! `run.rs`; everything here is pure so it can be tested.

use crate::capture::Frame;
use crate::geom::{Point, Rect};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Older name of Summer Event currency rows in `drops` (the end-screen
/// parser). The event currency is the pearls (the pink flower).
pub const EVENT_CURRENCY: &str = "Summer Coin";
/// Name pearls have in `drops`.
pub const PEARLS: &str = "Pearls";
/// Runs kept in the history file (oldest dropped).
pub const HISTORY_KEEP: usize = 5000;
/// End panel (VICTORY / DEFEAT, 800x599): where "+7291" (the pearls this
/// run) sits, right of "+55 XP". Used when the pearl icon isn't found.
pub const PEARL_RECT: Rect = Rect::new(330, 336, 84, 36);
/// Where the pearl icon is looked for on the end panel.
pub const PEARL_SEARCH: Rect = Rect::new(250, 315, 300, 80);
/// The top-left HUD's pearl counter ("+7291" with the flower, under the
/// wave number) - shows the run's pearls on the end screen too.
pub const PEARL_HUD: Rect = Rect::new(140, 90, 100, 30);
/// Where the reward popup / end panel is: only here can a drop be.
pub const REWARD_AREA: Rect = Rect::new(200, 100, 400, 400);

/// Longest the End step waits for the Discord upload before going on.
pub const WEBHOOK_WAIT_SECS: u64 = 15;

/// One finished run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RunStats {
    /// Run number this session (1 = first after Start).
    pub run: u32,
    /// Local time, "YYYY-MM-DD HH:MM:SS".
    pub at: String,
    pub won: bool,
    pub mode: String,
    pub map: String,
    pub event_currency: u64,
    pub pearls: u64,
    /// The charm / portal seen on the reward screen (its OCR text).
    pub charm: Option<String>,
    /// Everything read off the end + reward screens.
    pub drops: Vec<(String, u64)>,
    /// Cropped picture of the dropped item, if one was seen.
    pub image: Option<PathBuf>,
}

impl RunStats {
    pub fn result(&self) -> &'static str {
        if self.won {
            "Victory"
        } else {
            "Defeat"
        }
    }
}

/// Totals since Start.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub runs: u32,
    pub wins: u32,
    pub event_currency: u64,
    pub pearls: u64,
    pub charms: u32,
    /// Every drop's total this session ("Gems" -> 350).
    pub items: std::collections::BTreeMap<String, u64>,
}

/// What a Dungeon report lists (each with this session's total).
pub const DUNGEON_ITEMS: [&str; 4] = ["Star Remnant", "Gems", "Gold", "Snow Reaper"];

impl Session {
    /// Count a run in; gives it its run number.
    pub fn add(&mut self, r: &mut RunStats) {
        self.runs += 1;
        r.run = self.runs;
        self.wins += r.won as u32;
        self.event_currency += r.event_currency;
        self.pearls += r.pearls;
        self.charms += r.charm.is_some() as u32;
        for (n, a) in &r.drops {
            *self.items.entry(n.clone()).or_default() += a;
        }
    }
}

/// Same item read on two screens (end screen, then the reward screen):
/// the bigger number, never the sum.
pub fn merge(a: &[(String, u64)], b: &[(String, u64)]) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = a.to_vec();
    for (n, v) in b {
        match out.iter_mut().find(|(m, _)| m == n) {
            Some(e) => e.1 = e.1.max(*v),
            None => out.push((n.clone(), *v)),
        }
    }
    out
}

pub fn amount(drops: &[(String, u64)], name: &str) -> u64 {
    drops
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, a)| *a)
        .unwrap_or(0)
}

/// In the middle of the screen, where the reward popup shows.
pub fn in_reward_area(r: Rect) -> bool {
    let c = r.center();
    crate::ocrareas::get("reward_area", REWARD_AREA).contains(c)
}

/// The pearls from OCR lines of the end panel: the "+7291" on the line
/// with "+55 XP" (the one that isn't the XP). None = no such line.
pub fn pearls_from_lines(lines: &[Vec<(String, Rect)>]) -> Option<u64> {
    for l in lines {
        let words: Vec<&str> = l.iter().map(|(w, _)| w.as_str()).collect();
        let Some(xp) = words.iter().position(|w| w.to_uppercase().contains("XP")) else {
            continue;
        };
        // a "+number" on that line that isn't the XP's own number
        for (i, w) in words.iter().enumerate() {
            if !w.starts_with('+') || i + 1 == xp || i == xp {
                continue;
            }
            if let Some(n) = parse_amount(w) {
                return Some(n);
            }
        }
    }
    None
}

/// A charm / portal on the reward screen by OCR: its line's text and the
/// box around that line (frame coordinates).
pub fn find_charm(lines: &[Vec<(String, Rect)>]) -> Option<(String, Rect)> {
    for l in lines {
        let text: String = l
            .iter()
            .map(|(w, _)| w.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let low = text.to_lowercase();
        // only the reward popup in the middle: "Equipped Charm:" (bottom
        // left, the charm you wear) and the minimap aren't drops
        if low.contains("equip") || l.first().is_some_and(|(_, r)| !in_reward_area(*r)) {
            continue;
        }
        if !(low.contains("charm") || low.contains("portal")) {
            continue;
        }
        let r = l.iter().map(|(_, r)| *r).reduce(|a, b| {
            let x = a.x.min(b.x);
            let y = a.y.min(b.y);
            Rect::new(
                x,
                y,
                a.right().max(b.right()) - x,
                a.bottom().max(b.bottom()) - y,
            )
        })?;
        return Some((text.trim().to_string(), r));
    }
    None
}

/// Picture area for a dropped item whose label / icon is at `at`: the icon
/// sits above its name, so mostly upwards. Clamped to the frame.
/// The middle of the end screen: the VICTORY / DEFEAT title, the pearls and
/// the rewards (not the HUD / chat around them).
pub fn end_shot_rect(w: u32, h: u32) -> Rect {
    let (w, h) = (w as i32, h as i32);
    Rect::new(w * 15 / 100, h * 5 / 100, w * 70 / 100, h * 80 / 100)
}

pub fn crop_around(at: Rect, w: u32, h: u32) -> Option<Rect> {
    let c = at.center();
    Rect::new(c.x - 110, c.y - 150, 220, 200).intersect(&Rect::new(0, 0, w as i32, h as i32))
}

/// Where to click the item again: the middle of what was found.
pub fn click_point(at: Rect) -> Point {
    at.center()
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The Discord loot report for `r` (with `image` as the attachment).
/// A Dungeon run's report: the dungeon loot (with session totals).
fn dungeon_message(
    r: &RunStats,
    s: &Session,
    image: Option<Frame>,
    ping: bool,
) -> crate::webhook::Message {
    use crate::webhook::{Kind, Message};
    let mut fields: Vec<(String, String, bool)> = DUNGEON_ITEMS
        .iter()
        .map(|n| {
            (
                n.to_string(),
                format!(
                    "{} (session {})",
                    thousands(amount(&r.drops, n)),
                    thousands(s.items.get(*n).copied().unwrap_or(0))
                ),
                true,
            )
        })
        .collect();
    fields.push((
        "Session".to_string(),
        format!("{} runs · {}W / {}L", s.runs, s.wins, s.runs - s.wins),
        true,
    ));
    let others: Vec<String> = r
        .drops
        .iter()
        .filter(|(n, _)| !DUNGEON_ITEMS.contains(&n.as_str()))
        .map(|(n, a)| format!("{} {n}", thousands(*a)))
        .collect();
    if !others.is_empty() {
        fields.push(("Other drops".into(), others.join(", "), false));
    }
    Message {
        kind: if r.won { Kind::Win } else { Kind::Loss },
        title: format!("Dungeon #{} Complete - {}", r.run, r.result()),
        body: r.map.clone(),
        screenshot: image,
        ping,
        fields,
        here: false,
        extra: vec![],
        thumbnail: None,
    }
}

pub fn message(
    r: &RunStats,
    s: &Session,
    image: Option<Frame>,
    ping: bool,
) -> crate::webhook::Message {
    use crate::webhook::{Kind, Message};
    if r.mode == "Dungeon" {
        return dungeon_message(r, s, image, ping);
    }
    // pearls are the Summer Event currency: one field, not two
    let mut fields = vec![
        (
            "Event Currency (Pearls)".to_string(),
            thousands(r.event_currency),
            true,
        ),
        (
            "Charm Dropped".to_string(),
            match &r.charm {
                Some(c) => format!("Yes - {c}"),
                None => "No".into(),
            },
            true,
        ),
        (
            "Session Total Event Currency".to_string(),
            thousands(s.event_currency),
            true,
        ),
        (
            "Session".to_string(),
            format!("{} runs · {}W / {}L", s.runs, s.wins, s.runs - s.wins),
            true,
        ),
    ];
    let others: Vec<String> = r
        .drops
        .iter()
        .filter(|(n, _)| n != EVENT_CURRENCY && n != PEARLS)
        .map(|(n, a)| format!("{} {n}", thousands(*a)))
        .collect();
    if !others.is_empty() {
        fields.push(("Other drops".into(), others.join(", "), false));
    }
    Message {
        kind: if r.won { Kind::Win } else { Kind::Loss },
        title: format!("Run #{} Complete - {}", r.run, r.result()),
        body: [r.mode.as_str(), r.map.as_str()]
            .iter()
            .filter(|t| !t.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" · "),
        screenshot: image,
        ping,
        fields,
        here: false,
        extra: vec![],
        thumbnail: None,
    }
}

/// The pink pearl icon right of the amount on the end panel: the rightmost
/// blob of pink pixels there (the "+7291" digits are pink too, the icon is
/// the separate blob after them).
pub fn pearl_icon(f: &Frame) -> Option<Rect> {
    let pinkish = |x: i32, y: i32| {
        f.rgb(x, y)
            .is_some_and(|(r, g, b)| r > 180 && b > 170 && r as i32 - g as i32 > 25)
    };
    let a = crate::ocrareas::get("pearls_search", PEARL_SEARCH);
    let col: Vec<u32> = (a.x..a.right())
        .map(|x| (a.y..a.bottom()).filter(|y| pinkish(x, *y)).count() as u32)
        .collect();
    // runs of columns with pink in them, split by gaps of 3+ columns
    let mut runs: Vec<(i32, i32)> = vec![];
    let mut cur: Option<(i32, i32)> = None;
    let mut gap = 0;
    for (i, n) in col.iter().enumerate() {
        let x = a.x + i as i32;
        if *n > 0 {
            cur = Some(cur.map_or((x, x), |(s, _)| (s, x)));
            gap = 0;
        } else if let Some(c) = cur {
            gap += 1;
            if gap >= 3 {
                runs.push(c);
                cur = None;
                gap = 0;
            }
        }
    }
    runs.extend(cur);
    // the icon: the last blob, icon-sized, with digits (another blob) before it
    let (x0, x1) = *runs.last()?;
    let w = x1 - x0 + 1;
    if runs.len() < 2 || !(14..=45).contains(&w) {
        return None;
    }
    let ys: Vec<i32> = (a.y..a.bottom())
        .filter(|y| (x0..=x1).any(|x| pinkish(x, *y)))
        .collect();
    let (y0, y1) = (*ys.first()?, *ys.last()?);
    Some(Rect::new(x0, y0, w, y1 - y0 + 1))
}

/// Where to read the amount: right before the icon (else [`PEARL_RECT`]).
pub fn pearl_amount_rect(icon: Option<Rect>) -> Rect {
    match icon {
        Some(i) => {
            let cy = i.y + i.h / 2;
            Rect::new((i.x - 90).max(0), cy - 18, 88, 36)
        }
        None => crate::ocrareas::get("pearls_box", PEARL_RECT),
    }
}

/// `area` cut out, 3x bigger, as black text on white - the pink-white
/// "+7291" on the dark panel is easy for OCR this way. (Fresh picture: its
/// own size, so OCR never swaps it for a new capture.)
pub fn clean_for_ocr(f: &Frame, area: Rect) -> Frame {
    const K: i32 = 3;
    let (w, h) = (area.w.max(1), area.h.max(1));
    let mut px = vec![255u8; (w * K * h * K * 4) as usize];
    for y in 0..h * K {
        for x in 0..w * K {
            let ink = f
                .rgb(area.x + x / K, area.y + y / K)
                .is_some_and(|(r, g, b)| {
                    let lo = r.min(g).min(b);
                    // white / pink-white text, not the dark panel
                    lo > 150 || (r > 200 && b > 180 && g > 110)
                });
            if ink {
                let i = ((y * w * K + x) * 4) as usize;
                px[i..i + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
    }
    Frame::new((w * K) as u32, (h * K) as u32, px)
}

/// "+7291", "+ 7,291", "+7291 &" (the icon read as a letter) -> 7291.
/// The longest digit group wins; a stray O read for 0 is fixed.
pub fn parse_amount(text: &str) -> Option<u64> {
    let t: String = text
        .chars()
        .map(|c| match c {
            'O' | 'o' => '0',
            'l' | 'I' => '1',
            _ => c,
        })
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != '.' && *c != '+')
        .collect();
    t.split(|c: char| !c.is_ascii_digit())
        .filter(|d| !d.is_empty() && d.len() <= 9)
        .max_by_key(|d| d.len())
        .and_then(|d| d.parse().ok())
}

pub fn history_path() -> PathBuf {
    super::data_dir().join("runtime").join("run_history.json")
}

/// Folder for the item pictures.
pub fn image_dir() -> PathBuf {
    super::data_dir().join("runtime").join("loot")
}

/// `history` plus `r`, at most [`HISTORY_KEEP`] runs.
pub fn appended(mut history: Vec<RunStats>, r: &RunStats) -> Vec<RunStats> {
    history.push(r.clone());
    let n = history.len();
    if n > HISTORY_KEEP {
        history.drain(..n - HISTORY_KEEP);
    }
    history
}

/// Add `r` to `run_history.json` (temp file + rename; a broken file is
/// kept aside as `.bad` and started over).
pub fn append_history(r: &RunStats) -> std::io::Result<PathBuf> {
    let p = history_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let old: Vec<RunStats> = match std::fs::read_to_string(&p) {
        Ok(t) => serde_json::from_str(&t).unwrap_or_else(|_| {
            let _ = std::fs::rename(&p, p.with_extension("json.bad"));
            vec![]
        }),
        Err(_) => vec![],
    };
    let all = appended(old, r);
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&all).unwrap_or_default())?;
    std::fs::rename(&tmp, &p)?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(words: &[(&str, i32)]) -> Vec<(String, Rect)> {
        words
            .iter()
            .map(|(w, x)| (w.to_string(), Rect::new(*x, 300, 40, 14)))
            .collect()
    }

    #[test]
    fn pearl_icon_and_amount_box_on_the_end_panel() {
        let f = Frame::load_png(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/end_defeat_pearls.png"
        )))
        .unwrap();
        let icon = pearl_icon(&f).expect("pearl icon on the DEFEAT panel");
        assert!((405..=418).contains(&icon.x), "{icon:?}");
        assert!((15..=35).contains(&icon.w), "{icon:?}");
        let r = pearl_amount_rect(Some(icon));
        // "+7291" is at x 345..404, y ~340..366: the box holds all of it
        assert!(r.x <= 345 && r.right() >= 404, "{r:?}");
        assert!(r.y <= 342 && r.bottom() >= 364, "{r:?}");
        // nothing pink there: no icon, the fixed box instead
        assert!(pearl_icon(&Frame::solid(800, 599, 10, 10, 10)).is_none());
        assert_eq!(pearl_amount_rect(None), PEARL_RECT);
        let fixed = PEARL_RECT;
        assert!(fixed.x <= 345 && fixed.right() >= 404);
    }

    #[test]
    fn pearls_from_the_xp_line() {
        let w = |t: &str, x: i32| (t.to_string(), Rect::new(x, 345, 40, 18));
        let lines = vec![
            vec![w("Total", 240), w("Time:", 290), w("33:12", 340)],
            vec![w("+55", 230), w("XP", 270), w("+7291", 345)],
        ];
        assert_eq!(pearls_from_lines(&lines), Some(7291));
        assert_eq!(
            pearls_from_lines(&[vec![w("+55", 230), w("XP", 270)]]),
            None
        );
        assert_eq!(
            pearls_from_lines(&[vec![w("Wave", 200), w("+30", 260)]]),
            None
        );
    }

    #[test]
    fn amounts_read_cleanly() {
        assert_eq!(parse_amount("+7291"), Some(7291));
        assert_eq!(parse_amount("+ 7,291"), Some(7291));
        assert_eq!(parse_amount("+7291 &"), Some(7291));
        assert_eq!(parse_amount("+72O1"), Some(7201));
        assert_eq!(parse_amount("+"), None);
        assert_eq!(parse_amount(""), None);
    }

    #[test]
    fn merge_takes_the_bigger_read() {
        let a = vec![("Summer Coin".into(), 1200), ("Gems".into(), 50)];
        let b = vec![("Summer Coin".into(), 1250), ("Pearls".into(), 15)];
        let m = merge(&a, &b);
        assert_eq!(amount(&m, "Summer Coin"), 1250);
        assert_eq!(amount(&m, "Gems"), 50);
        assert_eq!(amount(&m, "Pearls"), 15);
        assert_eq!(amount(&m, "Yen"), 0);
    }

    #[test]
    fn charm_found_by_ocr_with_its_box() {
        let lines = vec![
            line(&[("VICTORY", 350)]),
            line(&[("Lucky", 300), ("Charm", 345)]),
        ];
        let (t, r) = find_charm(&lines).unwrap();
        assert_eq!(t, "Lucky Charm");
        assert_eq!((r.x, r.w), (300, 85));
        assert!(find_charm(&[line(&[("Gems", 1), ("x50", 50)])]).is_none());
        // your run: "Equipped Charm:" (the one you wear) was taken for a drop
        let equipped = vec![
            ("Equipped".to_string(), Rect::new(65, 520, 50, 12)),
            ("Charm:".to_string(), Rect::new(118, 520, 40, 12)),
        ];
        assert!(find_charm(&[equipped]).is_none());
        // the minimap corner / anything outside the popup: never a drop
        assert!(find_charm(&[vec![("Charm".to_string(), Rect::new(700, 520, 40, 12))]]).is_none());
        assert!(find_charm(&[vec![("Portal".to_string(), Rect::new(20, 300, 40, 12))]]).is_none());
        assert!(find_charm(&[vec![(
            "Summer Charm".to_string(),
            Rect::new(380, 300, 60, 12)
        )]])
        .is_some());
        assert!(find_charm(&[vec![(
            "Summer Charm".to_string(),
            Rect::new(85, 578, 60, 10)
        )]])
        .is_none());
        let c = crop_around(r, 800, 599).unwrap();
        assert!(c.contains(click_point(r)));
        // near the edge: clamped, still there
        assert!(crop_around(Rect::new(0, 0, 10, 10), 800, 599).is_some());
    }

    #[test]
    fn session_numbers_runs_and_sums() {
        let mut s = Session::default();
        let mut a = RunStats {
            won: true,
            event_currency: 1250,
            pearls: 15,
            charm: Some("Lucky Charm".into()),
            ..Default::default()
        };
        let mut b = RunStats {
            event_currency: 1000,
            ..Default::default()
        };
        s.add(&mut a);
        s.add(&mut b);
        assert_eq!((a.run, b.run), (1, 2));
        assert_eq!(
            (s.runs, s.wins, s.event_currency, s.pearls, s.charms),
            (2, 1, 2250, 15, 1)
        );
    }

    #[test]
    fn discord_report_fields() {
        let mut s = Session {
            event_currency: 13_250,
            ..Default::default()
        };
        let mut r = RunStats {
            won: true,
            event_currency: 1250,
            pearls: 15,
            charm: Some("Lucky Charm".into()),
            drops: vec![
                ("Summer Coin".into(), 1250),
                ("Pearls".into(), 15),
                ("Gems".into(), 50),
            ],
            ..Default::default()
        };
        s.runs = 11;
        s.add(&mut r);
        let m = message(&r, &s, None, false);
        assert_eq!(m.title, "Run #12 Complete - Victory");
        let f = |n: &str| m.fields.iter().find(|x| x.0 == n).map(|x| x.1.clone());
        assert_eq!(f("Event Currency (Pearls)").as_deref(), Some("1,250"));
        assert_eq!(f("Charm Dropped").as_deref(), Some("Yes - Lucky Charm"));
        assert_eq!(f("Session Total Event Currency").as_deref(), Some("14,500"));
        assert_eq!(f("Other drops").as_deref(), Some("50 Gems"));
    }

    #[test]
    fn history_is_capped() {
        let h: Vec<RunStats> = (0..HISTORY_KEEP as u32)
            .map(|i| RunStats {
                run: i,
                ..Default::default()
            })
            .collect();
        let h = appended(
            h,
            &RunStats {
                run: 99_999,
                ..Default::default()
            },
        );
        assert_eq!(h.len(), HISTORY_KEEP);
        assert_eq!(h[0].run, 1);
        assert_eq!(h.last().unwrap().run, 99_999);
        assert_eq!(thousands(1_250), "1,250");
        assert_eq!(thousands(999), "999");
    }

    #[test]
    fn end_shot_is_the_middle() {
        let r = end_shot_rect(800, 600);
        assert_eq!((r.x, r.y, r.w, r.h), (120, 30, 560, 480));
        assert!(r.x + r.w <= 800 && r.y + r.h <= 600);
        assert!(r.contains(REWARD_AREA.center()));
    }

    #[test]
    fn dungeon_report_fields() {
        let mut s = Session::default();
        let mut r = RunStats {
            won: true,
            mode: "Dungeon".into(),
            drops: vec![
                ("Gems".into(), 40),
                ("Gold".into(), 1200),
                ("Snow Reaper".into(), 1),
                ("Trait Crystal".into(), 2),
            ],
            ..Default::default()
        };
        s.add(&mut r);
        let mut r2 = RunStats {
            mode: "Dungeon".into(),
            drops: vec![("Gems".into(), 10)],
            ..Default::default()
        };
        s.add(&mut r2);
        let m = message(&r2, &s, None, false);
        assert!(m.title.starts_with("Dungeon #2"));
        let f = |n: &str| m.fields.iter().find(|x| x.0 == n).unwrap().1.clone();
        assert_eq!(f("Gems"), "10 (session 50)");
        assert_eq!(f("Gold"), "0 (session 1,200)");
        assert_eq!(f("Snow Reaper"), "0 (session 1)");
        let m1 = message(&r, &s, None, false);
        assert!(m1
            .fields
            .iter()
            .any(|x| x.0 == "Other drops" && x.1.contains("Trait Crystal")));
    }

    #[test]
    fn pearls_cleaned_for_ocr() {
        let f = Frame::load_png(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/end_defeat_pearls.png"),
        )
        .unwrap();
        let area = pearl_amount_rect(pearl_icon(&f));
        let c = clean_for_ocr(&f, area);
        assert_eq!((c.width, c.height), (area.w as u32 * 3, area.h as u32 * 3));
        // "+7291" is ink: a good share of the box, but not all of it
        let ink = c.bgra.chunks_exact(4).filter(|p| p[0] == 0).count();
        let all = (c.width * c.height) as usize;
        assert!(ink > all / 20 && ink < all / 2, "{ink} of {all}");
    }
}
