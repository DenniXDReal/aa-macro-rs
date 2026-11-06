//! Backup for picture matching: find a button by reading its label with OCR
//! ("Start", "Vote Start", "Next", ...). Used when the captured picture isn't
//! found (or was never captured).

use crate::geom::{Point, Rect};

/// Pattern name -> labels the button can show. One-word labels must be the
/// whole text line ("Start" never matches "Vote Start") unless they start
/// with "~"; longer labels may sit inside a line.
pub const LABELS: &[(&str, &[&str])] = &[
    ("start_button", &["start"]),
    // the vote box's "Yes" may share its line with a count ("Yes 0/2")
    ("vote_start", &["vote start", "~yes"]),
    ("play_here", &["play here"]),
    ("join_matchmaking", &["join matchmaking", "matchmaking"]),
    ("find_match", &["find match"]),
    ("next_button", &["next"]),
    ("replay", &["replay"]),
    ("return_to_lobby", &["return to lobby", "return lobby"]),
    ("party_invite", &["invite"]),
    ("party_accept", &["accept"]),
    ("party_invited", &["invited"]),
    ("select_button", &["select"]),
    ("play_button", &["play"]),
    ("areas_button", &["areas"]),
    ("raids_button", &["raids"]),
    ("open_button", &["open"]),
    ("open_portal", &["open portal"]),
    ("card_confirm", &["confirm"]),
    ("reconnect_button", &["reconnect"]),
    (
        "tp_spawn",
        &[
            "teleport back to spawn",
            "teleport to spawn",
            "back to spawn",
        ],
    ),
    ("legend_stages_toggle", &["legend stages"]),
    ("main_stages_toggle", &["main stages"]),
];

/// Labels for a (normalised) pattern name.
pub fn labels_for(name: &str) -> Option<&'static [&'static str]> {
    LABELS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

fn flat(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            // usual OCR slips on game fonts
            '0' => 'o',
            '1' | '|' | 'l' => 'i',
            c => c,
        })
        .collect()
}

/// Centres of every place one of `labels` is written, top to bottom.
pub fn find_all(
    lines: &[Vec<(String, Rect)>],
    labels: &[&str],
    within: Option<Rect>,
) -> Vec<Point> {
    let mut out: Vec<Point> = vec![];
    for line in lines {
        for label in labels {
            let loose = label.starts_with('~');
            let want: Vec<String> = label
                .trim_start_matches('~')
                .split_whitespace()
                .map(flat)
                .collect();
            let n = want.len();
            if n == 0 || line.len() < n {
                continue;
            }
            // one word labels: the line is just that word (unless "~")
            if n == 1 && line.len() != 1 && !loose {
                continue;
            }
            for i in 0..=line.len() - n {
                let ok = (0..n).all(|j| flat(&line[i + j].0) == want[j]);
                if !ok {
                    continue;
                }
                let r = line[i..i + n]
                    .iter()
                    .map(|(_, r)| *r)
                    .reduce(|a, b| {
                        let x = a.x.min(b.x);
                        let y = a.y.min(b.y);
                        let x2 = (a.x + a.w).max(b.x + b.w);
                        let y2 = (a.y + a.h).max(b.y + b.h);
                        Rect::new(x, y, x2 - x, y2 - y)
                    })
                    .unwrap();
                let c = Point::new(r.x + r.w / 2, r.y + r.h / 2);
                let inside = within.map_or(true, |w| {
                    c.x >= w.x && c.y >= w.y && c.x < w.x + w.w && c.y < w.y + w.h
                });
                if inside
                    && !out
                        .iter()
                        .any(|p| (p.x - c.x).abs() < 6 && (p.y - c.y).abs() < 6)
                {
                    out.push(c);
                }
            }
        }
    }
    out.sort_by_key(|p| (p.y, p.x));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(words: &[(&str, i32)]) -> Vec<(String, Rect)> {
        words
            .iter()
            .map(|(w, x)| (w.to_string(), Rect::new(*x, 100, 40, 20)))
            .collect()
    }

    #[test]
    fn finds_labels() {
        let lines = vec![
            line(&[("Vote", 300), ("Start", 345)]),
            line(&[("START", 600)]),
            line(&[("Return", 100), ("to", 145), ("Lobby", 170)]),
        ];
        let start = labels_for("start_button").unwrap();
        // "Start" alone only, not the one in "Vote Start"
        assert_eq!(find_all(&lines, start, None), vec![Point::new(620, 110)]);
        let vote = &["vote start"];
        assert_eq!(find_all(&lines, vote, None), vec![Point::new(342, 110)]);
        let back = labels_for("return_to_lobby").unwrap();
        assert_eq!(find_all(&lines, back, None).len(), 1);
        // outside the area -> nothing
        assert!(find_all(&lines, start, Some(Rect::new(0, 0, 200, 200))).is_empty());
        assert!(labels_for("settings_button").is_none());
    }

    #[test]
    fn vote_yes_with_a_count() {
        let lines = vec![line(&[("Yes", 300), ("0/2", 345)]), line(&[("No", 500)])];
        let v = labels_for("vote_start").unwrap();
        assert_eq!(find_all(&lines, v, None), vec![Point::new(320, 110)]);
    }

    #[test]
    fn finds_every_invite() {
        let lines = vec![
            line(&[("Invite", 500)]),
            vec![("lnvite".into(), Rect::new(500, 160, 40, 20))],
            line(&[("Invited", 500)]),
        ];
        assert_eq!(
            find_all(&lines, labels_for("party_invite").unwrap(), None).len(),
            2
        );
    }
}
