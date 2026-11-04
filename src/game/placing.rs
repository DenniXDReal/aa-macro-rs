//! Unit placement, upgrades and the top-down camera.
//!
//! Ported from the old AHK framework / Spiral v10 behaviour:
//! * camera: mouse to centre, zoom fully in (first person), look down, zoom
//!   back out -> top-down view;
//! * spiral spots: an expanding square spiral from the play-area centre with
//!   ±15 px jitter, restarting from the centre when it leaves the area;
//! * one placement try = unit key -> click the spot -> cancel key (Q), then
//!   (if you captured `unit_panel`) wait for the unit panel to prove it worked.
//!
//! Every try is one short arbiter batch, so 2-3 accounts can place at the
//! same time: their tries simply take turns on the mouse.

use crate::config::Keybinds;
use crate::geom::{Hwnd, Point, Rect};
use crate::input::{Action, ArbiterHandle, Batch, InputError, Priority};
use crate::keys::Key;
use crate::platform::MouseButton;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraSettings {
    /// Wheel notches up to reach first person.
    pub zoom_in: u32,
    /// Relative mouse motion down while in first person (look at the floor).
    pub look_down_px: i32,
    /// Wheel notches down afterwards (camera height).
    pub zoom_out: u32,
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            zoom_in: 10,
            look_down_px: 300,
            zoom_out: 20,
        }
    }
}

/// The batch that turns the camera top-down (client coords, 800x599).
pub fn camera_batch(hwnd: Hwnd, c: &CameraSettings) -> Batch {
    let centre = Point::new(400, 300);
    let mut b = Batch::new(hwnd, "camera: top-down").priority(Priority::High);
    b.long = true;
    b.actions = vec![
        Action::MoveTo(centre),
        Action::Wait(100),
        Action::Scroll {
            at: centre,
            notches: c.zoom_in as i32,
        },
        Action::Wait(200),
        Action::Click {
            at: centre,
            button: MouseButton::Left,
        },
        Action::MoveRel {
            dx: 0,
            dy: c.look_down_px,
            steps: 15,
        },
        Action::Wait(150),
        Action::Scroll {
            at: centre,
            notches: -(c.zoom_out as i32),
        },
        Action::Wait(150),
        Action::MoveTo(centre),
    ];
    b
}

/// Play area used by the spiral (client coords) - from the AHK bounds.
pub const PLAY_AREA: Rect = Rect::new(190, 150, 445, 370);
/// Spiral centre: client centre nudged up-left like the AHK (-30, -30).
pub const SPIRAL_CENTRE: Point = Point::new(370, 270);

/// Placement shapes from the AHK macro (+ the v10 spiral).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Shape {
    /// v10: jittered spiral out from the middle.
    #[default]
    Spiral,
    /// 4 rings (45 / 90 / 135 / 180 px) around the middle.
    Circle,
    /// Even grid over the middle of the screen, row by row.
    Grid,
    /// Tight 3x3 around the middle (30 px apart).
    Grid3x3,
    /// Square spiral on a 40 px grid (7x7).
    SpiralGrid,
    /// 7x7 grid, column by column (top to bottom, left to right).
    UpDown,
    /// Random spots, at least 40 px apart, middle last.
    Random,
}

impl Shape {
    pub const ALL: [Shape; 7] = [
        Shape::Spiral,
        Shape::Circle,
        Shape::Grid,
        Shape::Grid3x3,
        Shape::SpiralGrid,
        Shape::UpDown,
        Shape::Random,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Shape::Spiral => "Spiral",
            Shape::Circle => "Circle",
            Shape::Grid => "Grid",
            Shape::Grid3x3 => "3x3 Grid",
            Shape::SpiralGrid => "Spiral grid",
            Shape::UpDown => "Up and Down",
            Shape::Random => "Random",
        }
    }
}

