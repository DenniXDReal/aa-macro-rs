//! Card pick screen (event modifiers): take the green card if there is one,
//! otherwise the middle card - same fallback spot the AHK macro uses.

use crate::capture::Frame;
use crate::geom::{Point, Rect};

/// Where the cards sit (client px, 800x599).
pub const CARD_AREA: Rect = Rect {
    x: 80,
    y: 170,
    w: 640,
    h: 300,
};
/// The AHK macro's "middle card" click.
pub const MIDDLE: Point = Point { x: 430, y: 303 };

/// Strong, clearly green pixel (card border / glow), not grey or yellow.
fn green(r: u8, g: u8, b: u8) -> bool {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    g >= 120 && g - r >= 50 && g - b >= 40
}

/// The green card's centre, if a card in the card area is green.
pub fn green_card(f: &Frame) -> Option<Point> {
    let a = CARD_AREA;
    // green pixel count per column, then the biggest run of green columns
    let mut cols = vec![0u32; a.w as usize];
    let mut ys: Vec<Vec<i32>> = vec![vec![]; a.w as usize];
    for x in 0..a.w {
        for y in 0..a.h {
            if let Some((r, g, b)) = f.rgb(a.x + x, a.y + y) {
                if green(r, g, b) {
                    cols[x as usize] += 1;
                    ys[x as usize].push(a.y + y);
                }
            }
        }
    }
    let mut best: Option<(usize, usize, u32)> = None; // (from, to, total)
    let mut i = 0;
    while i < cols.len() {
        if cols[i] < 3 {
            i += 1;
            continue;
        }
        let from = i;
        let mut total = 0;
        let mut gap = 0;
        let mut to = i;
        while i < cols.len() && gap <= 6 {
            if cols[i] >= 3 {
                total += cols[i];
                to = i;
                gap = 0;
            } else {
                gap += 1;
            }
            i += 1;
        }
        if best.is_none_or(|b| total > b.2) {
            best = Some((from, to, total));
        }
    }
    let (from, to, total) = best?;
    // a real card: wide enough and a fair amount of green
    if to - from < 40 || total < 300 {
        return None;
    }
    let mut all: Vec<i32> = ys[from..=to].iter().flatten().copied().collect();
    all.sort_unstable();
    let y = (all[0] + all[all.len() - 1]) / 2;
    Some(Point::new(a.x + ((from + to) / 2) as i32, y))
}

/// Card to click: green one, else the middle.
pub fn pick(f: &Frame) -> (Point, bool) {
    match green_card(f) {
        Some(p) => (p, true),
        None => (MIDDLE, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paint(f: &mut Frame, r: Rect, rgb: (u8, u8, u8)) {
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                let i = ((y as u32 * f.width + x as u32) * 4) as usize;
                f.bgra[i..i + 3].copy_from_slice(&[rgb.2, rgb.1, rgb.0]);
            }
        }
    }

    /// Three cards with a 4 px coloured border.
    fn cards(colours: [(u8, u8, u8); 3]) -> Frame {
        let mut f = Frame::solid(800, 599, 20, 20, 25);
        for (i, c) in colours.iter().enumerate() {
            let x = 170 + i as i32 * 180;
            paint(&mut f, Rect::new(x, 200, 140, 200), *c);
            paint(&mut f, Rect::new(x + 4, 204, 132, 192), (40, 35, 50));
        }
        f
    }

    #[test]
    fn picks_the_green_card() {
        let f = cards([(200, 40, 40), (60, 200, 70), (50, 90, 220)]);
        let (p, green) = pick(&f);
        assert!(green);
        assert!(
            (350..=420).contains(&p.x) && (280..=320).contains(&p.y),
            "{p:?}"
        );
        let f = cards([(60, 200, 70), (200, 40, 40), (50, 90, 220)]);
        assert!((200..=270).contains(&pick(&f).0.x));
    }

    #[test]
    fn no_green_means_middle() {
        let f = cards([(200, 40, 40), (220, 200, 40), (50, 90, 220)]);
        assert_eq!(pick(&f), (MIDDLE, false));
    }
}

/// The three cards (client px): left, middle, right.
pub const CARD_BOXES: [Rect; 3] = [
    Rect {
        x: 145,
        y: 200,
        w: 160,
        h: 225,
    },
    Rect {
        x: 320,
        y: 200,
        w: 160,
        h: 225,
    },
    Rect {
        x: 497,
        y: 200,
        w: 160,
        h: 225,
    },
];

/// Title strip of card `i` (where "+ Enemy Speed I" is written).
pub fn title_box(i: usize) -> Rect {
    let b = CARD_BOXES[i];
    Rect::new(b.x + 4, b.y + 12, b.w - 8, 34)
}

