//! Collects a picture of every different card pick screen (cards/ next to
//! the exe), so you can see which cards exist before choosing favourites.

use super::cards::CARD_AREA;
use crate::capture::Frame;
use std::path::{Path, PathBuf};

pub fn dir() -> PathBuf {
    super::data_dir().join("cards")
}

/// 128-bit fingerprint of the card area (16x8 grey, above / below average).
pub fn fingerprint(f: &Frame) -> u128 {
    let a = CARD_AREA;
    let mut cells = [0u32; 128];
    for gy in 0..8 {
        for gx in 0..16 {
            let mut sum = 0u32;
            let mut n = 0u32;
            for y in (a.y + gy * a.h / 8..a.y + (gy + 1) * a.h / 8).step_by(3) {
                for x in (a.x + gx * a.w / 16..a.x + (gx + 1) * a.w / 16).step_by(3) {
                    if let Some((r, g, b)) = f.rgb(x, y) {
                        sum += (r as u32 * 38 + g as u32 * 75 + b as u32 * 15) >> 7;
                        n += 1;
                    }
                }
            }
            cells[(gy * 16 + gx) as usize] = sum / n.max(1);
        }
    }
    let avg = cells.iter().sum::<u32>() / 128;
    cells.iter().enumerate().fold(
        0u128,
        |acc, (i, v)| if *v > avg { acc | (1 << i) } else { acc },
    )
}

/// Save the screen when at least one of `cards` wasn't seen before (names
/// kept in cards/seen.txt). The file name lists the new cards.
pub fn save_if_new_cards(
    f: &Frame,
    folder: &Path,
    cards: &[String],
    weather: &str,
) -> Option<PathBuf> {
    let _ = std::fs::create_dir_all(folder);
    let seen_path = folder.join("seen.txt");
    let mut seen: Vec<String> = std::fs::read_to_string(&seen_path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let new: Vec<&String> = cards.iter().filter(|c| !seen.contains(c)).collect();
    if new.is_empty() {
        return None;
    }
    let tag: String = new
        .iter()
        .map(|c| c.replace(' ', ""))
        .collect::<Vec<_>>()
        .join("+");
    let w: String = weather.chars().filter(|c| c.is_alphanumeric()).collect();
    let name = format!(
        "new-{}-{tag}{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        if w.is_empty() {
            String::new()
        } else {
            format!("-{w}")
        }
    );
    let path = folder.join(format!("{name}.png"));
    f.save_png(&path).ok()?;
    seen.extend(new.into_iter().cloned());
    let _ = std::fs::write(&seen_path, seen.join("\n") + "\n");
    Some(path)
}

/// Save this card screen if it looks new. Returns the file when saved.
pub fn save_if_new(f: &Frame, folder: &Path, weather: &str) -> Option<PathBuf> {
    let _ = std::fs::create_dir_all(folder);
    let index = folder.join("index.txt");
    let fp = fingerprint(f);
    let known: Vec<u128> = std::fs::read_to_string(&index)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| u128::from_str_radix(l.split_whitespace().next()?, 16).ok())
        .collect();
    if known.iter().any(|k| (k ^ fp).count_ones() <= 10) {
        return None;
    }
    let w: String = weather.chars().filter(|c| c.is_alphanumeric()).collect();
    let name = format!(
        "cards-{}{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        if w.is_empty() {
            String::new()
        } else {
            format!("-{w}")
        }
    );
    let a = CARD_AREA;
    let mut crop = Vec::with_capacity((a.w * a.h * 4) as usize);
    for y in a.y..a.y + a.h {
        let i = ((y as u32 * f.width + a.x as u32) * 4) as usize;
        crop.extend_from_slice(&f.bgra[i..i + (a.w * 4) as usize]);
    }
    let path = folder.join(format!("{name}.png"));
    Frame::new(a.w as u32, a.h as u32, crop)
        .save_png(&path)
        .ok()?;
    let _ = f.save_png(&folder.join(format!("{name}-full.png")));
    let mut text = std::fs::read_to_string(&index).unwrap_or_default();
    text.push_str(&format!("{fp:032x} {name}\n"));
    let _ = std::fs::write(&index, text);
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_screen_saved_once() {
        let dir = std::env::temp_dir().join(format!("aa-cards-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut f = Frame::solid(800, 599, 20, 20, 20);
        for y in 200..400 {
            for x in 170..310 {
                let i = ((y * 800 + x) * 4) as usize;
                f.bgra[i..i + 3].copy_from_slice(&[40, 200, 60]);
            }
        }
        assert!(save_if_new(&f, &dir, "").is_some());
        assert!(save_if_new(&f, &dir, "").is_none(), "same cards again");
        let mut g = Frame::solid(800, 599, 20, 20, 20);
        for y in 200..400 {
            for x in 530..670 {
                let i = ((y * 800 + x) * 4) as usize;
                g.bgra[i..i + 3].copy_from_slice(&[220, 60, 60]);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(save_if_new(&g, &dir, "Rainy").is_some(), "different cards");
        // by card name: only new names trigger a screenshot
        let a = ["Attack I".to_string(), "Enemy Speed II".to_string()];
        assert!(save_if_new_cards(&f, &dir, &a, "").is_some());
        assert!(save_if_new_cards(&f, &dir, &a, "").is_none(), "nothing new");
        let b = ["Attack I".to_string(), "Yen III".to_string()];
        let p = save_if_new_cards(&f, &dir, &b, "Rainy").unwrap();
        assert!(p.to_string_lossy().contains("YenIII"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