/// Hill units: while one is selected the high ground it can go on glows
/// bright green (as AA Spiral v16 uses). Spots on that green, middle first.
pub fn hill_spots(f: &crate::capture::Frame, max: usize) -> Vec<Point> {
    let green = |x: i32, y: i32| {
        f.rgb(x, y).is_some_and(|(r, g, b)| {
            let (r, g, b) = (r as i32, g as i32, b as i32);
            // the glowing high ground: bright green (r ~0-80, g ~230-255,
            // b ~30-110); a bit of room for maps / lighting that differ
            g >= 140 && g - r >= 50 && g - b >= 50
        })
    };
    let a = SHAPE_AREA;
    let mut out = vec![];
    let mut y = a.y + 8;
    while y < a.y + a.h - 8 {
        let mut x = a.x + 8;
        while x < a.x + a.w - 8 {
            // the spot and its neighbours green = solidly on the high ground
            let n = [(0, 0), (-6, 0), (6, 0), (0, -6), (0, 6)]
                .iter()
                .filter(|(dx, dy)| green(x + dx, y + dy))
                .count();
            if n >= 4 {
                out.push(Point::new(x, y));
            }
            x += 22;
        }
        y += 22;
    }
    let c = SHAPE_CENTRE;
    out.sort_by_key(|p| (p.x - c.x).pow(2) + (p.y - c.y).pow(2));
    out.truncate(max);
    out
}

/// AHK's centre of the play field (client px).
pub const SHAPE_CENTRE: Point = Point::new(408, 320);
/// Spots must stay off the hotbar / top UI.
pub const SHAPE_AREA: Rect = Rect::new(12, 60, 776, 470);

/// Where each placing account's shape is centred when several place at
/// once (offset from the middle), so they don't fight over the same spots:
/// 1st middle, 2nd left, 3rd right, 4th lower.
pub fn account_offset(nth: usize, placers: usize) -> Point {
    if placers <= 1 {
        return Point::new(0, 0);
    }
    match nth % 4 {
        0 => Point::new(0, -40),
        1 => Point::new(-210, 30),
        2 => Point::new(210, 30),
        _ => Point::new(0, 150),
    }
}

/// Shape spots for the `nth` (0-based) of `placers` placing accounts.
pub fn shape_spots_for(
    shape: Shape,
    n: usize,
    seed: u64,
    scale: f32,
    nth: usize,
    placers: usize,
) -> Vec<Point> {
    // every account uses the whole shape; they don't share spots because
    // each spot is claimed by one account at a time (see place_all)
    let _ = (nth, placers);
    let o = Point::new(0, 0);
    let mut out: Vec<Point> = shape_spots_scaled(shape, n * 2, seed, scale)
        .into_iter()
        .map(|p| Point::new(p.x + o.x, p.y + o.y))
        .filter(|q| SHAPE_AREA.contains(*q))
        .collect();
    // always from the middle outwards to the furthest point
    let c = if shape == Shape::Spiral {
        SPIRAL_CENTRE
    } else {
        SHAPE_CENTRE
    };
    let c = Point::new(c.x + o.x, c.y + o.y);
    out.sort_by_key(|p| (p.x - c.x).pow(2) + (p.y - c.y).pow(2));
    out.truncate(n);
    out
}

/// Candidate spots for `shape` (deterministic for a seed), spread out by
/// `scale` around the middle (1.0 = the AHK / v10 size).
pub fn shape_spots_scaled(shape: Shape, n: usize, seed: u64, scale: f32) -> Vec<Point> {
    let c = if shape == Shape::Spiral {
        SPIRAL_CENTRE
    } else {
        SHAPE_CENTRE
    };
    let scale = scale.clamp(0.5, 3.0);
    let mut out: Vec<Point> = vec![];
    for p in shape_spots(shape, n * 2, seed) {
        let q = Point::new(
            c.x + ((p.x - c.x) as f32 * scale).round() as i32,
            c.y + ((p.y - c.y) as f32 * scale).round() as i32,
        );
        if SHAPE_AREA.contains(q)
            && !out
                .iter()
                .any(|o| (o.x - q.x).abs() < 8 && (o.y - q.y).abs() < 8)
        {
            out.push(q);
        }
        if out.len() >= n {
            break;
        }
    }
    out
}

