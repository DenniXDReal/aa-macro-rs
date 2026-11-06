//! Loot tracking for the End step: what one run dropped ([`RunStats`]), the
//! session totals, `runtime/run_history.json`, and the Discord loot report.
//!
//! The scanning itself (OCR / patterns on the reward screen) lives in
//! `run.rs`; everything here is pure so it can be tested.

use crate::capture::Frame;
use crate::geom::{Point, Rect};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Name the Summer Event currency has in `drops` (the end-screen parser).
pub const EVENT_CURRENCY: &str = "Summer Coin";
/// Name pearls have in `drops`.
pub const PEARLS: &str = "Pearls";
/// Runs kept in the history file (oldest dropped).
pub const HISTORY_KEEP: usize = 5000;
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
}

impl Session {
    /// Count a run in; gives it its run number.
    pub fn add(&mut self, r: &mut RunStats) {
        self.runs += 1;
        r.run = self.runs;
        self.wins += r.won as u32;
        self.event_currency += r.event_currency;
        self.pearls += r.pearls;
        self.charms += r.charm.is_some() as u32;
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
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The Discord loot report for `r` (with `image` as the attachment).
pub fn message(
    r: &RunStats,
    s: &Session,
    image: Option<Frame>,
    ping: bool,
) -> crate::webhook::Message {
    use crate::webhook::{Kind, Message};
    let mut fields = vec![
        (
            "Event Currency".to_string(),
            thousands(r.event_currency),
            true,
        ),
        ("Pearls".to_string(), thousands(r.pearls), true),
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
            "Session Total Pearls".to_string(),
            thousands(s.pearls),
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
    }
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
        assert_eq!(f("Event Currency").as_deref(), Some("1,250"));
        assert_eq!(f("Pearls").as_deref(), Some("15"));
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
}
