//! Weather shift card picker: your priority list (highest first) decides
//! which of the 3 offered cards is voted for; nothing on the list (or
//! nothing readable) = the leftmost card.
//!
//! Weather shift cards are their own set (Grounded, Mud, Haboob, ...: see
//! [`WEATHER_CARDS`]) and have no tiers - not the I / II / III cards of the
//! normal card pick.
//!
//! The list lives in `weather_priorities.json` next to the exe (edited in
//! Settings > Weather). Until you save one, the built-in default from
//! `assets/weather_priorities.json` is used (buffs first, then the pearl
//! cards from mildest to harshest). It's read again every weather shift,
//! so edits apply at once.

use std::path::PathBuf;

/// Every weather shift card seen (from your cards/ screenshots, Nov 2026):
/// (name, weather, buff). Buff = helps you (green text); the others pay
/// SummerPearls and make the enemies stronger (red text).
pub const WEATHER_CARDS: &[(&str, &str, bool)] = &[
    // Windy
    ("Grounded", "Windy", true),     // -20% enemy flying chance
    ("Kite Strings", "Windy", true), // +75% damage vs flying
    ("Gale", "Windy", false),        // +50% pearls, +15% enemy flying chance
    ("Crosswind", "Windy", false),   // +50% pearls, +15% cooldown
    ("Cyclone", "Windy", false),     // +100% pearls, +30% flying, +25% enemy speed
    // Rainy
    ("Rain Barrel", "Rainy", true),  // +6000 yen
    ("Mud", "Rainy", true),          // -25% enemy speed
    ("Flash Flood", "Rainy", false), // +50% pearls, +25% enemy speed
    ("Washout", "Rainy", false),     // +50% pearls, +50% enemy health
    ("Torrent", "Rainy", false),     // +100% pearls, +50% speed, +50% health
    // Flooded
    ("Salvage", "Flooded", true),      // +6000 yen
    ("Low Tide", "Flooded", true),     // -25% enemy speed
    ("Rising Tide", "Flooded", false), // +50% pearls, +50% enemy health
    ("Tidal Surge", "Flooded", false), // +50% pearls, +50% burst enemy speed
    ("Tsunami", "Flooded", false),     // +100% pearls, +50% speed, +50% health
    // Sandstorm
    ("Buried Treasure", "Sandstorm", true), // +6000 yen
    ("Clear Skies", "Sandstorm", true),     // -55 enemy magic resist
    ("Sandblast", "Sandstorm", true),       // +40% physical damage taken
    ("Dust Devil", "Sandstorm", false),     // +50% pearls, +40 enemy magic resist
    ("Sand Armor", "Sandstorm", false),     // +50% pearls, +3 enemy shield
    ("Haboob", "Sandstorm", false),         // +100% pearls, +50% health, +75 magic res
    // Magical
    ("Mana Surge", "Magical", true),    // +40% magic damage taken
    ("Dispel", "Magical", true),        // -110 enemy physical resist
    ("Leyline", "Magical", true),       // -15% cooldown
    ("Arcane Ward", "Magical", false),  // +50% pearls, +75 physical resist
    ("Mana Shield", "Magical", false),  // +50% pearls, +3 enemy shield
    ("Arcane Storm", "Magical", false), // +100% pearls, +50% health, +150 phys res
    // Inferno
    ("Cinders", "Inferno", true),   // +50% boss damage
    ("Firebreak", "Inferno", true), // -30% enemy speed
    ("Heatwave", "Inferno", false), // +50% pearls, +25% enemy speed
    ("Scorch", "Inferno", false),   // +50% pearls, +15% cooldown
    ("Phoenix", "Inferno", false),  // +100% pearls, +50% health, +25% speed
    // Starfall
    ("Stardust", "Starfall", true),     // +6000 yen
    ("Wishing Star", "Starfall", true), // +100% Star Golem chance
    ("Cosmic Rays", "Starfall", false), // +50% pearls, +15% cooldown
    ("Starforged", "Starfall", false),  // +50% pearls, +50% secret drops, +50% health
    ("Doomsday", "Starfall", false),    // +100% pearls, +100% secret drops, +100% health
];

/// Built-in default (baked into the exe).
pub const DEFAULT_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/weather_priorities.json"
));