/// Candidate spots for `shape` (deterministic for a seed).
pub fn shape_spots(shape: Shape, n: usize, seed: u64) -> Vec<Point> {
    let c = SHAPE_CENTRE;
    let mut v: Vec<Point> = match shape {
        Shape::Spiral => return spiral_spots(n, seed),
        Shape::Circle => {
            // middle, then 12 rings out to the edge, a spot about every 32 px
            let mut v = vec![c];
            for ring in 1..=12 {
                let r = 28.0 * ring as f64;
                let k = ((std::f64::consts::TAU * r / 32.0).round() as usize).max(6);
                // every other ring turned half a step: no straight lines of spots
                let turn = if ring % 2 == 0 { 0.5 } else { 0.0 };
                for i in 0..k {
                    let i = i as f64 + turn;
                    let a = i * std::f64::consts::TAU / k as f64;
                    v.push(Point::new(
                        c.x + (r * a.cos()).round() as i32,
                        c.y + (r * a.sin()).round() as i32,
                    ));
                }
            }
            v
        }
        Shape::Grid => {
            let mut v = vec![];
            let mut y = c.y - 140;
            while y <= c.y + 140 {
                let mut x = c.x - 180;
                while x <= c.x + 180 {
                    v.push(Point::new(x, y));
                    x += 25;
                }
                y += 28;
            }
            v
        }
        Shape::Grid3x3 => square_spiral(c, 30, 9),
        Shape::SpiralGrid => square_spiral(Point::new(c.x - 30, c.y - 30), 40, 49),
        Shape::UpDown => {
            let mut v = vec![];
            for col in 0..7 {
                for row in 0..7 {
                    v.push(Point::new(c.x - 120 + col * 40, c.y - 120 + row * 40));
                }
            }
            v
        }
        Shape::Random => {
            let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
            let mut next = |lo: i32, hi: i32| {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                lo + (rng % (hi - lo + 1) as u64) as i32
            };
            let mut v: Vec<Point> = vec![];
            for _ in 0..200 {
                let p = Point::new(next(c.x - 180, c.x + 180), next(c.y - 140, c.y + 140));
                if v.iter().all(|q| {
                    let (dx, dy) = ((q.x - p.x) as f64, (q.y - p.y) as f64);
                    (dx * dx + dy * dy).sqrt() >= 40.0
                }) {
                    v.push(p);
                }
                if v.len() >= 40 {
                    break;
                }
            }
            v.push(c);
            v
        }
    };
    v.retain(|p| SHAPE_AREA.contains(*p));
    v.truncate(n);
    v
}

/// Square spiral out from `start`: right, down, left, up, growing.
fn square_spiral(start: Point, step: i32, count: usize) -> Vec<Point> {
    let dirs = [(1, 0), (0, 1), (-1, 0), (0, -1)];
    let (mut x, mut y) = (start.x, start.y);
    let mut out = vec![Point::new(x, y)];
    let (mut len, mut d) = (1, 0usize);
    while out.len() < count {
        for _ in 0..2 {
            for _ in 0..len {
                x += dirs[d].0 * step;
                y += dirs[d].1 * step;
                out.push(Point::new(x, y));
                if out.len() >= count {
                    return out;
                }
            }
            d = (d + 1) % 4;
        }
        len += 1;
    }
    out
}

/// `n` spiral candidate spots (deterministic for a given `seed`).
pub fn spiral_spots(n: usize, seed: u64) -> Vec<Point> {
    let dirs = [(1, 0), (0, 1), (-1, 0), (0, -1)];
    let mut rng = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let mut jitter = || {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((rng >> 33) % 31) as i32 - 15
    };
    let mut out = Vec::with_capacity(n);
    let (mut x, mut y) = (SPIRAL_CENTRE.x, SPIRAL_CENTRE.y);
    let (mut steps, mut dir, mut turns) = (30, 0usize, 0);
    let mut guard = 0;
    while out.len() < n && guard < n * 50 {
        guard += 1;
        let p = Point::new(x, y);
        if PLAY_AREA.contains(p)
            && !out
                .iter()
                .any(|q: &Point| (q.x - p.x).abs() < 12 && (q.y - p.y).abs() < 12)
        {
            out.push(p);
        }
        x += dirs[dir].0 * steps + jitter();
        y += dirs[dir].1 * steps + jitter();
        if !PLAY_AREA.contains(Point::new(x, y)) {
            x = SPIRAL_CENTRE.x + jitter();
            y = SPIRAL_CENTRE.y + jitter();
            steps = 30;
        }
        turns += 1;
        if turns == 2 {
            steps += 30;
            turns = 0;
        }
        dir = (dir + 1) % 4;
    }
    out
}

