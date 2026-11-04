//! Buttons that never move (the settings cog, its X, ...): once found, the
//! spot is kept (button-spots.json) and later clicks go straight there - no
//! picture search, no OCR, no waiting.

use crate::geom::Point;
use std::collections::HashMap;
use std::sync::Mutex;

/// Buttons that always sit in the same place.
pub const STATIC: &[&str] = &["settings_button", "settings_close", "party_close"];

/// Toggles stay on screen after a click: a second "safe" click would undo it.
pub fn is_toggle(name: &str) -> bool {
    name == "settings_button" || name == "party_plus" || name.ends_with("_toggle")
}

pub fn is_static(name: &str) -> bool {
    STATIC.contains(&name)
}

static MEM: Mutex<Option<HashMap<String, (i32, i32)>>> = Mutex::new(None);

fn path() -> std::path::PathBuf {
    super::data_dir().join("button-spots.json")
}

fn with<T>(f: impl FnOnce(&mut HashMap<String, (i32, i32)>) -> T) -> T {
    let mut g = MEM.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(|| {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    });
    f(m)
}

fn save(m: &HashMap<String, (i32, i32)>) {
    if let Ok(s) = serde_json::to_string_pretty(m) {
        let _ = std::fs::create_dir_all(super::data_dir());
        let _ = std::fs::write(path(), s);
    }
}

/// Where a static button was last found.
pub fn get(name: &str) -> Option<Point> {
    if !is_static(name) {
        return None;
    }
    with(|m| m.get(name).map(|(x, y)| Point::new(*x, *y)))
}

/// A static button was found at `p`: keep it (saved when it moved).
pub fn remember(name: &str, p: Point) {
    if !is_static(name) {
        return;
    }
    with(|m| {
        let moved = m
            .get(name)
            .is_none_or(|(x, y)| (x - p.x).abs() > 4 || (y - p.y).abs() > 4);
        if moved {
            m.insert(name.to_string(), (p.x, p.y));
            save(m);
        }
    });
}

/// Forget one spot (it didn't work), or all of them (`None`).
pub fn forget(name: Option<&str>) {
    with(|m| {
        match name {
            Some(n) => {
                m.remove(n);
            }
            None => m.clear(),
        }
        save(m);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_static_spots_only() {
        remember("replay", Point::new(1, 2));
        assert_eq!(get("replay"), None);
        remember("settings_close", Point::new(583, 147));
        assert_eq!(get("settings_close"), Some(Point::new(583, 147)));
        forget(Some("settings_close"));
        assert_eq!(get("settings_close"), None);
        assert!(is_toggle("settings_button") && is_toggle("legend_stages_toggle"));
    }
}