fn letters(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Edit distance (letters changed / added / dropped).
fn edits(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let sub = prev[j - 1] + (a[i - 1] != b[j - 1]) as usize;
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// OCR of a weather card title ("8, Kite Strings '", "Areane Storm",
/// "fain Barrel") -> "Kite Strings". The name inside the text, else the
/// closest name within a few letters (OCR slips). None = not a weather card.
pub fn card_name(text: &str) -> Option<String> {
    let t = letters(text);
    if t.len() < 3 {
        return None;
    }
    // the whole name in there - longest first
    let mut by_len: Vec<&(&str, &str, bool)> = WEATHER_CARDS.iter().collect();
    by_len.sort_by_key(|c| std::cmp::Reverse(c.0.len()));
    if let Some(c) = by_len.iter().find(|c| t.contains(&letters(c.0))) {
        return Some(c.0.to_string());
    }
    // the closest name, against every stretch of the text about its length
    // (the weather icon in front often reads as a letter or two)
    // (short names like "Mud" only against the whole text: as a stretch
    // they'd match bits of longer names). Best = fewest slips per letter.
    let mut best: Option<(f32, &str)> = None;
    for (name, _, _) in WEATHER_CARDS {
        let k = letters(name);
        let mut d = edits(&t, &k);
        if k.len() > 4 {
            for len in k.len() - 1..=k.len() + 1 {
                for start in 0..=t.len().saturating_sub(len) {
                    if let Some(w) = t.get(start..start + len) {
                        d = d.min(edits(w, &k));
                    }
                }
            }
        }
        let max = (k.len() / 4).max(1);
        let ratio = d as f32 / k.len() as f32;
        if d <= max && best.is_none_or(|(br, _)| ratio < br) {
            best = Some((ratio, name));
        }
    }
    best.map(|(_, n)| n.to_string())
}

pub fn is_buff(name: &str) -> bool {
    WEATHER_CARDS.iter().any(|(n, _, b)| *b && *n == name)
}

pub fn path() -> PathBuf {
    super::data_dir().join("weather_priorities.json")
}

pub fn default_list() -> Vec<String> {
    serde_json::from_str(DEFAULT_JSON).unwrap_or_default()
}

/// Your list, else the default (also when the file is broken or empty).
pub fn load() -> Vec<String> {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(default_list)
}

/// Save your list (temp file + rename).
pub fn save(list: &[String]) -> std::io::Result<PathBuf> {
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_string_pretty(list).unwrap_or_else(|_| "[]".into()),
    )?;
    std::fs::rename(&tmp, &p)?;
    Ok(p)
}

/// The editor's text (one card per line) -> the list: trimmed, no blank
/// lines, no repeats (the first one counts).
pub fn from_text(text: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for l in text.lines() {
        let l = l.trim().trim_matches(|c| c == '"' || c == ',').trim();
        if !l.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(l)) {
            out.push(l.to_string());
        }
    }
    out
}

pub fn to_text(list: &[String]) -> String {
    list.join("\n")
}

/// Entries that aren't a weather card name (typos, unknown cards): they
/// can't match anything offered.
pub fn unknown(list: &[String]) -> Vec<String> {
    list.iter()
        .filter(|n| {
            !WEATHER_CARDS
                .iter()
                .any(|(c, _, _)| c.eq_ignore_ascii_case(n))
        })
        .cloned()
        .collect()
}

