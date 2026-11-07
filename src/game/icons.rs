//! Small built-in pictures found anywhere on screen: the dungeon room icons,
//! the Dungeon / Enter Dungeon buttons, the captcha title. Only a picture's
//! opaque pixels are compared (its background is cut away), so an icon is
//! found on any tile colour.

use crate::capture::Frame;
use crate::geom::{Point, Rect};

pub struct Icon {
    pub name: &'static str,
    pub w: i32,
    pub h: i32,
    /// Opaque pixels: (dx, dy, r, g, b).
    px: Vec<(i32, i32, u8, u8, u8)>,
}

impl Icon {
    pub fn from_png(name: &'static str, bytes: &[u8]) -> Option<Icon> {
        let f = Frame::from_png_bytes(bytes).ok()?;
        let mut px = vec![];
        for y in 0..f.height as i32 {
            for x in 0..f.width as i32 {
                let i = ((y as u32 * f.width + x as u32) * 4) as usize;
                if f.bgra[i + 3] >= 128 {
                    px.push((x, y, f.bgra[i + 2], f.bgra[i + 1], f.bgra[i]));
                }
            }
        }
        (!px.is_empty()).then_some(Icon {
            name,
            w: f.width as i32,
            h: f.height as i32,
            px,
        })
    }

    /// Every place in `area` where at least `min_match` of the icon's
    /// pixels are within `tol` per channel: (middle, score), best first,
    /// overlapping hits merged.
    pub fn find_all(&self, f: &Frame, area: Rect, tol: u8, min_match: f32) -> Vec<(Point, f32)> {
        let n = self.px.len();
        let allowed = ((1.0 - min_match) * n as f32).floor() as usize;
        let x1 = (area.x + area.w).min(f.width as i32) - self.w;
        let y1 = (area.y + area.h).min(f.height as i32) - self.h;
        let fw = f.width as i32;
        let tol = tol as i32;
        let close = |i: usize, r: u8, g: u8, b: u8| {
            (f.bgra[i + 2] as i32 - r as i32).abs() <= tol
                && (f.bgra[i + 1] as i32 - g as i32).abs() <= tol
                && (f.bgra[i] as i32 - b as i32).abs() <= tol
        };
        let mut hits: Vec<(Point, f32)> = vec![];
        for y in area.y.max(0)..=y1 {
            for x in area.x.max(0)..=x1 {
                let mut bad = 0;
                let mut ok = true;
                for &(dx, dy, r, g, b) in &self.px {
                    let i = (((y + dy) * fw + x + dx) * 4) as usize;
                    if !close(i, r, g, b) {
                        bad += 1;
                        if bad > allowed {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    let score = 1.0 - bad as f32 / n as f32;
                    hits.push((Point::new(x + self.w / 2, y + self.h / 2), score));
                }
            }
        }
        // best first, one hit per spot
        hits.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut out: Vec<(Point, f32)> = vec![];
        for h in hits {
            if out
                .iter()
                .all(|o| (o.0.x - h.0.x).abs() >= self.w || (o.0.y - h.0.y).abs() >= self.h)
            {
                out.push(h);
            }
        }
        out
    }

    /// The best place in the whole frame, if any.
    pub fn find(&self, f: &Frame, tol: u8, min_match: f32) -> Option<Point> {
        let all = Rect::new(0, 0, f.width as i32, f.height as i32);
        self.find_all(f, all, tol, min_match).first().map(|h| h.0)
    }
}

macro_rules! icon {
    ($name:literal) => {
        Icon::from_png(
            $name,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/dungeon/",
                $name,
                ".png"
            )),
        )
    };
}

/// The built-in pictures, loaded once.
pub fn get(name: &str) -> Option<&'static Icon> {
    static ALL: std::sync::OnceLock<Vec<Icon>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        [
            icon!("icon_hoard"),
            icon!("icon_hoard_big"),
            icon!("icon_chest"),
            icon!("icon_rare_chest"),
            icon!("icon_shop"),
            icon!("icon_shrine"),
            icon!("icon_curse"),
            icon!("button_dungeon"),
            icon!("button_enter_dungeon"),
            icon!("captcha_title"),
            icon!("button_next"),
        ]
        .into_iter()
        .flatten()
        .collect()
    })
    .iter()
    .find(|i| i.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload(n: &str) -> Frame {
        Frame::load_png(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(n),
        )
        .unwrap()
    }

    #[test]
    fn all_load() {
        for n in [
            "icon_hoard",
            "icon_hoard_big",
            "icon_chest",
            "icon_rare_chest",
            "icon_shop",
            "icon_shrine",
            "icon_curse",
            "button_dungeon",
            "button_enter_dungeon",
            "captcha_title",
        ] {
            assert!(get(n).is_some(), "{n}");
        }
    }

    #[test]
    fn captcha_title_on_the_dialog() {
        let f = upload("captcha.png");
        let p = get("captcha_title").unwrap().find(&f, 40, 0.85);
        let p = p.expect("found");
        assert!((p.x - 200).abs() < 6 && (p.y - 21).abs() < 6, "{p:?}");
        // not on the dungeon buttons
        assert!(get("captcha_title")
            .unwrap()
            .find(&upload("dungeon_enter.png"), 40, 0.85)
            .is_none());
    }
}