/// Every Summer Event card seen so far (from your cards/ screenshots).
/// Green = helps you, red = buffs enemies but pays Summer Pearls.
pub const CARD_BASES: &[(&str, bool)] = &[
    // green (no pearls)
    ("Random Blessings", true),
    ("Attack", true),
    ("Boss Damage", true),
    ("Active Cooldown", true),
    ("Cooldown", true),
    ("Range", true),
    ("Yen", true),
    // red (Summer Pearls: I +30%, II +70% & +25% secret drops, III +150% & +50%)
    ("Explosive Deaths", false),
    ("Enemy Regen", false),
    ("Enemy Shield", false),
    ("Spellproof Enemies", false),
    ("Plated Enemies", false),
    ("Resistant Enemies", false),
    ("Enemy Health", false),
    ("Enemy Speed", false),
    ("Random Curses", false),
];

/// Default pick order (farming Summer Pearls + secret portal drops): the
/// highest tier pays the most (III: +150% pearls, +50% secret drops), and
/// Plated (physical resist), Enemy Health and Enemy Speed go first, as you
/// pick them. Other red cards next, Random Curses after, green cards last.
/// Change it in Settings > Card priority.
pub fn default_priority() -> Vec<String> {
    let fav = ["Plated Enemies", "Enemy Health", "Enemy Speed"];
    let rest = [
        "Enemy Regen",
        "Enemy Shield",
        "Explosive Deaths",
        "Spellproof Enemies",
        "Resistant Enemies",
    ];
    let mut v = vec![];
    for t in ["III", "II", "I"] {
        for b in fav {
            v.push(format!("{b} {t}"));
        }
    }
    for t in ["III", "II", "I"] {
        for b in rest {
            v.push(format!("{b} {t}"));
        }
    }
    for t in ["II", "I"] {
        v.push(format!("Random Curses {t}"));
    }
    for t in ["III", "II", "I"] {
        for b in ["Random Blessings", "Attack"] {
            v.push(format!("{b} {t}"));
        }
    }
    for c in [
        "Boss Damage I",
        "Active Cooldown II",
        "Cooldown II",
        "Range II",
        "Active Cooldown I",
        "Cooldown I",
        "Range I",
        "Yen III",
        "Yen II",
        "Yen I",
    ] {
        v.push(c.to_string());
    }
    v
}

/// OCR text of a card title -> "Enemy Speed II" (None if not a known card).
pub fn card_name(text: &str) -> Option<String> {
    let flat: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let low = flat.to_lowercase();
    // longest base first ("Active Cooldown" before "Cooldown")
    let mut bases: Vec<&(&str, bool)> = CARD_BASES.iter().collect();
    bases.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
    for (b, _) in bases {
        let key: String = b
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_lowercase();
        if let Some(i) = low.find(&key) {
            // tier = the I / l / | / ! marks right after the name
            let tail = &flat[i + key.len()..];
            let n = tail
                .chars()
                .take_while(|c| matches!(c, 'I' | 'l' | '|' | '!' | '1' | 'i'))
                .count()
                .clamp(1, 3);
            let tier = ["I", "II", "III"][n - 1];
            return Some(format!("{b} {tier}"));
        }
    }
    None
}

/// Is this card name a green (helps you) card?
pub fn is_green(name: &str) -> bool {
    CARD_BASES.iter().any(|(b, g)| *g && name.starts_with(b))
}

/// Which card to take: the first in `priority` that's on offer.
pub fn choose(names: &[Option<String>; 3], priority: &[String]) -> Option<usize> {
    priority
        .iter()
        .find_map(|p| names.iter().position(|n| n.as_deref() == Some(p.as_str())))
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn names_from_ocr() {
        assert_eq!(card_name("+ EnemySpeed!").as_deref(), Some("Enemy Speed I"));
        assert_eq!(
            card_name("EnemyHealthll").as_deref(),
            Some("Enemy Health II")
        );
        assert_eq!(
            card_name("EnemyHealthIll").as_deref(),
            Some("Enemy Health III")
        );
        assert_eq!(card_name("Rangel").as_deref(), Some("Range I"));
        assert_eq!(card_name("Attack").as_deref(), Some("Attack I"));
        assert_eq!(
            card_name("-ActiveCooldown ll").as_deref(),
            Some("Active Cooldown II")
        );
        assert_eq!(
            card_name("+Random BlessingsIl").as_deref(),
            Some("Random Blessings II")
        );
        assert_eq!(
            card_name("+ResistantEnemiesIll").as_deref(),
            Some("Resistant Enemies III")
        );
        assert_eq!(card_name("Wave 3"), None);
        let p = default_priority();
        let o = |a: &str, b: &str, c: &str| {
            [
                Some(a.to_string()),
                Some(b.to_string()),
                Some(c.to_string()),
            ]
        };
        assert_eq!(
            choose(&o("Attack I", "Enemy Speed I", "Yen I"), &p),
            Some(1),
            "pearls over green"
        );
        assert_eq!(
            choose(
                &o("Plated Enemies I", "Enemy Regen III", "Enemy Health II"),
                &p
            ),
            Some(2),
            "favourite at a higher tier"
        );
        assert_eq!(
            choose(&o("Enemy Regen I", "Plated Enemies I", "Attack III"), &p),
            Some(1)
        );
        assert!(is_green("Active Cooldown I") && !is_green("Enemy Regen II"));
    }
}