/// Spots key for a client: the main uses the map name, other placing
/// clients get their own set ("Space Center @2").
pub fn spots_key(map: &str, client_no: usize) -> String {
    if client_no <= 1 {
        map.to_string()
    } else {
        format!("{map} @{client_no}")
    }
}

/// Keys for slots 1..6 + upgrade + cancel, parsed once.
#[derive(Clone, Debug)]
pub struct Keys {
    pub units: Vec<Key>,
    pub upgrade: Key,
    pub cancel: Key,
}

impl Keys {
    pub fn from(k: &Keybinds) -> Result<Keys, String> {
        let p = |s: &str| {
            Key::parse(s).ok_or_else(|| format!("unknown key '{s}' (Settings > Keybinds)"))
        };
        Ok(Keys {
            units: k.units.iter().map(|s| p(s)).collect::<Result<_, _>>()?,
            upgrade: p(&k.upgrade)?,
            cancel: p(&k.cancel)?,
        })
    }
}

/// One placement try for `slot` (1-6) at `at`.
pub fn place_batch(hwnd: Hwnd, keys: &Keys, slot: usize, at: Point) -> Batch {
    let mut b = place_open_batch(hwnd, keys, slot, at);
    b.actions.push(Action::KeyTap {
        key: keys.cancel,
        hold_ms: 40,
    });
    b
}

/// Pick the unit and click the spot, leaving whatever opened (the unit's
/// "UPG [0]" panel on success) on screen so it can be checked.
/// Pause between each step of a placement (select > click > check > next
/// account), ms. Set from Settings when a run starts.
pub static PLACE_SETTLE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(2000);
/// How long the check (unit_placed picture / "UPG [0]" OCR) may look, ms.
pub static PLACE_CHECK_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(3000);

fn gap() -> u64 {
    PLACE_SETTLE_MS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Select the unit > wait > click the spot > wait (the check comes next).
pub fn place_open_batch(hwnd: Hwnd, keys: &Keys, slot: usize, at: Point) -> Batch {
    let key = keys.units[(slot.clamp(1, 6)) - 1];
    Batch::new(hwnd, format!("place slot {slot}"))
        .then(Action::KeyTap { key, hold_ms: 50 })
        .wait(gap())
        .move_to(at.x, at.y)
        .wait(250)
        .click(at.x, at.y)
        .wait(gap())
}

pub fn cancel_batch(hwnd: Hwnd, keys: &Keys) -> Batch {
    Batch::new(hwnd, "close panel").then(Action::KeyTap {
        key: keys.cancel,
        hold_ms: 40,
    })
}

/// Open the unit at `at`, press upgrade `times` times, close the panel.
pub fn upgrade_batch(hwnd: Hwnd, keys: &Keys, at: Point, times: u32) -> Batch {
    let mut b = Batch::new(hwnd, "upgrade").click(at.x, at.y).wait(250);
    for _ in 0..times.max(1) {
        b = b
            .then(Action::KeyTap {
                key: keys.upgrade,
                hold_ms: 40,
            })
            .wait(120);
    }
    b.then(Action::KeyTap {
        key: keys.cancel,
        hold_ms: 40,
    })
}

/// Result of trying to place one unit.
#[derive(Debug, PartialEq, Eq)]
pub enum Placed {
    /// Confirmed (unit panel seen) or assumed (no `unit_panel` pattern).
    Yes,
    No,
}

/// Try `spots` in order until `count` units of `slot` are down.
/// `confirm` returns Some(true/false) when it can tell (the "UPG [0]" badge
/// of a freshly placed unit), None when it can't - then every try counts.
pub fn place_slot(
    arb: &ArbiterHandle,
    hwnd: Hwnd,
    keys: &Keys,
    slot: usize,
    count: u32,
    spots: &[Point],
    badge: Option<crate::input::Probe>,
    keep_open: bool,
    delay_ms: u64,
    before_each: &dyn Fn() -> bool,
    on_placed: &dyn Fn(Point),
    on_failed: &dyn Fn(Point),
) -> Result<Vec<Point>, InputError> {
    place_slot_checked(
        arb,
        hwnd,
        keys,
        slot,
        count,
        spots,
        badge,
        keep_open,
        delay_ms,
        before_each,
        &|_| true,
        &|seen, _| if seen { Check::Placed } else { Check::Failed },
        on_placed,
        on_failed,
    )
}

/// What a placement attempt turned out to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Placed,
    Failed,
    /// Placed, and that was the last one this unit may have.
    PlacedLast,
    /// Stop this unit (unique unit already down / at its limit).
    Stop,
}

