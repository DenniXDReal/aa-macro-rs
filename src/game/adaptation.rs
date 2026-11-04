//! Mahoraga's "Choose an Adaptation" panel (Megumi's summon, every 50
//! kills). It offers Divine Wings, Heavenly Force and Adapted Slash; the
//! badge says what the choice is for: ATK (blue), DEF (green) or BOSS (red).
//! A choice that was taken before is drawn grey and can't be picked.

use crate::capture::Frame;
use crate::config::{Adaptations, ADAPTATION_NAMES};
use crate::geom::{Point, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    Atk,
    Def,
    Boss,
}

impl Badge {
    pub fn label(self) -> &'static str {
        match self {
            Badge::Atk => "ATK",
            Badge::Def => "DEF",
            Badge::Boss => "BOSS",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Choice {
    pub name: &'static str,
    pub at: Point,
    pub grey: bool,
}

#[derive(Clone, Debug)]
pub struct Panel {
    pub badge: Option<Badge>,
    pub choices: Vec<Choice>,
}

type Lines = [Vec<(String, Rect)>];

fn flat(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Is the panel up? (text of the whole screen)
pub fn showing(text: &str) -> bool {
    let f = flat(text);
    f.contains("chooseanadaptation") || f.contains("chooseadaptation")
}

/// White text at `at` = still available; grey = taken.
pub fn is_grey(f: &Frame, at: Point) -> bool {
    let mut white = 0;
    for y in (at.y - 7)..=(at.y + 7) {
        for x in (at.x - 45)..=(at.x + 45) {
            if let Some((r, g, b)) = f.rgb(x, y) {
                let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
                if lo > 205 && hi - lo < 30 {
                    white += 1;
                }
            }
        }
    }
    white < 6
}

/// Badge from its word (OCR), else from the colour below the choices.
fn badge(lines: &Lines, f: &Frame, below_y: i32, xs: (i32, i32)) -> Option<Badge> {
    for l in lines {
        for (w, _) in l {
            match flat(w).as_str() {
                "atk" => return Some(Badge::Atk),
                "def" => return Some(Badge::Def),
                "boss" => return Some(Badge::Boss),
                _ => {}
            }
        }
    }
    let (mut bl, mut gr, mut rd) = (0, 0, 0);
    for y in below_y..(below_y + 90) {
        for x in xs.0..xs.1 {
            if let Some((r, g, b)) = f.rgb(x, y) {
                let (r, g, b) = (r as i32, g as i32, b as i32);
                if b > 150 && b > r + 60 && b > g + 20 {
                    bl += 1;
                } else if g > 150 && g > r + 60 && g > b + 60 {
                    gr += 1;
                } else if r > 150 && r > g + 80 && r > b + 80 {
                    rd += 1;
                }
            }
        }
    }
    let best = bl.max(gr).max(rd);
    if best < 40 {
        None
    } else if best == bl {
        Some(Badge::Atk)
    } else if best == gr {
        Some(Badge::Def)
    } else {
        Some(Badge::Boss)
    }
}

/// Read the panel: where each readable choice is, whether it's grey, the badge.
pub fn read(lines: &Lines, f: &Frame) -> Option<Panel> {
    let text: String = lines
        .iter()
        .map(|l| {
            l.iter()
                .map(|(w, _)| w.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !showing(&text) {
        return None;
    }
    let mut choices = vec![];
    for name in ADAPTATION_NAMES {
        if let Some(at) = super::textbuttons::find_all(lines, &[name], None)
            .into_iter()
            .next()
        {
            choices.push(Choice {
                name,
                at,
                grey: is_grey(f, at),
            });
        }
    }
    let low = choices
        .iter()
        .map(|c| c.at.y)
        .max()
        .unwrap_or(f.height as i32 / 2)
        + 15;
    let xs = (
        choices.iter().map(|c| c.at.x).min().unwrap_or(200) - 80,
        choices.iter().map(|c| c.at.x).max().unwrap_or(600) + 80,
    );
    Some(Panel {
        badge: badge(lines, f, low, xs),
        choices,
    })
}

/// The first name in the player's order (for this badge) that isn't grey.
pub fn choose<'a>(p: &'a Panel, s: &Adaptations) -> Option<&'a Choice> {
    let order = match p.badge.unwrap_or(Badge::Atk) {
        Badge::Atk => &s.atk,
        Badge::Def => &s.def,
        Badge::Boss => &s.boss,
    };
    order
        .iter()
        .filter_map(|n| p.choices.iter().find(|c| c.name.eq_ignore_ascii_case(n)))
        .find(|c| !c.grey)
        .or_else(|| p.choices.iter().find(|c| !c.grey))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(w: &str, x: i32, y: i32) -> (String, Rect) {
        (w.into(), Rect::new(x, y, 50, 14))
    }

    #[test]
    fn picks_by_order_skipping_grey() {
        let mut f = Frame::solid(800, 599, 20, 20, 30);
        // white text under Heavenly Force and Adapted Slash, Divine Wings grey
        for (cx, cy) in [(400, 300), (600, 300)] {
            for y in cy - 3..cy + 3 {
                for x in cx - 20..cx + 20 {
                    let i = ((y * 800 + x) * 4) as usize;
                    f.bgra[i..i + 3].copy_from_slice(&[250, 250, 250]);
                }
            }
        }
        let lines = vec![
            vec![
                word("Choose", 300, 100),
                word("an", 355, 100),
                word("Adaptation", 380, 100),
            ],
            vec![word("Divine", 150, 293), word("Wings", 205, 293)],
            vec![word("Heavenly", 350, 293), word("Force", 405, 293)],
            vec![word("Adapted", 550, 293), word("Slash", 605, 293)],
            vec![word("DEF", 380, 380)],
        ];
        let p = read(&lines, &f).unwrap();
        assert_eq!(p.badge, Some(Badge::Def));
        assert_eq!(p.choices.len(), 3);
        assert!(p.choices[0].grey);
        let s = Adaptations::default(); // Divine Wings first
        assert_eq!(choose(&p, &s).unwrap().name, "Heavenly Force");
        assert!(read(&lines[1..], &f).is_none(), "no title, no panel");
    }
}
