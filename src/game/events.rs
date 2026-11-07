//! Custom events: your own "when X shows on screen, do Y" rules, kept in
//! `custom_events.json` - no new code for a new mode or situation.
//!
//! A rule: a trigger (when it's looked at: start, before placing, after an
//! upgrade, before the end, every N s; optionally only on some maps /
//! modes), an OCR condition (an area of the client and the text to find
//! there: exact, contains or regex - empty text = always), and an action
//! (type text, press a key, click a spot, jump to a step). It fires once,
//! then not again until the text has gone (and at most once a second).
//!
//! The text typed is always the text you wrote - never what was read.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum When {
    MacroStart,
    BeforePlace,
    AfterUpgrade,
    BeforeEnd,
    /// Every this many seconds while a stage runs.
    Every(u32),
}

impl When {
    pub const KINDS: [&'static str; 5] = [
        "On Macro Start",
        "Before Place",
        "After Upgrade",
        "Before End",
        "Every X seconds",
    ];
    pub fn kind(self) -> usize {
        match self {
            When::MacroStart => 0,
            When::BeforePlace => 1,
            When::AfterUpgrade => 2,
            When::BeforeEnd => 3,
            When::Every(_) => 4,
        }
    }
    pub fn from_kind(k: usize, secs: u32) -> When {
        match k {
            0 => When::MacroStart,
            1 => When::BeforePlace,
            2 => When::AfterUpgrade,
            3 => When::BeforeEnd,
            _ => When::Every(secs.max(1)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Match {
    Exact,
    #[default]
    Contains,
    Regex,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Do {
    /// Type `text` ("\n" = Enter). `chat`: press / first and Enter after.
    TypeText {
        text: String,
        chat: bool,
    },
    PressKey {
        key: String,
    },
    Click {
        x: i32,
        y: i32,
    },
    /// Jump the step machine to this step ("place", "upgrade", "end", ...).
    ChangeStep {
        step: String,
    },
}

impl Do {
    pub const KINDS: [&'static str; 4] =
        ["Type Text", "Press Key", "Click Coordinates", "Change Step"];
    pub fn kind(&self) -> usize {
        match self {
            Do::TypeText { .. } => 0,
            Do::PressKey { .. } => 1,
            Do::Click { .. } => 2,
            Do::ChangeStep { .. } => 3,
        }
    }
    pub fn of_kind(k: usize) -> Do {
        match k {
            0 => Do::TypeText {
                text: String::new(),
                chat: false,
            },
            1 => Do::PressKey { key: "e".into() },
            2 => Do::Click { x: 400, y: 300 },
            _ => Do::ChangeStep {
                step: "upgrade".into(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub name: String,
    pub enabled: bool,
    pub when: When,
    /// Only on these maps (empty = all).
    pub maps: Vec<String>,
    /// Only in these modes, by name (empty = all).
    pub modes: Vec<String>,
    /// Where to read: [x, y, w, h] on the client.
    pub area: [i32; 4],
    /// What to find there (empty = no OCR, always true).
    pub text: String,
    pub matching: Match,
    pub action: Do,
    /// Look (and act) on every client, not only the main one.
    pub all_clients: bool,
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            name: "New rule".into(),
            enabled: true,
            when: When::Every(5),
            maps: vec![],
            modes: vec![],
            area: [300, 200, 200, 60],
            text: String::new(),
            matching: Match::Contains,
            action: Do::PressKey { key: "e".into() },
            all_clients: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rules {
    pub rules: Vec<Rule>,
    /// Pause between typed characters (ms).
    pub type_gap_ms: u64,
}

pub fn path() -> std::path::PathBuf {
    super::data_dir().join("custom_events.json")
}

/// Your rules (none / unreadable file = no rules).
pub fn load() -> Rules {
    load_from(&path())
}

pub fn load_from(p: &std::path::Path) -> Rules {
    let mut r: Rules = std::fs::read_to_string(p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    if r.type_gap_ms == 0 {
        r.type_gap_ms = 50;
    }
    r
}

pub fn save(r: &Rules) -> anyhow::Result<()> {
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(r)?)?;
    Ok(())
}

fn norm(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Does `read` (what OCR saw) meet the rule's text?
pub fn matches(rule: &Rule, read: &str) -> bool {
    if rule.text.trim().is_empty() {
        return true;
    }
    match rule.matching {
        Match::Exact => norm(read) == norm(&rule.text),
        Match::Contains => norm(read).contains(&norm(&rule.text)),
        Match::Regex => regex::Regex::new(&rule.text)
            .map(|re| re.is_match(read))
            .unwrap_or(false),
    }
}

/// Is the rule on for this map and mode?
pub fn applies(rule: &Rule, map: &str, mode: &str) -> bool {
    let key = |s: &str| super::mapnav::key(s);
    rule.enabled
        && (rule.maps.is_empty() || rule.maps.iter().any(|m| key(m) == key(map)))
        && (rule.modes.is_empty() || rule.modes.iter().any(|m| norm(m) == norm(mode)))
}

/// Fire once, then not again until the text has gone (and at most once a
/// second): per (rule, client).
#[derive(Default)]
pub struct Latch {
    state: HashMap<(usize, isize), (bool, Instant)>,
}

impl Latch {
    /// `seen` this look: should the action run now?
    pub fn fire(&mut self, rule: usize, client: isize, seen: bool, now: Instant) -> bool {
        let e = self
            .state
            .entry((rule, client))
            .or_insert((false, now - Duration::from_secs(10)));
        if !seen {
            e.0 = false;
            return false;
        }
        if e.0 || now.duration_since(e.1) < Duration::from_secs(1) {
            return false;
        }
        *e = (true, now);
        true
    }
}

/// "Every N s" rules: when each last ran.
#[derive(Default)]
pub struct Due {
    last: HashMap<usize, Instant>,
}

impl Due {
    pub fn due(&mut self, rule: usize, secs: u32, now: Instant) -> bool {
        match self.last.get(&rule) {
            Some(t) if now.duration_since(*t) < Duration::from_secs(secs as u64) => false,
            _ => {
                self.last.insert(rule, now);
                true
            }
        }
    }
}

/// A step jump asked for by a rule (taken by the step machine after the
/// step it's in).
static JUMP: Mutex<Option<super::steps::Step>> = Mutex::new(None);

pub fn request_jump(step: super::steps::Step) {
    *JUMP.lock().unwrap_or_else(|e| e.into_inner()) = Some(step);
}

pub fn take_jump() -> Option<super::steps::Step> {
    JUMP.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// "place" / "Upgrade" / "end" -> the step.
pub fn step_named(name: &str) -> Option<super::steps::Step> {
    use super::steps::Step;
    [
        Step::Lobby,
        Step::StageSetup,
        Step::Walk,
        Step::Vote,
        Step::Place,
        Step::Upgrade,
        Step::Dungeon,
        Step::End,
    ]
    .into_iter()
    .find(|s| s.name().eq_ignore_ascii_case(name.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(text: &str, m: Match) -> Rule {
        Rule {
            text: text.into(),
            matching: m,
            ..Default::default()
        }
    }

    #[test]
    fn text_matching() {
        assert!(matches(
            &rule("Wave 10", Match::Contains),
            "  WAVE   10  / 50"
        ));
        assert!(!matches(&rule("Wave 10", Match::Exact), "Wave 10 / 50"));
        assert!(matches(&rule("wave 10", Match::Exact), "Wave  10"));
        assert!(matches(&rule(r"Wave (1\d|20)\b", Match::Regex), "Wave 15"));
        assert!(!matches(&rule(r"Wave (1\d|20)\b", Match::Regex), "Wave 25"));
        // a bad regex never fires
        assert!(!matches(&rule("Wave (", Match::Regex), "Wave ("));
        // no text = always
        assert!(matches(&rule("", Match::Exact), "anything"));
    }

    #[test]
    fn only_where_asked() {
        let mut r = Rule::default();
        assert!(applies(&r, "Navy Bay", "Story"));
        r.maps = vec!["navy bay".into()];
        r.modes = vec!["Summer Event".into()];
        assert!(applies(&r, "Navy Bay_summerevent", "summer event"));
        assert!(!applies(&r, "Sand Village", "Summer Event"));
        assert!(!applies(&r, "Navy Bay", "Story"));
        r.enabled = false;
        assert!(!applies(&r, "Navy Bay", "Summer Event"));
    }

    #[test]
    fn fires_once_until_the_text_goes() {
        let mut l = Latch::default();
        let t = Instant::now();
        let s = |ms| t + Duration::from_millis(ms);
        assert!(l.fire(0, 1, true, s(0)));
        // still on screen: not again, however often it's looked at
        for ms in (100..5000).step_by(100) {
            assert!(!l.fire(0, 1, true, s(ms)));
        }
        // gone, then back: fires again
        assert!(!l.fire(0, 1, false, s(5100)));
        assert!(l.fire(0, 1, true, s(5200)));
        // gone and back within a second of firing: waits
        assert!(!l.fire(0, 1, false, s(5300)));
        assert!(!l.fire(0, 1, true, s(5400)));
        assert!(l.fire(0, 1, true, s(6300)));
        // another client / rule is its own
        assert!(l.fire(0, 2, true, s(0)));
        assert!(l.fire(1, 1, true, s(0)));
    }

    #[test]
    fn every_n_seconds() {
        let mut d = Due::default();
        let t = Instant::now();
        assert!(d.due(3, 5, t));
        assert!(!d.due(3, 5, t + Duration::from_secs(4)));
        assert!(d.due(3, 5, t + Duration::from_secs(5)));
    }

    #[test]
    fn rules_round_trip() {
        let dir = std::env::temp_dir().join(format!("aa-events-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("custom_events.json");
        let rules = Rules {
            rules: vec![
                Rule {
                    name: "jump at wave 10".into(),
                    when: When::Every(3),
                    text: "Wave 10".into(),
                    action: Do::PressKey {
                        key: "space".into(),
                    },
                    ..Default::default()
                },
                Rule {
                    when: When::BeforeEnd,
                    action: Do::TypeText {
                        text: "gg".into(),
                        chat: true,
                    },
                    ..Default::default()
                },
            ],
            type_gap_ms: 50,
        };
        std::fs::write(&p, serde_json::to_string_pretty(&rules).unwrap()).unwrap();
        assert_eq!(load_from(&p), rules);
        assert_eq!(
            step_named("Upgrade"),
            Some(super::super::steps::Step::Upgrade)
        );
        assert_eq!(step_named("nope"), None);
    }
}