/// `place_slot` with a say in whether each attempt worked: `check(seen, spot)`
/// gets whether the unit panel showed (always true without a badge).
#[allow(clippy::too_many_arguments)]
pub fn place_slot_checked(
    arb: &ArbiterHandle,
    hwnd: Hwnd,
    keys: &Keys,
    slot: usize,
    count: u32,
    spots: &[Point],
    badge: Option<crate::input::Probe>,
    keep_open: bool,
    delay_ms: u64,
    before_each: &dyn Fn() -> bool,
    claim: &dyn Fn(Point) -> bool,
    check: &dyn Fn(bool, Point) -> Check,
    on_placed: &dyn Fn(Point),
    on_failed: &dyn Fn(Point),
) -> Result<Vec<Point>, InputError> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let mut placed: Vec<Point> = vec![];
    // the last attempt that looked failed: if its panel is up by the next
    // attempt, it did go down - just slowly (AA Spiral v17 "credit late")
    let mut last_fail: Option<Point> = None;
    // reached the furthest spot and still short? start again from the
    // middle (a spot may work now - e.g. there was no money before)
    let rounds = 2;
    let order: Vec<Point> = (0..rounds).flat_map(|_| spots.iter().copied()).collect();
    for (i, at) in order.iter().enumerate() {
        if placed.len() as u32 >= count {
            break;
        }
        if placed.contains(at) {
            continue;
        }
        // another account placing here (or already did)? take the next spot
        if !claim(*at) {
            continue;
        }
        if i > 0 && delay_ms > 0 {
            // let the game catch up so the next look / click is right
            // in short bits, checking for VICTORY / DEFEAT in between
            let mut left = delay_ms;
            let mut over = false;
            while left > 700 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                left -= 500;
                if !before_each() {
                    over = true;
                    break;
                }
            }
            if over {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(left));
        }
        // right before placing: card pop-ups, and stop if the stage ended
        if !before_each() {
            break;
        }
        if let (Some(prev), Some(p)) = (last_fail, &badge) {
            if (p.find)().is_some() {
                on_placed(prev);
                placed.push(prev);
                arb.run(cancel_batch(hwnd, keys))?;
                if placed.len() as u32 >= count {
                    break;
                }
            }
        }
        last_fail = None;
        // one input turn for key + click + badge check + close, so other
        // accounts can't cut in between (no extra window swaps)
        let seen = Arc::new(AtomicBool::new(false));
        let mut b = place_open_batch(hwnd, keys, slot, *at);
        if let Some(p) = &badge {
            let (f, s2) = (p.find.clone(), seen.clone());
            let rec = crate::input::Probe::new(p.label.clone(), move || {
                let r = f();
                if r.is_some() {
                    s2.store(true, Ordering::Relaxed);
                }
                r
            });
            b = b.then(Action::WaitFor {
                probe: rec,
                timeout_ms: PLACE_CHECK_MS.load(Ordering::Relaxed),
                required: false,
            });
        }
        if !keep_open {
            b = b
                .then(Action::KeyTap {
                    key: keys.cancel,
                    hold_ms: 40,
                })
                // let the close register before another account takes over
                .wait(gap());
        }
        arb.run(b)?;
        let res = check(badge.is_none() || seen.load(Ordering::Relaxed), *at);
        match res {
            Check::Placed | Check::PlacedLast => {
                // with keep_open its panel is still up (e.g. turn Auto ability on)
                on_placed(*at);
                placed.push(*at);
            }
            Check::Failed => {
                on_failed(*at);
                last_fail = Some(*at);
            }
            Check::Stop => {}
        }
        if keep_open {
            arb.run(cancel_batch(hwnd, keys))?;
        }
        if matches!(res, Check::PlacedLast | Check::Stop) {
            break;
        }
    }
    Ok(placed)
}

#[cfg(test)]
mod shape_tests {
    use super::*;

    #[test]
    fn hill_spots_on_green() {
        let mut f = crate::capture::Frame::solid(800, 599, 40, 60, 50);
        for y in 200..300 {
            for x in 500..640 {
                let i = ((y * 800 + x) * 4) as usize;
                f.bgra[i..i + 3].copy_from_slice(&[40, 230, 60]);
            }
        }
        let v = hill_spots(&f, 50);
        assert!(!v.is_empty());
        assert!(v
            .iter()
            .all(|p| (500..640).contains(&p.x) && (200..300).contains(&p.y)));
    }

