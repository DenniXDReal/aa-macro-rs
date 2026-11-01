//! Tiling up to N Roblox windows on one screen.
//!
//! Clients stay at the size patterns are captured for (800x599). When the grid
//! doesn't fit, rows/columns overlap evenly. That's fine: capture works on
//! covered windows and the arbiter brings a window to the front before input.

use crate::geom::Rect;

/// Outer rects for `n` windows of `outer_w x outer_h` inside `work` (the
/// screen's work area). 1 => single, 2 => side by side, 3-4 => 2x2,
/// 5-6 => 3x2, larger => ceil(sqrt) grid.
pub fn tile(n: usize, work: Rect, outer_w: i32, outer_h: i32) -> Vec<Rect> {
    if n == 0 {
        return vec![];
    }
    let cols = match n {
        1 => 1,
        2..=4 => 2,
        5..=6 => 3,
        _ => (n as f64).sqrt().ceil() as usize,
    };
    let rows = n.div_ceil(cols);
    let step = |count: usize, avail: i32, size: i32| -> i32 {
        if count <= 1 {
            0
        } else {
            // Exact fit when there is room, else overlap evenly.
            ((avail - size) / (count as i32 - 1)).min(size).max(0)
        }
    };
    let sx = step(cols, work.w, outer_w);
    let sy = step(rows, work.h, outer_h);
    (0..n)
        .map(|i| {
            let c = (i % cols) as i32;
            let r = (i / cols) as i32;
            Rect::new(work.x + c * sx, work.y + r * sy, outer_w, outer_h)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_on_1080p_overlap_vertically() {
        let work = Rect::new(0, 0, 1920, 1040);
        let r = tile(4, work, 816, 638);
        assert_eq!(r.len(), 4);
        // Fits side by side horizontally (2*816 <= 1920): no horizontal overlap.
        assert_eq!(r[0].x, 0);
        assert_eq!(r[1].x, 816);
        // Vertically overlaps: second row starts at 1040-638 = 402.
        assert_eq!(r[2].y, 402);
        for x in &r {
            assert!(x.right() <= 1920 && x.bottom() <= 1040);
        }
    }

    #[test]
    fn four_on_1440p_no_overlap() {
        let r = tile(4, Rect::new(0, 0, 2560, 1400), 816, 638);
        assert_eq!(r[3], Rect::new(816, 638, 816, 638));
    }

    #[test]
    fn single_and_zero() {
        assert!(tile(0, Rect::new(0, 0, 100, 100), 10, 10).is_empty());
        assert_eq!(
            tile(1, Rect::new(5, 6, 100, 100), 10, 10),
            vec![Rect::new(5, 6, 10, 10)]
        );
    }
}
