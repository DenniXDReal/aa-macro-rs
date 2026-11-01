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
pub fn place_open_batch(hwnd: Hwnd, keys: &Keys, slot: usize, at: Point) -> Batch {
    let key = keys.units[(slot.clamp(1, 6)) - 1];
    Batch::new(hwnd, format!("place slot {slot}"))
        .then(Action::KeyTap { key, hold_ms: 40 })
        .wait(60)
        .move_to(at.x, at.y)
        .wait(90)
        .click(at.x, at.y)
        .wait(60)
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
    confirm: &dyn Fn() -> Option<bool>,
) -> Result<Vec<Point>, InputError> {
    let mut placed = vec![];
    for at in spots {
        if placed.len() as u32 >= count {
            break;
        }
        arb.run(place_open_batch(hwnd, keys, slot, *at))?;
        // the badge pops up a moment after the click
        let mut ok = confirm();
        for _ in 0..4 {
            if ok != Some(false) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(120));
            ok = confirm();
        }
        arb.run(cancel_batch(hwnd, keys))?;
        if ok.unwrap_or(true) {
            placed.push(*at);
        }
    }
    Ok(placed)
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
            },
        );
        let keys = Keys::from(&Keybinds::default()).unwrap();
        let spots = spiral_spots(6, 1);
        let hs: Vec<_> = [100isize, 101]
            .into_iter()
            .map(|h| {
                let (a, k, s) = (arb.handle(), keys.clone(), spots.clone());
                std::thread::spawn(move || place_slot(&a, Hwnd(h), &k, 2, 3, &s, &|| None).unwrap())
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