    #[test]
    fn every_shape_gives_spots_on_screen() {
        for sh in Shape::ALL {
            let v = shape_spots(sh, 60, 3);
            assert!(v.len() >= 9, "{sh:?}: {}", v.len());
            assert!(v.iter().all(|p| SHAPE_AREA.contains(*p)), "{sh:?}");
        }
        assert_eq!(shape_spots(Shape::Grid3x3, 60, 0)[0], SHAPE_CENTRE);
        assert_eq!(shape_spots(Shape::Grid3x3, 60, 0).len(), 9);
        assert_eq!(
            shape_spots(Shape::Random, 60, 5),
            shape_spots(Shape::Random, 60, 5)
        );
        assert_eq!(
            *shape_spots(Shape::Random, 60, 5).last().unwrap(),
            SHAPE_CENTRE
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::Control;
    use crate::input::{Arbiter, ArbiterOptions};
    use crate::platform::mock::{Call, MockPlatform};
    use std::sync::Arc;

    #[test]
    fn spiral_stays_in_area_and_is_spread() {
        let s = spiral_spots(40, 7);
        assert_eq!(s.len(), 40);
        assert_eq!(s[0], SPIRAL_CENTRE);
        for p in &s {
            assert!(PLAY_AREA.contains(*p), "{p:?}");
        }
        assert_eq!(spiral_spots(40, 7), s, "deterministic for a seed");
        assert_ne!(spiral_spots(40, 8), s);
    }

    #[test]
    fn keys_and_spot_sets() {
        let k = Keys::from(&Keybinds::default()).unwrap();
        assert_eq!(k.units[0], Key::parse("1").unwrap());
        assert_eq!(k.upgrade, Key::parse("r").unwrap());
        assert_eq!(spots_key("Space Center", 1), "Space Center");
        assert_eq!(spots_key("Space Center", 2), "Space Center @2");
        let bad = Keybinds {
            upgrade: "??".into(),
            ..Default::default()
        };
        assert!(Keys::from(&bad).is_err());
    }

    #[test]
    fn two_accounts_place_at_the_same_time() {
        let mock = Arc::new(MockPlatform::with_windows(2));
        let arb = Arbiter::start(
            mock.clone(),
            Control::new(),
            ArbiterOptions {
                mouse_settle_ms: 1,
                click_hold_ms: 1,
                max_lease_ms: 400,
                click_mode: crate::input::ClickMode::Single,
                swap_delay_ms: 0,
            },
        );
        let keys = Keys::from(&Keybinds::default()).unwrap();
        let spots = spiral_spots(6, 1);
        let hs: Vec<_> = [100isize, 101]
            .into_iter()
            .map(|h| {
                let (a, k, s) = (arb.handle(), keys.clone(), spots.clone());
                std::thread::spawn(move || {
                    place_slot(
                        &a,
                        Hwnd(h),
                        &k,
                        2,
                        3,
                        &s,
                        None,
                        false,
                        0,
                        &|| true,
                        &|_| {},
                        &|_| {},
                    )
                    .unwrap()
                })
            })
            .collect();
        for h in hs {
            assert_eq!(h.join().unwrap().len(), 3);
        }
        let calls = mock.take_calls();
        let k2 = Key::parse("2").unwrap();
        // 6 tries total, each: unit key down/up ... cancel key, never interleaved mid-try.
        assert_eq!(
            calls.iter().filter(|c| **c == Call::Key(k2, true)).count(),
            6
        );
        let focus = calls.iter().filter(|c| matches!(c, Call::Focus(_))).count();
        assert!(focus >= 2, "both windows got turns");
    }

    #[test]
    fn camera_is_zoom_in_look_down_zoom_out() {
        let b = camera_batch(Hwnd(1), &CameraSettings::default());
        assert!(matches!(b.actions[2], Action::Scroll { notches: 10, .. }));
        assert!(matches!(b.actions[5], Action::MoveRel { dy: 300, .. }));
        assert!(matches!(b.actions[7], Action::Scroll { notches: -20, .. }));
    }
}
