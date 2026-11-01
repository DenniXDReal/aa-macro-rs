//! Rectangles and the client <-> screen coordinate conversion.
//!
//! All game positions (clicks, placement spots, search regions) are stored in
//! **client space**: pixels inside the Roblox viewport, (0,0) top-left, for an
//! 800x599 client. Screen space is only used at the OS boundary.

use serde::{Deserialize, Serialize};

/// Window handle as a plain integer so it is `Send`/`Sync` and serialisable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Hwnd(pub isize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom()
    }
    pub fn center(&self) -> Point {
        Point::new(self.x + self.w / 2, self.y + self.h / 2)
    }
    /// Intersection, `None` when empty.
    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        (r > x && b > y).then(|| Rect::new(x, y, r - x, b - y))
    }
}

/// Where a Roblox window is right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub hwnd: Hwnd,
    /// Outer window rect (incl. title bar / borders), screen coords.
    pub outer: Rect,
    /// Client (viewport) rect, screen coords.
    pub client: Rect,
}

impl Geometry {
    pub fn client_to_screen(&self, p: Point) -> Point {
        Point::new(self.client.x + p.x, self.client.y + p.y)
    }
    pub fn screen_to_client(&self, p: Point) -> Point {
        Point::new(p.x - self.client.x, p.y - self.client.y)
    }
    pub fn in_client(&self, p: Point) -> bool {
        p.x >= 0 && p.y >= 0 && p.x < self.client.w && p.y < self.client.h
    }
    /// Left/top border sizes (client origin relative to outer origin).
    pub fn border(&self) -> (i32, i32) {
        (self.client.x - self.outer.x, self.client.y - self.outer.y)
    }
    /// Outer size that yields `client_w x client_h` with the current borders.
    pub fn outer_for_client(&self, client_w: i32, client_h: i32) -> (i32, i32) {
        (
            client_w + (self.outer.w - self.client.w),
            client_h + (self.outer.h - self.client.h),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo() -> Geometry {
        Geometry {
            hwnd: Hwnd(1),
            outer: Rect::new(100, 50, 816, 638),
            client: Rect::new(108, 81, 800, 599),
        }
    }

    #[test]
    fn client_screen_roundtrip() {
        let g = geo();
        let p = Point::new(400, 300);
        let s = g.client_to_screen(p);
        assert_eq!(s, Point::new(508, 381));
        assert_eq!(g.screen_to_client(s), p);
        assert!(g.in_client(p));
        assert!(!g.in_client(Point::new(800, 10)));
    }

    #[test]
    fn borders_and_outer_size() {
        let g = geo();
        assert_eq!(g.border(), (8, 31));
        assert_eq!(g.outer_for_client(800, 599), (816, 638));
        assert_eq!(g.outer_for_client(640, 480), (656, 519));
    }

    #[test]
    fn rect_intersect() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(
            a.intersect(&Rect::new(5, 5, 10, 10)),
            Some(Rect::new(5, 5, 5, 5))
        );
        assert_eq!(a.intersect(&Rect::new(10, 0, 5, 5)), None);
    }
}
