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

/// The card's text under its title ("+70% SummerPearls", the effect).
pub fn body_box(i: usize) -> Rect {
    let b = CARD_BOXES[i];
    Rect::new(b.x + 6, b.y + 45, b.w - 12, 85)
}

/// The vote count under a card ("0", "1", ...).
pub fn count_box(i: usize) -> Rect {
    let b = CARD_BOXES[i];
    Rect::new(b.x + b.w / 2 - 24, b.y + 208, 48, 28)
}

/// A red card's tier from the pearls it pays: +30% = I, +70% = II,
/// +150% = III (more reliable than reading "II" / "III" in the title).
pub fn tier_from_pearls(text: &str) -> Option<&'static str> {
    let t: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    let i = t.find("%summer").or_else(|| t.find("%pearl"))?;
    let num: String = t[..i]
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    match num.parse::<u32>().ok()? {
        30 => Some("I"),
        70 => Some("II"),
        150 => Some("III"),
        _ => None,
    }
}

/// "Enemy Health I" with the tier replaced.
pub fn with_tier(name: &str, tier: &str) -> String {
    let base = name
        .trim_end_matches(" III")
        .trim_end_matches(" II")
        .trim_end_matches(" I");
    format!("{base} {tier}")
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

/// Default pick order: Enemy Health and Enemy Speed first (highest tier
/// first - most Summer Pearls + secret drops), then the green buff cards,
/// then the rest of the red modifiers. Change it in Settings > Card priority.
pub fn default_priority() -> Vec<String> {
    let mut v = vec![];
    for t in ["III", "II", "I"] {
        for b in ["Enemy Health", "Enemy Speed"] {
            v.push(format!("{b} {t}"));
        }
    }
    for t in ["III", "II", "I"] {
        for b in ["Random Blessings", "Attack"] {
            v.push(format!("{b} {t}"));
        }
    }
    for c in [
        "Boss Damage III",
        "Boss Damage II",
        "Boss Damage I",
        "Active Cooldown III",
        "Active Cooldown II",
        "Cooldown III",
        "Cooldown II",
        "Range III",
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
    let rest = [
        "Plated Enemies",
        "Enemy Regen",
        "Enemy Shield",
        "Explosive Deaths",
        "Spellproof Enemies",
        "Resistant Enemies",
    ];
    for t in ["III", "II", "I"] {
        for b in rest {
            v.push(format!("{b} {t}"));
        }
    }
    for t in ["III", "II", "I"] {
        v.push(format!("Random Curses {t}"));
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

/// The log line after a card choice: which card, where it is on the list
/// (or why it wasn't picked from the list), what was offered.
/// `list` = "priority list" / "weather list"; `chosen` = what choose() /
/// weather::pick() said (None = fallback `fallback_i`, `fallback_why`).
pub fn picker_line(
    list: &str,
    names: &[Option<String>; 3],
    priority: &[String],
    chosen: Option<usize>,
    fallback_i: usize,
    fallback_why: &str,
) -> String {
    let shown: Vec<String> = names
        .iter()
        .map(|n| n.clone().unwrap_or_else(|| "?".into()))
        .collect();
    let offered = shown.join(" / ");
    let base = |n: &str| {
        n.trim_end_matches(" III")
            .trim_end_matches(" II")
            .trim_end_matches(" I")
            .to_string()
    };
    match chosen {
        Some(i) => {
            let n = &shown[i];
            match priority.iter().position(|p| p.eq_ignore_ascii_case(n)) {
                Some(r) => format!(
                    "Card Picker: Chose {n} because it was #{} on the {list} (offered: {offered})",
                    r + 1
                ),
                None => match priority.iter().position(|p| base(p) == base(n)) {
                    Some(r) => format!(
                        "Card Picker: Chose {n} because it was #{} on the {list} as {} - other tier (offered: {offered})",
                        r + 1,
                        priority[r]
                    ),
                    None => format!("Card Picker: Chose {n} (offered: {offered})"),
                },
            }
        }
        None => {
            let why = if names.iter().all(|n| n.is_none()) {
                "none of the cards could be read".to_string()
            } else {
                format!("none of them is on the {list}")
            };
            format!(
                "Card Picker: {why} - took card {} ({fallback_why}) (offered: {offered})",
                fallback_i + 1
            )
        }
    }
}

/// Which card to take: the first in `priority` that's on offer.
pub fn choose(names: &[Option<String>; 3], priority: &[String]) -> Option<usize> {
    // the card's place in your list: its exact name (with tier), else -
    // when OCR got the tier wrong / missed it - the best entry of that card
    let base = |n: &str| {
        n.trim_end_matches(" III")
            .trim_end_matches(" II")
            .trim_end_matches(" I")
            .to_string()
    };
    let rank = |n: &str| -> Option<(usize, u8)> {
        if let Some(i) = priority.iter().position(|p| p == n) {
            return Some((i, 0));
        }
        let b = base(n);
        priority.iter().position(|p| base(p) == b).map(|i| (i, 1))
    };
    names
        .iter()
        .enumerate()
        .filter_map(|(i, n)| n.as_deref().and_then(rank).map(|r| (r, i)))
        .min()
        .map(|(_, i)| i)
}

/// Card names read anywhere on the card screen (OCR lines with positions),
/// given to the card whose box is nearest (left / middle / right).
pub fn names_from_lines(lines: &[Vec<(String, Rect)>]) -> [Option<String>; 3] {
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
        let x = l.iter().map(|(_, r)| r.x + r.w / 2).sum::<i32>() / l.len().max(1) as i32;
        let i = (0..3)
            .min_by_key(|k| (CARD_BOXES[*k].x + CARD_BOXES[*k].w / 2 - x).abs())
            .unwrap_or(1);
        if out[i].is_none() {
            out[i] = Some(name);
        }
    }
    out
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn tier_from_the_pearls_line() {
        assert_eq!(
            tier_from_pearls("+70% SummerPearls +150 Enemy Physical"),
            Some("II")
        );
        assert_eq!(tier_from_pearls("+150%Summer Pearls"), Some("III"));
        assert_eq!(tier_from_pearls("+30% SummerPearls"), Some("I"));
        assert_eq!(tier_from_pearls("+25% Attack"), None);
        assert_eq!(with_tier("Plated Enemies I", "II"), "Plated Enemies II");
    }

    #[test]
    fn picker_says_why() {
        let prio: Vec<String> = ["Enemy Health III", "Attack I", "Cooldown I"]
            .map(String::from)
            .to_vec();
        let names: [Option<String>; 3] = [
            Some("Cooldown I".into()),
            Some("Range I".into()),
            Some("Attack I".into()),
        ];
        let c = choose(&names, &prio);
        assert_eq!(c, Some(2), "Attack I is #2, Cooldown I #3");
        assert_eq!(
            picker_line("priority list", &names, &prio, c, 1, "middle one"),
            "Card Picker: Chose Attack I because it was #2 on the priority list (offered: Cooldown I / Range I / Attack I)"
        );
        // same card at another tier
        let names2: [Option<String>; 3] =
            [Some("Enemy Health II".into()), None, Some("Range I".into())];
        let c2 = choose(&names2, &prio);
        assert_eq!(c2, Some(0));
        assert_eq!(
            picker_line("priority list", &names2, &prio, c2, 1, "middle one"),
            "Card Picker: Chose Enemy Health II because it was #1 on the priority list as Enemy Health III - other tier (offered: Enemy Health II / ? / Range I)"
        );
        // nothing on the list / nothing read: says so, and what it took
        let none: [Option<String>; 3] = [Some("Yen I".into()), Some("Range I".into()), None];
        assert_eq!(choose(&none, &prio), None);
        assert!(
            picker_line("priority list", &none, &prio, None, 1, "middle one").starts_with(
                "Card Picker: none of them is on the priority list - took card 2 (middle one)"
            )
        );
        assert!(picker_line(
            "weather list",
            &[None, None, None],
            &prio,
            None,
            0,
            "leftmost"
        )
        .starts_with("Card Picker: none of the cards could be read - took card 1 (leftmost)"));
    }

    #[test]
    fn priority_survives_tier_misreads() {
        let prio: Vec<String> = ["Enemy Health III", "Attack III", "Enemy Health I"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // exact match wins
        let n = [
            Some("Attack III".into()),
            Some("Enemy Health III".into()),
            None,
        ];
        assert_eq!(choose(&n, &prio), Some(1));
        // "Enemy Health II" isn't in the list: ranks as its best entry (1st)
        let n = [
            Some("Attack III".into()),
            Some("Enemy Health II".into()),
            None,
        ];
        assert_eq!(choose(&n, &prio), Some(1));
        assert_eq!(choose(&[None, None, None], &prio), None);
    }

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
            choose(&o("Attack III", "Enemy Speed I", "Yen I"), &p),
            Some(1),
            "health / speed first"
        );
        assert_eq!(
            choose(&o("Plated Enemies III", "Enemy Regen III", "Attack I"), &p),
            Some(2),
            "buffs before other debuffs"
        );
        assert_eq!(
            choose(
                &o("Enemy Regen I", "Plated Enemies I", "Enemy Shield II"),
                &p
            ),
            Some(2),
            "higher tier first"
        );
        assert!(is_green("Active Cooldown I") && !is_green("Enemy Regen II"));
    }
}
