//! Weather shift card picker: your priority list (highest first) decides
//! which of the 3 offered cards is voted for; nothing on the list = the
//! leftmost card.
//!
//! The list lives in `weather_priorities.json` next to the exe (edited in
//! Settings > Weather). Until you save one, the built-in default from
//! `assets/weather_priorities.json` is used (buffs first, then debuffs).
//! It's read again every weather shift, so edits apply at once.

use std::path::PathBuf;

/// Built-in default (baked into the exe).
pub const DEFAULT_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/weather_priorities.json"
));

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

/// Entries the card reader would never produce (typos, unknown cards):
/// they can't match anything offered.
pub fn unknown(list: &[String]) -> Vec<String> {
    list.iter()
        .filter(|n| super::cards::card_name(n).as_deref() != Some(n.as_str()))
        .cloned()
        .collect()
}

/// Which of the 3 cards to vote for: the one highest on `priority` (exact
/// name first, else the same card at another tier); none on the list =
/// the leftmost (0). Also says where the pick is on your list.
pub fn pick(names: &[Option<String>; 3], priority: &[String]) -> (usize, Option<usize>) {
    match super::cards::choose(names, priority) {
        Some(i) => {
            let rank = names[i]
                .as_ref()
                .and_then(|n| priority.iter().position(|p| p == n));
            (i, rank)
        }
        None => (0, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(v: [&str; 3]) -> [Option<String>; 3] {
        v.map(|s| (!s.is_empty()).then(|| s.to_string()))
    }

    #[test]
    fn highest_on_the_list_wins() {
        let prio: Vec<String> = ["Attack I", "Cooldown I", "Enemy Speed I"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            pick(&n(["Cooldown I", "Enemy Speed I", "Attack I"]), &prio),
            (2, Some(0))
        );
        assert_eq!(
            pick(&n(["Enemy Speed I", "Cooldown I", ""]), &prio),
            (1, Some(1))
        );
        // none of them on the list: the leftmost
        assert_eq!(
            pick(&n(["Yen I", "Range I", "Enemy Regen I"]), &prio),
            (0, None)
        );
        // nothing read at all: the leftmost too
        assert_eq!(pick(&n(["", "", ""]), &prio), (0, None));
    }

    #[test]
    fn default_list_is_valid_and_buffs_first() {
        let d = default_list();
        assert!(d.len() >= 40, "{}", d.len());
        assert!(unknown(&d).is_empty(), "{:?}", unknown(&d));
        let first_red = d
            .iter()
            .position(|c| !super::super::cards::is_green(c))
            .unwrap();
        assert!(d[first_red..]
            .iter()
            .all(|c| !super::super::cards::is_green(c)));
    }

    #[test]
    fn text_editor_round_trip() {
        let l = from_text("Attack I\n\n  Cooldown I \n\"Enemy Speed I\",\nattack i\n");
        assert_eq!(l, ["Attack I", "Cooldown I", "Enemy Speed I"]);
        assert_eq!(from_text(&to_text(&l)), l);
        assert_eq!(unknown(&from_text("Attak I\nAttack I")), ["Attak I"]);
    }

    #[test]
    fn saved_list_is_loaded_and_bad_files_fall_back() {
        let mine = vec!["Range I".to_string(), "Yen I".to_string()];
        save(&mine).unwrap();
        assert_eq!(load(), mine);
        std::fs::write(path(), "not json").unwrap();
        assert_eq!(load(), default_list());
        let _ = std::fs::remove_file(path());
    }
}
