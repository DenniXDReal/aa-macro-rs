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
