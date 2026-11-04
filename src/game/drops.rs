//! Rewards read off the end screen (OCR): Gems, Summer Coins, Star
//! Remnants, ... Rows with a number ("<name> x12", "+12 <name>",
//! "<name> 12"). Tracked names are matched loosely; anything else only
//! counts when its name has Coin or Portal in it (OCR picks up other text).

/// Names that always count (matched loosely).
pub const TRACKED: &[&str] = &[
    "Gems",
    "Summer Coin",
    "Star Remnant",
    "Snow Reaper",
    "Trait Crystal",
];

/// One OCR line -> (item, amount).
pub fn parse_line(line: &str) -> Option<(String, u64)> {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    // the number: "x12", "+12", "12", "12x", "1,200"
    let num = |w: &str| -> Option<u64> {
        let d: String = w
            .trim_start_matches(['x', 'X', '+', '×'])
            .trim_end_matches(['x', 'X'])
            .chars()
            .filter(|c| *c != ',')
            .collect();
        if !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()) && d.len() <= 7 {
            d.parse().ok()
        } else {
            None
        }
    };
    let i = words.iter().position(|w| num(w).is_some())?;
    let amount = num(words[i])?;
    if amount == 0 {
        return None;
    }
    let name_words: Vec<&str> = words
        .iter()
        .enumerate()
        .filter(|(j, w)| *j != i && w.chars().any(|c| c.is_alphabetic()))
        .map(|(_, w)| *w)
        .collect();
    let name = name_words.join(" ");
    let low = name.to_lowercase();
    if low.is_empty() {
        return None;
    }
    // loose matches for what OCR does to the names
    if low.contains("remn") {
        return Some(("Star Remnant".into(), amount));
    }
    if low.contains("summer") {
        return Some(("Summer Coin".into(), amount));
    }
    if low.contains("gem") {
        return Some(("Gems".into(), amount));
    }
    for t in TRACKED {
        if low.contains(&t.to_lowercase()) {
            return Some((t.to_string(), amount));
        }
    }
    if low.contains("coin") || low.contains("portal") {
        let clean: String = name
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '\'')
            .collect();
        return Some((clean.trim().to_string(), amount));
    }
    None
}

/// Every reward on the screen, one entry per item (amounts of repeated
/// rows add up).
pub fn parse(lines: &[String]) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = vec![];
    for l in lines {
        if let Some((n, a)) = parse_line(l) {
            match out.iter_mut().find(|(m, _)| *m == n) {
                Some(e) => e.1 += a,
                None => out.push((n, a)),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rewards() {
        let lines: Vec<String> = [
            "VICTORY",
            "Wave 25",
            "Gems x50",
            "+12 Summer Coins",
            "Star Remnamt 3",
            "Raid Portal x1",
            "Time 12:30",
            "Yen 9000",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let d = parse(&lines);
        assert!(d.contains(&("Gems".into(), 50)));
        assert!(d.contains(&("Summer Coin".into(), 12)));
        assert!(d.contains(&("Raid Portal".into(), 1)));
        assert!(
            d.contains(&("Star Remnant".into(), 3)),
            "OCR slip in the name"
        );
        assert_eq!(d.len(), 4, "{d:?}");
        assert_eq!(
            parse_line("Star Remnant x2"),
            Some(("Star Remnant".into(), 2))
        );
    }
}