/// Weather card names on the screen (OCR lines with positions), given to
/// the card whose title strip they're in (left / middle / right).
pub fn names_from_lines(lines: &[Vec<(String, crate::geom::Rect)>]) -> [Option<String>; 3] {
    let mut out: [Option<String>; 3] = [None, None, None];
    for l in lines {
        let text: String = l
            .iter()
            .map(|(w, _)| w.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let Some(name) = card_name(&text) else {
            continue;
        };
        let Some((_, r)) = l.first() else { continue };
        let cx = r.x + r.w / 2;
        if let Some(i) = super::cards::CARD_BOXES
            .iter()
            .position(|b| cx >= b.x && cx < b.x + b.w && r.y < b.y + 60)
        {
            out[i].get_or_insert(name);
        }
    }
    out
}

/// Which of the 3 cards to vote for: the one highest on `priority`; none
/// on the list (or none read) = the leftmost (0). Also says where the pick
/// is on your list.
pub fn pick(names: &[Option<String>; 3], priority: &[String]) -> (usize, Option<usize>) {
    names
        .iter()
        .enumerate()
        .filter_map(|(i, n)| {
            let n = n.as_deref()?;
            priority
                .iter()
                .position(|p| p.eq_ignore_ascii_case(n))
                .map(|r| (r, i))
        })
        .min()
        .map(|(r, i)| (i, Some(r)))
        .unwrap_or((0, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(v: [&str; 3]) -> [Option<String>; 3] {
        v.map(|s| (!s.is_empty()).then(|| s.to_string()))
    }

    #[test]
    fn highest_on_the_list_wins() {
        let prio: Vec<String> = ["Mud", "Rain Barrel", "Washout"].map(String::from).to_vec();
        assert_eq!(pick(&n(["Torrent", "Washout", "Mud"]), &prio), (2, Some(0)));
        assert_eq!(
            pick(&n(["Washout", "Rain Barrel", ""]), &prio),
            (1, Some(1))
        );
        // none of them on the list: the leftmost
        assert_eq!(pick(&n(["Gale", "Cyclone", "Grounded"]), &prio), (0, None));
        // nothing read at all: the leftmost too
        assert_eq!(pick(&n(["", "", ""]), &prio), (0, None));
    }

    #[test]
    fn default_list_is_valid_and_buffs_first() {
        let d = default_list();
        assert!(unknown(&d).is_empty(), "{:?}", unknown(&d));
        // every card exactly once, buffs before the rest
        assert_eq!(d.len(), WEATHER_CARDS.len());
        let first_red = d.iter().position(|c| !is_buff(c)).unwrap();
        assert!(d[first_red..].iter().all(|c| !is_buff(c)));
    }

    #[test]
    fn text_editor_round_trip() {
        let l = from_text("Mud\n\n  Leyline \n\"Rain Barrel\",\nmud\n");
        assert_eq!(l, ["Mud", "Leyline", "Rain Barrel"]);
        assert_eq!(from_text(&to_text(&l)), l);
        assert_eq!(unknown(&from_text("Mudd\nMud")), ["Mudd"]);
    }

    /// What OCR made of the titles on your screenshots.
    #[test]
    fn titles_read_despite_ocr_slips() {
        for (ocr, want) in [
            ("8, Kite Strings '", "Kite Strings"),
            ("Areane Storm |", "Arcane Storm"),
            ("fain Barrel", "Rain Barrel"),
            ("= Rain Garrel", "Rain Barrel"),
            ("Dispei", "Dispel"),
            ("Levline", "Leyline"),
            ("Mana Surae", "Mana Surge"),
            ("ManaShield", "Mana Shield"),
            ("Guried Treasure", "Buried Treasure"),
            ("Doomcdav", "Doomsday"),
            ("Wichina Star", "Wishing Star"),
            ("_. sand Armor |", "Sand Armor"),
            ("= Mud |", "Mud"),
            ("Cinderc", "Cinders"),
        ] {
            assert_eq!(card_name(ocr).as_deref(), Some(want), "{ocr}");
        }
        for junk in ["re", "", "Rainy Card", "SummerPearls", "+6000 Y"] {
            assert_eq!(card_name(junk), None, "{junk}");
        }
    }

    #[test]
    fn names_go_to_their_card() {
        use crate::geom::Rect;
        let line = |w: &str, x: i32| vec![(w.to_string(), Rect::new(x, 218, 60, 16))];
        let got = names_from_lines(&[line("Torrent", 175), line("Washout", 350), line("Mud", 560)]);
        assert_eq!(
            got,
            [
                Some("Torrent".into()),
                Some("Washout".into()),
                Some("Mud".into())
            ]
        );
    }

    #[test]
    fn saved_list_is_loaded_and_bad_files_fall_back() {
        let mine = vec!["Mud".to_string(), "Leyline".to_string()];
        save(&mine).unwrap();
        assert_eq!(load(), mine);
        std::fs::write(path(), "not json").unwrap();
        assert_eq!(load(), default_list());
        let _ = std::fs::remove_file(path());
    }
}
