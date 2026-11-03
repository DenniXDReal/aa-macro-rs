//! Map recognition from the in-game minimap (bottom right). Needed for the
//! Summer Event, whose loading screen and stage title say "Planet Greenie
//! (Summer)" on every map. Each map's minimap is saved once as
//! maps/minimap/<Map Name>.png; later runs compare against those.

use crate::capture::Frame;
use crate::geom::Rect;
use std::path::{Path, PathBuf};

/// Where the minimap sits (client px, 800x599).
pub const MINIMAP: Rect = Rect {
    x: 642,
    y: 444,
    w: 146,
    h: 146,
};
const N: u32 = 24;

pub fn dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("maps").join("minimap")))
        .unwrap_or_else(|| PathBuf::from("maps/minimap"))
}

/// The minimap cut out of a full client picture.
pub fn crop(f: &Frame) -> Option<Frame> {
    let a = MINIMAP;
    if (a.x + a.w) as u32 > f.width || (a.y + a.h) as u32 > f.height {
        return None;
    }
    let mut v = Vec::with_capacity((a.w * a.h * 4) as usize);
    for y in a.y..a.y + a.h {
        let i = ((y as u32 * f.width + a.x as u32) * 4) as usize;
        v.extend_from_slice(&f.bgra[i..i + (a.w * 4) as usize]);
    }
    Some(Frame::new(a.w as u32, a.h as u32, v))
}

/// Small colour thumbnail (24x24 RGB averages) of a minimap crop.
pub fn fingerprint(m: &Frame) -> Vec<u8> {
    let mut out = Vec::with_capacity((N * N * 3) as usize);
    for gy in 0..N {
        for gx in 0..N {
            let (x0, x1) = (gx * m.width / N, (gx + 1) * m.width / N);
            let (y0, y1) = (gy * m.height / N, (gy + 1) * m.height / N);
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for y in y0..y1.max(y0 + 1) {
                for x in x0..x1.max(x0 + 1) {
                    if let Some((pr, pg, pb)) = m.rgb(x as i32, y as i32) {
                        r += pr as u32;
                        g += pg as u32;
                        b += pb as u32;
                        n += 1;
                    }
                }
            }
            let n = n.max(1);
            out.extend_from_slice(&[(r / n) as u8, (g / n) as u8, (b / n) as u8]);
        }
    }
    out
}

/// Mean colour difference (0 = identical, 255 = opposite).
pub fn distance(a: &[u8], b: &[u8]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 255.0;
    }
    a.iter()
        .zip(b)
        .map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs())
        .sum::<u32>() as f32
        / a.len() as f32
}

/// Saved minimaps: (map name, fingerprint). Files starting "unknown" are skipped.
pub fn library(folder: &Path) -> Vec<(String, Vec<u8>)> {
    let Ok(rd) = std::fs::read_dir(folder) else {
        return vec![];
    };
    rd.flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_stem()?.to_string_lossy().to_string();
            if p.extension()?.to_str()? != "png" || name.to_lowercase().starts_with("unknown") {
                return None;
            }
            // "Magic Hills 2" = a second look of Magic Hills
            let map = name
                .rsplit_once(' ')
                .filter(|_| !name.starts_with("Summer map"))
                .filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit()))
                .map(|(m, _)| m.to_string())
                .unwrap_or(name);
            Some((map, fingerprint(&Frame::load_png(&p).ok()?)))
        })
        .collect()
}

/// Best matching saved map, if close enough.
pub fn recognise(f: &Frame, lib: &[(String, Vec<u8>)]) -> Option<(String, f32)> {
    let fp = fingerprint(&crop(f)?);
    lib.iter()
        .map(|(n, l)| (n.clone(), distance(&fp, l)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, d)| *d <= 18.0)
}

/// Save an unrecognised minimap under the next free "Summer map N" name and
/// return that name - no naming needed; rename the file later if you like.
pub fn save_new(f: &Frame, folder: &Path) -> Option<String> {
    let m = crop(f)?;
    let _ = std::fs::create_dir_all(folder);
    let mut n = 1;
    while folder.join(format!("Summer map {n}.png")).exists() {
        n += 1;
    }
    let name = format!("Summer map {n}");
    m.save_png(&folder.join(format!("{name}.png"))).ok()?;
    Some(name)
}

/// Normal modes know their map: keep its minimap too (once), so the
/// library fills up by itself.
pub fn save_named_if_missing(f: &Frame, folder: &Path, map: &str) -> bool {
    let clean: String = map
        .chars()
        .filter(|c| c.is_alphanumeric() || " ()-".contains(*c))
        .collect();
    let p = folder.join(format!("{clean}.png"));
    if clean.is_empty() || p.exists() {
        return false;
    }
    let Some(m) = crop(f) else { return false };
    let _ = std::fs::create_dir_all(folder);
    m.save_png(&p).is_ok()
}

/// Keep an unrecognised minimap so you can name it (rename the file to the
/// map's name). Skips it if an unknown that looks the same is already there.
pub fn save_unknown(f: &Frame, folder: &Path) -> Option<PathBuf> {
    let m = crop(f)?;
    let fp = fingerprint(&m);
    let _ = std::fs::create_dir_all(folder);
    if let Ok(rd) = std::fs::read_dir(folder) {
        for e in rd.flatten() {
            let p = e.path();
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if stem.starts_with("unknown") {
                if let Ok(o) = Frame::load_png(&p) {
                    if distance(&fingerprint(&o), &fp) <= 18.0 {
                        return None;
                    }
                }
            }
        }
    }
    let p = folder.join(format!(
        "unknown-{}.png",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    m.save_png(&p).ok()?;
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(r: u8, g: u8, b: u8, path_y: i32) -> Frame {
        let mut f = Frame::solid(800, 599, 30, 30, 30);
        for y in MINIMAP.y..MINIMAP.y + MINIMAP.h {
            for x in MINIMAP.x..MINIMAP.x + MINIMAP.w {
                let on_path = (y - (MINIMAP.y + path_y)).abs() < 6;
                let i = ((y as u32 * 800 + x as u32) * 4) as usize;
                let c = if on_path { [40, 220, 60] } else { [b, g, r] };
                f.bgra[i..i + 3].copy_from_slice(&c);
            }
        }
        f
    }

    #[test]
    fn learns_and_recognises_minimaps() {
        let dir = std::env::temp_dir().join(format!("aa-mm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let hills = shot(120, 90, 60, 40);
        let island = shot(60, 110, 160, 100);
        assert!(recognise(&hills, &library(&dir)).is_none());
        let u = save_unknown(&hills, &dir).unwrap();
        assert!(save_unknown(&hills, &dir).is_none(), "same unknown once");
        std::fs::rename(&u, dir.join("Magic Hills.png")).unwrap();
        crop(&island)
            .unwrap()
            .save_png(&dir.join("Puppet Island.png"))
            .unwrap();
        let lib = library(&dir);
        assert_eq!(recognise(&hills, &lib).unwrap().0, "Magic Hills");
        assert_eq!(recognise(&island, &lib).unwrap().0, "Puppet Island");
        // brand new map: numbered by itself, recognised next time
        let other = shot(200, 40, 40, 70);
        assert_eq!(save_new(&other, &dir).as_deref(), Some("Summer map 1"));
        let lib = library(&dir);
        assert_eq!(recognise(&other, &lib).unwrap().0, "Summer map 1");
        assert_eq!(
            save_new(&shot(10, 10, 200, 20), &dir).as_deref(),
            Some("Summer map 2")
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
