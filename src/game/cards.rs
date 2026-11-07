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

/// The tier from the characters right after a card's name: I / II / III
/// (OCR reads I as l | ! 1 i too; "II" + "ncreases" = the last I starts the
/// next word).
fn tier_after(tail: &[char]) -> &'static str {
    let mut n = tail
        .iter()
        .take_while(|c| matches!(c, 'I' | 'l' | '|' | '!' | '1' | 'i'))
        .count();
    if n > 0 && tail.get(n).is_some_and(|c| c.is_lowercase()) {
        n -= 1;
    }
    ["I", "II", "III"][n.clamp(1, 3) - 1]
}

/// OCR text of a card title -> "Enemy Speed II" (None if not a known card).
/// The exact name first; else the closest name with at most one slip per
/// 4 letters ("Atfack III" = Attack III).
pub fn card_name(text: &str) -> Option<String> {
    let flat: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    let low: Vec<char> = flat.iter().map(|c| c.to_ascii_lowercase()).collect();
    let key = |b: &str| -> Vec<char> {
        b.chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    // longest base first ("Active Cooldown" before "Cooldown")
    let mut bases: Vec<&(&str, bool)> = CARD_BASES.iter().collect();
    bases.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
    for (b, _) in &bases {
        let k = key(b);
        if let Some(i) = low.windows(k.len()).position(|w| w == k.as_slice()) {
            return Some(format!("{b} {}", tier_after(&flat[i + k.len()..])));
        }
    }
    // closest name against every stretch of about its length
    let mut best: Option<(f32, usize, &str, usize)> = None;
    for (b, _) in &bases {
        let k = key(b);
        if k.len() < 5 {
            continue; // "Yen", "Range": too short to guess at
        }
        let ks: String = k.iter().collect();
        let max = k.len() / 4;
        for len in k.len() - 1..=k.len() + 1 {
            for start in 0..=low.len().saturating_sub(len) {
                if start + len > low.len() {
                    break;
                }
                let w: String = low[start..start + len].iter().collect();
                let d = super::weather::edits(&w, &ks);
                let ratio = d as f32 / k.len() as f32;
                if d <= max && best.is_none_or(|(r, ..)| ratio < r) {
                    best = Some((ratio, d, b, start + len));
                }
            }
        }
    }
    best.map(|(_, _, b, end)| format!("{b} {}", tier_after(&flat[end..])))
}

/// What OCR read over each card (left / middle / right), shortened - for
/// the log.
pub fn column_texts(lines: &[Vec<(String, Rect)>]) -> [String; 3] {
    let col = |r: &Rect| {
        let x = r.x + r.w / 2;
        (0..3)
            .min_by_key(|k| (CARD_BOXES[*k].x + CARD_BOXES[*k].w / 2 - x).abs())
            .unwrap_or(1)
    };
    let mut out: [String; 3] = Default::default();
    for (i, o) in out.iter_mut().enumerate() {
        let t: Vec<&str> = lines
            .iter()
            .flatten()
            .filter(|(_, r)| col(r) == i)
            .map(|(w, _)| w.as_str())
            .collect();
        let t = t.join(" ");
        *o = if t.chars().count() > 50 {
            format!("{}...", t.chars().take(50).collect::<String>())
        } else {
            t
        };
    }
    out
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
    let base = |n: &str| {
        n.trim_end_matches(" III")
            .trim_end_matches(" II")
            .trim_end_matches(" I")
            .to_string()
    };
    // each card with its place on the list
    let offered = shown
        .iter()
        .map(|n| {
            if n == "?" {
                return "? (not read)".to_string();
            }
            match priority.iter().position(|p| p.eq_ignore_ascii_case(n)) {
                Some(r) => format!("{n} #{}", r + 1),
                None => match priority.iter().position(|p| base(p) == base(n)) {
                    Some(r) => format!("{n} ~#{}", r + 1),
                    None => format!("{n} (not on list)"),
                },
            }
        })
        .collect::<Vec<_>>()
        .join(" / ");
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
    // OCR can join the three titles into one line: split each line by card
    // first (each word to the card box it sits over)
    let col = |r: &Rect| {
        let x = r.x + r.w / 2;
        (0..3)
            .min_by_key(|k| (CARD_BOXES[*k].x + CARD_BOXES[*k].w / 2 - x).abs())
            .unwrap_or(1)
    };
    for l in lines {
        for (i, slot) in out.iter_mut().enumerate() {
            if slot.is_some() {
                continue;
            }
            let text: String = l
                .iter()
                .filter(|(_, r)| col(r) == i)
                .map(|(w, _)| w.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(name) = card_name(&text) {
                *slot = Some(name);
            }
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
            "Card Picker: Chose Attack I because it was #2 on the priority list (offered: Cooldown I #3 / Range I (not on list) / Attack I #2)"
        );
        // same card at another tier
        let names2: [Option<String>; 3] =
            [Some("Enemy Health II".into()), None, Some("Range I".into())];
        let c2 = choose(&names2, &prio);
        assert_eq!(c2, Some(0));
        assert_eq!(
            picker_line("priority list", &names2, &prio, c2, 1, "middle one"),
            "Card Picker: Chose Enemy Health II because it was #1 on the priority list as Enemy Health III - other tier (offered: Enemy Health II ~#1 / ? (not read) / Range I (not on list))"
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

    #[test]
    fn titles_joined_on_one_line() {
        // OCR read all three titles as one line
        let w = |t: &str, i: usize, dx: i32| {
            let b = CARD_BOXES[i];
            (t.to_string(), Rect::new(b.x + dx, b.y + 20, 30, 14))
        };
        let lines = vec![vec![
            w("Enemy", 0, 10),
            w("Speed", 0, 50),
            w("II", 0, 90),
            w("Attack", 1, 20),
            w("I", 1, 70),
            w("Range", 2, 20),
            w("III", 2, 70),
        ]];
        let n = names_from_lines(&lines);
        assert_eq!(n[0].as_deref(), Some("Enemy Speed II"));
        assert_eq!(n[1].as_deref(), Some("Attack I"));
        assert_eq!(n[2].as_deref(), Some("Range III"));
    }

    #[test]
    fn tier_not_eaten_by_the_next_word() {
        assert_eq!(
            card_name("Enemy Speed IIncreases").as_deref(),
            Some("Enemy Speed I")
        );
        assert_eq!(
            card_name("Enemy Speed II").as_deref(),
            Some("Enemy Speed II")
        );
        assert_eq!(card_name("Attack III").as_deref(), Some("Attack III"));
    }

    /// The pick follows your list - not the leftmost card - whatever
    /// position the best card is in, and through small OCR slips.
    #[test]
    fn picks_by_your_list_not_position() {
        let prio = default_priority();
        let w = |t: &str, i: usize| {
            let b = CARD_BOXES[i];
            vec![(t.to_string(), Rect::new(b.x + 20, b.y + 20, 80, 14))]
        };
        // best card on the right
        let lines = vec![w("Yen I", 0), w("Range II", 1), w("Enemy Health III", 2)];
        let n = names_from_lines(&lines);
        assert_eq!(choose(&n, &prio), Some(2));
        // OCR slips: "Atfack III", "Enemv Speed II", "Rangc I"
        let lines = vec![w("Rangc I", 0), w("Atfack III", 1), w("Enemv Speed II", 2)];
        let n = names_from_lines(&lines);
        assert_eq!(n[1].as_deref(), Some("Attack III"));
        assert_eq!(n[2].as_deref(), Some("Enemy Speed II"));
        // Enemy Speed II is above Attack III on the default list
        assert_eq!(choose(&n, &prio), Some(2));
        // the log says what was read and why
        let line = picker_line("priority list", &n, &prio, Some(2), 0, "");
        assert!(
            line.contains("Chose Enemy Speed II because it was #"),
            "{line}"
        );
        assert!(line.contains("Attack III #"), "{line}");
        // nothing readable: not "the list" - the fallback says so
        let none: [Option<String>; 3] = [None, None, None];
        assert!(
            picker_line("priority list", &none, &prio, None, 1, "the middle one")
                .contains("none of the cards could be read")
        );
        // short names aren't guessed from junk
        assert_eq!(card_name("Yan"), None);
        assert_eq!(card_name("Choose a card"), None);
    }
}
