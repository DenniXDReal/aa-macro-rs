//! The hotbar counter ("1/3") a selected unit shows: how many of it stand /
//! may stand. A second way to know a placement worked (AA Spiral v0.17).

use crate::geom::Rect;

/// Where the counter can be: the hotbar along the bottom of the client.
pub fn counter_area(w: i32, h: i32) -> Rect {
    let top = h * 72 / 100;
    Rect::new(0, top, w, h - top)
}

/// "1/3" -> (1, 3), also inside a longer line ("1/3 900Y"). OCR can read
/// 0 as O and 1 as l / I / |.
pub fn parse_count(text: &str) -> Option<(u32, u32)> {
    let t: Vec<char> = text
        .chars()
        .map(|c| match c {
            'O' | 'o' | 'Q' | 'D' => '0',
            'l' | 'I' | '|' | 'i' | '!' => '1',
            '\\' => '/',
            c => c,
        })
        .collect();
    for (i, c) in t.iter().enumerate() {
        if *c != '/' {
            continue;
        }
        // digits right before the slash (spaces allowed)
        let mut j = i;
        while j > 0 && t[j - 1] == ' ' {
            j -= 1;
        }
        let mut a = String::new();
        while j > 0 && t[j - 1].is_ascii_digit() && a.len() < 2 {
            j -= 1;
            a.insert(0, t[j]);
        }
        let mut k = i + 1;
        while k < t.len() && t[k] == ' ' {
            k += 1;
        }
        let mut b = String::new();
        while k < t.len() && t[k].is_ascii_digit() && b.len() < 2 {
            b.push(t[k]);
            k += 1;
        }
        let (Ok(n), Ok(m)) = (a.parse::<u32>(), b.parse::<u32>()) else {
            continue;
        };
        // a unit may stand at most a handful of times: bigger = other text
        if (1..=9).contains(&m) && n <= m {
            return Some((n, m));
        }
    }
    None
}

/// The game's "Unique units may not be placed with others of its kind":
/// one of this unit already stands.
pub fn unique_blocked(text: &str) -> bool {
    let t = text.to_lowercase();
    t.contains("may not be placed") || (t.contains("unique unit") && t.contains("placed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts() {
        assert_eq!(parse_count("1/3"), Some((1, 3)));
        assert_eq!(parse_count("1/3 900Y"), Some((1, 3)));
        assert_eq!(parse_count("O/4"), Some((0, 4)));
        assert_eq!(parse_count("l / 3"), Some((1, 3)));
        assert_eq!(parse_count("Wave 12/40 ... 2/2"), Some((2, 2)));
        assert_eq!(parse_count("UPG [3]"), None);
        assert_eq!(parse_count("5/3"), None);
        assert_eq!(parse_count("20/20"), None, "not a unit counter");
        assert!(unique_blocked(
            "Unique units may not be placed with others of its kind"
        ));
        assert!(!unique_blocked("Choose a card"));
    }
}
