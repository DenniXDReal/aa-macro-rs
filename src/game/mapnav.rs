//! Minimap navigation: map data files, reading the minimap, and walking
//! with it.
//!
//! The minimap (bottom right, 146x146, see `minimap::MINIMAP`) is north-up
//! and shows the whole stage: the enemies' bright green path, the base
//! (white heart), the enemy spawn (green swirl at the far end of the path)
//! and every player as a small white chevron with a blue mark behind it.
//! Buildings aren't on it, so walking is: hold the keys that point most
//! towards the next waypoint, look again, correct; when the arrow stops
//! moving, back up and strafe around whatever is in the way.
//!
//! Map data (`<Map>.json`): built-in in assets/mapdata/, yours in
//! maps/data/ next to the exe (yours win). Coordinates are minimap pixels
//! (0..146). `walk` = where to walk, in order; empty = no minimap walk on
//! that map. A file can be made from a saved minimap picture
//! ([`from_minimap`]): base, spawn and path are found, `walk` is yours to
//! set (Routes page).

use crate::capture::Frame;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

/// Minimap size (px).
pub const SIZE: i32 = 146;
/// The frame around the minimap: ignored.
const BORDER: i32 = 4;
/// Within this many minimap px of a waypoint = there.
pub const ARRIVE_PX: f32 = 3.0;
/// One walking pulse (hold the keys this long, then look again).
pub const PULSE_MS: u64 = 200;
/// The arrow moved less than this over [`STUCK_MS`] while walking = stuck.
pub const STUCK_PX: f32 = 1.0;
pub const STUCK_MS: u64 = 600;
/// Back up / strafe when stuck (ms each).
pub const BACKUP_MS: u64 = 300;
pub const STRAFE_MS: u64 = 400;
/// Give up after this many stuck episodes on one waypoint.
pub const MAX_STUCK: u32 = 8;

// ---------------- map data ----------------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MapData {
    pub map: String,
    /// File format version (1).
    pub version: u32,
    /// The base (heart), minimap px.
    pub base: Option<[i32; 2]>,
    /// Where enemies come from (the far end of the path).
    pub spawn: Option<[i32; 2]>,
    /// The enemies' path, spawn first, every few px.
    pub path: Vec<[i32; 2]>,
    /// More lanes on 2- / 3-lane maps (purple, ...), each spawn first,
    /// ending at the base like `path`.
    pub lanes: Vec<Vec<[i32; 2]>>,
    /// Where to walk, in order. Empty = along the enemy path from the base
    /// towards the spawn, `walk_pct` of the way (the front line).
    pub walk: Vec<[i32; 2]>,
    /// This map's share of the path to walk, % (None = Settings' value).
    pub walk_pct: Option<u8>,
    /// Camera settings the file was made with (free-form, for reference).
    pub camera: Option<serde_json::Value>,
    pub notes: String,
}

/// "Navy Bay" / "navy_bay" / "Navy Bay (Summer)" -> "navybay".
pub fn key(name: &str) -> String {
    let base = name.split('(').next().unwrap_or(name);
    base.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

pub fn user_dir() -> PathBuf {
    super::data_dir().join("maps").join("data")
}

/// Every map file: built-in, then yours on top (same map = yours).
pub fn load_all() -> BTreeMap<String, MapData> {
    let mut out = BTreeMap::new();
    for (stem, text) in crate::movement::gen::MAPDATA {
        if let Ok(mut d) = serde_json::from_str::<MapData>(text) {
            if d.map.is_empty() {
                d.map = stem.to_string();
            }
            out.insert(key(&d.map), d);
        }
    }
    if let Ok(rd) = std::fs::read_dir(user_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(t) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Ok(mut d) = serde_json::from_str::<MapData>(&t) else {
                continue;
            };
            if d.map.is_empty() {
                d.map = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
            }
            out.insert(key(&d.map), d);
        }
    }
    out
}

pub fn find(map: &str) -> Option<MapData> {
    load_all().remove(&key(map))
}

pub fn save(d: &MapData) -> std::io::Result<PathBuf> {
    let dir = user_dir();
    std::fs::create_dir_all(&dir)?;
    let p = dir.join(format!("{}.json", d.map));
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(d).unwrap_or_default())?;
    std::fs::rename(&tmp, &p)?;
    Ok(p)
}

/// A map file from a minimap picture: base, spawn and path filled in.
pub fn from_minimap(map: &str, m: &Frame) -> MapData {
    let r = read(m);
    MapData {
        map: map.to_string(),
        version: 1,
        base: r.base.map(|p| [p.0 as i32, p.1 as i32]),
        spawn: r.spawn.map(|p| [p.0, p.1]),
        path: r.path.iter().map(|p| [p.0, p.1]).collect(),
        lanes: r
            .lanes
            .iter()
            .map(|l| l.iter().map(|p| [p.0, p.1]).collect())
            .collect(),
        walk: vec![],
        walk_pct: None,
        camera: None,
        notes: String::new(),
    }
}

/// Where to walk on this map: your walk points if it has any, else the
/// enemy path from the base end towards the spawn, stopping `pct` % of the
/// way along it (this map's own % if set). Empty = nothing to walk.
pub fn walk_points(d: &MapData, default_pct: u8) -> Vec<(f32, f32)> {
    if !d.walk.is_empty() {
        return d.walk.iter().map(|p| (p[0] as f32, p[1] as f32)).collect();
    }
    if d.path.len() < 2 {
        return vec![];
    }
    let pct = d.walk_pct.unwrap_or(default_pct).clamp(1, 100) as f32 / 100.0;
    // stored spawn -> base: walk it base -> spawn
    let pts: Vec<(f32, f32)> = d
        .path
        .iter()
        .rev()
        .map(|p| (p[0] as f32, p[1] as f32))
        .collect();
    let total: f32 = pts
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum();
    let stop = total * pct;
    let mut out = vec![pts[0]];
    let mut done = 0.0;
    for w in pts.windows(2) {
        let seg = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        if done + seg >= stop {
            // the exact stop point on this stretch
            let t = if seg > 0.0 { (stop - done) / seg } else { 0.0 };
            out.push((
                w[0].0 + (w[1].0 - w[0].0) * t,
                w[0].1 + (w[1].1 - w[0].1) * t,
            ));
            return out;
        }
        done += seg;
        out.push(w[1]);
    }
    out
}

/// How far (minimap px) the stop may be from every lane on a 2- / 3-lane
/// map: about what units placed there still reach.
pub const COVER_PX: f32 = 18.0;

/// Shortest distance from `p` to the line through `poly`.
fn poly_dist(p: (f32, f32), poly: &[[i32; 2]]) -> f32 {
    let seg = |a: (f32, f32), b: (f32, f32)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l2 = dx * dx + dy * dy;
        let t = if l2 > 0.0 {
            (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
    };
    let f = |q: &[i32; 2]| (q[0] as f32, q[1] as f32);
    match poly {
        [] => f32::MAX,
        [one] => seg(f(one), f(one)),
        _ => poly
            .windows(2)
            .map(|w| seg(f(&w[0]), f(&w[1])))
            .fold(f32::MAX, f32::min),
    }
}

/// `pts` with a point every `step` px along it.
fn densify(pts: &[(f32, f32)], step: f32) -> Vec<(f32, f32)> {
    let mut out = pts.first().copied().into_iter().collect::<Vec<_>>();
    for w in pts.windows(2) {
        let len = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        let n = (len / step).ceil().max(1.0) as usize;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            out.push((
                w[0].0 + (w[1].0 - w[0].0) * t,
                w[0].1 + (w[1].1 - w[0].1) * t,
            ));
        }
    }
    out
}

/// `walk_points`, and on a map with more than one lane: out along the main
/// (green) lane only as far as a spot still within `COVER_PX` of every
/// lane - units there cover all of them (never past the % either).
pub fn walk_points_covering(d: &MapData, default_pct: u8) -> Vec<(f32, f32)> {
    let route = walk_points(d, default_pct);
    if !d.walk.is_empty() || d.lanes.is_empty() || route.len() < 2 {
        return route;
    }
    let fine = densify(&route, 2.0);
    let worst = |p: (f32, f32)| {
        d.lanes
            .iter()
            .map(|l| poly_dist(p, l))
            .fold(poly_dist(p, &d.path), f32::max)
    };
    let scores: Vec<f32> = fine.iter().map(|p| worst(*p)).collect();
    let stop = match scores.iter().rposition(|s| *s <= COVER_PX) {
        Some(i) => i,
        // nowhere covers every lane: the closest spot to all of them
        None => scores
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap_or(0),
    };
    // the route's own corners up to there, then the stop itself
    let end = fine[stop];
    let mut out: Vec<(f32, f32)> = vec![];
    let mut gone = 0.0;
    let reach: f32 = fine
        .windows(2)
        .take(stop)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum();
    for (i, p) in route.iter().enumerate() {
        if i > 0 {
            let q = route[i - 1];
            gone += (p.0 - q.0).hypot(p.1 - q.1);
        }
        if gone >= reach - 0.01 {
            break;
        }
        out.push(*p);
    }
    if out.last() != Some(&end) {
        out.push(end);
    }
    out
}

/// `points` from the one nearest `me` on (no walking back to the start).
pub fn from_nearest(points: &[(f32, f32)], me: (f32, f32)) -> Vec<(f32, f32)> {
    let Some(i) = points
        .iter()
        .enumerate()
        .min_by(|a, b| {
            let da = (a.1 .0 - me.0).hypot(a.1 .1 - me.1);
            let db = (b.1 .0 - me.0).hypot(b.1 .1 - me.1);
            da.total_cmp(&db)
        })
        .map(|(i, _)| i)
    else {
        return vec![];
    };
    points[i..].to_vec()
}

/// Make a map file for every saved minimap picture (maps/minimap/*.png)
/// that has none yet. Returns the maps made.
pub fn build_missing() -> Vec<String> {
    let have = load_all();
    let mut made = vec![];
    let Ok(rd) = std::fs::read_dir(super::minimap::dir()) else {
        return made;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        let name = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.is_empty() || name.starts_with("unknown") || have.contains_key(&key(&name)) {
            continue;
        }
        let Ok(f) = Frame::load_png(&p) else { continue };
        if f.width as i32 != SIZE || f.height as i32 != SIZE {
            continue;
        }
        if save(&from_minimap(&name, &f)).is_ok() {
            made.push(name);
        }
    }
    made
}

// ---------------- reading the minimap ----------------

#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    /// Middle of the arrow (minimap px).
    pub at: (f32, f32),
    /// Which way it points: degrees, 0 = north (up), clockwise.
    pub heading: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MiniRead {
    /// Every player arrow, biggest first.
    pub players: Vec<Player>,
    pub base: Option<(f32, f32)>,
    pub spawn: Option<(i32, i32)>,
    /// Spawn to base, every few px.
    pub path: Vec<(i32, i32)>,
    /// Other lanes (purple, ...), spawn to base like `path`.
    pub lanes: Vec<Vec<(i32, i32)>>,
    /// Each extra lane's spawn.
    pub lane_spawns: Vec<(i32, i32)>,
}

fn white(r: u8, g: u8, b: u8) -> bool {
    r > 200 && g > 200 && b > 200
}
fn blue(r: u8, g: u8, b: u8) -> bool {
    b > 170 && r < 110 && g > 90 && b as i32 - r as i32 > 90
}
fn green(r: u8, g: u8, b: u8) -> bool {
    g > 150 && r < 160 && b < 110 && g as i32 - r as i32 > 50
}
/// The second lane's colour (Assassin Park's purple).
fn purple(r: u8, g: u8, b: u8) -> bool {
    r > 100 && b > 150 && g < 80 && b as i32 - g as i32 > 100
}
/// Lanes other than the green one, by colour.
const EXTRA_LANES: [fn(u8, u8, u8) -> bool; 1] = [purple];

/// Breadth-first over `on` from `start`: (steps, came-from) per pixel.
fn trace(on: &[bool], w: i32, h: i32, start: (i32, i32)) -> (Vec<u32>, Vec<usize>) {
    let mut dist = vec![u32::MAX; (w * h) as usize];
    let mut prev = vec![usize::MAX; (w * h) as usize];
    let mut q = VecDeque::new();
    dist[(start.1 * w + start.0) as usize] = 0;
    q.push_back(start);
    while let Some((x, y)) = q.pop_front() {
        let i = (y * w + x) as usize;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w || ny >= h {
                    continue;
                }
                let j = (ny * w + nx) as usize;
                if on[j] && dist[j] == u32::MAX {
                    dist[j] = dist[i] + 1;
                    prev[j] = i;
                    q.push_back((nx, ny));
                }
            }
        }
    }
    (dist, prev)
}

/// From `far` back to the start of `trace`, a point every 6 steps.
fn back(prev: &[usize], w: i32, far: (i32, i32)) -> Vec<(i32, i32)> {
    let mut out = vec![];
    let mut i = (far.1 * w + far.0) as usize;
    let mut n = 0;
    loop {
        if n % 6 == 0 || prev[i] == usize::MAX {
            out.push(((i as i32) % w, (i as i32) / w));
        }
        if prev[i] == usize::MAX {
            break;
        }
        i = prev[i];
        n += 1;
    }
    out
}

/// The pixel of `pts` nearest `to`.
fn nearest(pts: &[(i32, i32)], to: (f32, f32)) -> Option<(i32, i32)> {
    pts.iter().copied().min_by(|a, b| {
        let da = (a.0 as f32 - to.0).powi(2) + (a.1 as f32 - to.1).powi(2);
        let db = (b.0 as f32 - to.0).powi(2) + (b.1 as f32 - to.1).powi(2);
        da.total_cmp(&db)
    })
}

/// 8-connected groups of the pixels where `mask` is true (inside the frame).
fn groups(m: &Frame, mask: impl Fn(u8, u8, u8) -> bool) -> Vec<Vec<(i32, i32)>> {
    let (w, h) = (m.width as i32, m.height as i32);
    let inside = |x: i32, y: i32| x >= BORDER && y >= BORDER && x < w - BORDER && y < h - BORDER;
    let on = |x: i32, y: i32| inside(x, y) && m.rgb(x, y).is_some_and(|(r, g, b)| mask(r, g, b));
    let mut seen = vec![false; (w * h) as usize];
    let mut out = vec![];
    for y in 0..h {
        for x in 0..w {
            if seen[(y * w + x) as usize] || !on(x, y) {
                continue;
            }
            let mut st = vec![(x, y)];
            seen[(y * w + x) as usize] = true;
            let mut g = vec![];
            while let Some((px, py)) = st.pop() {
                g.push((px, py));
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (qx, qy) = (px + dx, py + dy);
                        if qx < 0 || qy < 0 || qx >= w || qy >= h {
                            continue;
                        }
                        let i = (qy * w + qx) as usize;
                        if !seen[i] && on(qx, qy) {
                            seen[i] = true;
                            st.push((qx, qy));
                        }
                    }
                }
            }
            out.push(g);
        }
    }
    out.sort_by_key(|g| std::cmp::Reverse(g.len()));
    out
}

fn centre(g: &[(i32, i32)]) -> (f32, f32) {
    let n = g.len().max(1) as f32;
    (
        g.iter().map(|p| p.0 as f32).sum::<f32>() / n,
        g.iter().map(|p| p.1 as f32).sum::<f32>() / n,
    )
}

/// Direction of (dx, dy) in degrees, 0 = north, clockwise.
pub fn bearing(dx: f32, dy: f32) -> f32 {
    let a = dx.atan2(-dy).to_degrees();
    if a < 0.0 {
        a + 360.0
    } else {
        a
    }
}

/// Which way a chevron points: from the middle of its two arm tips to its
/// middle (the tips trail behind the point).
fn chevron_heading(g: &[(i32, i32)]) -> Option<f32> {
    let c = centre(g);
    let d = |p: &(i32, i32)| (p.0 as f32 - c.0).powi(2) + (p.1 as f32 - c.1).powi(2);
    let t1 = *g.iter().max_by(|a, b| d(a).total_cmp(&d(b)))?;
    // the other tip: far from the middle and far from the first tip
    let t2 = *g.iter().max_by(|a, b| {
        let s = |p: &(i32, i32)| d(p) + ((p.0 - t1.0).pow(2) + (p.1 - t1.1).pow(2)) as f32;
        s(a).total_cmp(&s(b))
    })?;
    let m = ((t1.0 + t2.0) as f32 / 2.0, (t1.1 + t2.1) as f32 / 2.0);
    let (dx, dy) = (c.0 - m.0, c.1 - m.1);
    (dx.abs() + dy.abs() > 0.3).then(|| bearing(dx, dy))
}

/// Everything on one minimap picture (146x146).
pub fn read(m: &Frame) -> MiniRead {
    let whites = groups(m, white);
    let blues = groups(m, blue);
    let mut out = MiniRead::default();
    // the heart: the big white blob; arrows: small ones
    if let Some(h) = whites.iter().find(|g| g.len() >= 30) {
        out.base = Some(centre(h));
    }
    // arrows are ~6x4; a unit's range ring breaks into 1 px wide arcs
    let arrow_like = |g: &&Vec<(i32, i32)>| {
        let (x0, x1) = (g.iter().map(|p| p.0).min(), g.iter().map(|p| p.0).max());
        let (y0, y1) = (g.iter().map(|p| p.1).min(), g.iter().map(|p| p.1).max());
        let (Some(x0), Some(x1), Some(y0), Some(y1)) = (x0, x1, y0, y1) else {
            return false;
        };
        let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
        (4..30).contains(&g.len()) && (3..=8).contains(&bw) && (3..=8).contains(&bh)
    };
    for g in whites.iter().filter(arrow_like) {
        let at = centre(g);
        // the blue mark behind the arrow, if there is one right by it
        let back = blues
            .iter()
            .map(|b| centre(b))
            .filter(|b| (b.0 - at.0).abs() < 7.0 && (b.1 - at.1).abs() < 7.0)
            .min_by(|a, b| {
                let da = (a.0 - at.0).powi(2) + (a.1 - at.1).powi(2);
                let db = (b.0 - at.0).powi(2) + (b.1 - at.1).powi(2);
                da.total_cmp(&db)
            });
        let heading = match back {
            Some(b) if (at.0 - b.0).abs() + (at.1 - b.1).abs() > 0.8 => {
                Some(bearing(at.0 - b.0, at.1 - b.1))
            }
            _ => chevron_heading(g),
        };
        out.players.push(Player { at, heading });
    }
    // the paths: from the end by the heart out to each lane's far end
    // (lanes share the stretch by the heart, so one walk over all of them)
    let greens: Vec<(i32, i32)> = groups(m, green).into_iter().flatten().collect();
    let extras: Vec<Vec<(i32, i32)>> = EXTRA_LANES
        .iter()
        .map(|f| groups(m, f).into_iter().flatten().collect::<Vec<_>>())
        .filter(|l| l.len() >= 20)
        .collect();
    if greens.len() >= 20 {
        let (w, h) = (m.width as i32, m.height as i32);
        // the arrows sit on the path (it's where players stand): their
        // pixels count as path too, and everything grows 2 px so the walk
        // crosses the gaps they leave
        let arrows: Vec<(i32, i32)> = whites
            .iter()
            .filter(arrow_like)
            .chain(blues.iter())
            .flatten()
            .copied()
            .collect();
        let mut on = vec![false; (w * h) as usize];
        for p in greens
            .iter()
            .chain(extras.iter().flatten())
            .chain(arrows.iter())
        {
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let (x, y) = (p.0 + dx, p.1 + dy);
                    if x >= 0 && y >= 0 && x < w && y < h {
                        on[(y * w + x) as usize] = true;
                    }
                }
            }
        }
        // start: the green nearest the heart (else any end of it)
        let from = out.base.unwrap_or((greens[0].0 as f32, greens[0].1 as f32));
        let start = nearest(&greens, from).unwrap_or(greens[0]);
        let (dist, prev) = trace(&on, w, h, start);
        let at = |p: &(i32, i32)| dist[(p.1 * w + p.0) as usize];
        // a lane's spawn: its pixel farthest along from the heart
        let far = |lane: &[(i32, i32)]| {
            lane.iter()
                .filter(|p| at(p) != u32::MAX)
                .max_by_key(|p| at(p))
                .copied()
        };
        let main = far(&greens).unwrap_or(start);
        out.spawn = Some(main);
        out.path = back(&prev, w, main);
        for lane in &extras {
            let path = match far(lane) {
                Some(s) => (s, back(&prev, w, s)),
                // a lane that never meets the green: its own walk
                None => {
                    let Some(s0) = nearest(lane, from) else {
                        continue;
                    };
                    let (d2, p2) = trace(&on, w, h, s0);
                    let Some(s) = lane
                        .iter()
                        .filter(|p| d2[(p.1 * w + p.0) as usize] != u32::MAX)
                        .max_by_key(|p| d2[(p.1 * w + p.0) as usize])
                        .copied()
                    else {
                        continue;
                    };
                    (s, back(&p2, w, s))
                }
            };
            if path.1.len() >= 3 {
                out.lane_spawns.push(path.0);
                out.lanes.push(path.1);
            }
        }
    }
    out
}

/// The minimap cut out of a client picture, read.
pub fn read_client(f: &Frame) -> Option<MiniRead> {
    super::minimap::crop(f).map(|m| read(&m))
}

// ---------------- walking ----------------

/// Movement keys, as route key names.
pub const W: &str = "w";
pub const A: &str = "a";
pub const S: &str = "s";
pub const D: &str = "d";

/// Which way each key moves the arrow on the minimap (unit vectors), from
/// one measured W move. The camera is fixed, so D is W turned 90°
/// clockwise, S the opposite of W, A the opposite of D.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyDirs {
    pub w: (f32, f32),
}

impl KeyDirs {
    /// From how far the arrow moved while W was held (None = too little to
    /// tell).
    pub fn from_w_move(dx: f32, dy: f32) -> Option<KeyDirs> {
        let len = (dx * dx + dy * dy).sqrt();
        (len >= 1.5).then(|| KeyDirs {
            w: (dx / len, dy / len),
        })
    }
    fn d(&self) -> (f32, f32) {
        (-self.w.1, self.w.0)
    }
    /// The 8 key combinations and where each one goes.
    pub fn combos(&self) -> [(Vec<&'static str>, (f32, f32)); 8] {
        let w = self.w;
        let d = self.d();
        let s = (-w.0, -w.1);
        let a = (-d.0, -d.1);
        let n = |v: (f32, f32)| {
            let l = (v.0 * v.0 + v.1 * v.1).sqrt().max(1e-6);
            (v.0 / l, v.1 / l)
        };
        let add = |p: (f32, f32), q: (f32, f32)| n((p.0 + q.0, p.1 + q.1));
        [
            (vec![W], w),
            (vec![W, D], add(w, d)),
            (vec![D], d),
            (vec![S, D], add(s, d)),
            (vec![S], s),
            (vec![S, A], add(s, a)),
            (vec![A], a),
            (vec![W, A], add(w, a)),
        ]
    }
    /// The keys pointing most towards (dx, dy).
    pub fn towards(&self, dx: f32, dy: f32) -> Vec<&'static str> {
        self.combos()
            .into_iter()
            .max_by(|a, b| {
                let da = a.1 .0 * dx + a.1 .1 * dy;
                let db = b.1 .0 * dx + b.1 .1 * dy;
                da.total_cmp(&db)
            })
            .map(|c| c.0)
            .unwrap_or_default()
    }
}

/// What to do next.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    /// Hold these keys for these ms, one after another, then look again.
    Moves(Vec<(Vec<&'static str>, u64)>),
    /// Reached the last waypoint.
    Arrived,
    GiveUp(String),
}

/// Walking a list of waypoints, one look at a time.
#[derive(Clone, Debug)]
pub struct Nav {
    pub dirs: KeyDirs,
    pub waypoints: Vec<(f32, f32)>,
    pub next: usize,
    /// (time ms, position) of recent looks while walking.
    seen: VecDeque<(u64, (f32, f32))>,
    stuck: u32,
    last_keys: Vec<&'static str>,
}

fn opposite(k: &str) -> &'static str {
    match k {
        W => S,
        S => W,
        A => D,
        _ => A,
    }
}

impl Nav {
    pub fn new(dirs: KeyDirs, waypoints: Vec<(f32, f32)>) -> Self {
        Nav {
            dirs,
            waypoints,
            next: 0,
            seen: VecDeque::new(),
            stuck: 0,
            last_keys: vec![],
        }
    }

    /// One look: the arrow is at `pos` at time `now_ms`.
    pub fn step(&mut self, now_ms: u64, pos: (f32, f32)) -> Cmd {
        // reached waypoints are skipped (more than one at once is fine)
        while let Some(t) = self.waypoints.get(self.next) {
            let d = ((t.0 - pos.0).powi(2) + (t.1 - pos.1).powi(2)).sqrt();
            if d > ARRIVE_PX {
                break;
            }
            self.next += 1;
            self.seen.clear();
            self.stuck = 0;
        }
        let Some(t) = self.waypoints.get(self.next).copied() else {
            return Cmd::Arrived;
        };
        self.seen.push_back((now_ms, pos));
        while self
            .seen
            .front()
            .is_some_and(|(t0, _)| now_ms.saturating_sub(*t0) > STUCK_MS)
        {
            self.seen.pop_front();
        }
        // stuck: walking for STUCK_MS and the arrow barely moved
        let walked_long = self
            .seen
            .front()
            .is_some_and(|(t0, _)| now_ms.saturating_sub(*t0) >= STUCK_MS - PULSE_MS);
        let moved = self.seen.front().map_or(f32::MAX, |(_, p)| {
            ((p.0 - pos.0).powi(2) + (p.1 - pos.1).powi(2)).sqrt()
        });
        if walked_long && moved < STUCK_PX && !self.last_keys.is_empty() {
            self.stuck += 1;
            self.seen.clear();
            if self.stuck > MAX_STUCK {
                return Cmd::GiveUp(format!(
                    "stuck {MAX_STUCK}x on the way to waypoint {} ({:.0},{:.0})",
                    self.next + 1,
                    t.0,
                    t.1
                ));
            }
            // back up, then strafe - left and right in turn
            let back: Vec<&'static str> = self.last_keys.iter().map(|k| opposite(k)).collect();
            let main = self.last_keys[0];
            let side = match (main, self.stuck % 2) {
                (W | S, 0) => D,
                (W | S, _) => A,
                (_, 0) => W,
                _ => S,
            };
            return Cmd::Moves(vec![(back, BACKUP_MS), (vec![side], STRAFE_MS)]);
        }
        let keys = self.dirs.towards(t.0 - pos.0, t.1 - pos.1);
        self.last_keys = keys.clone();
        Cmd::Moves(vec![(keys, PULSE_MS)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(n: u32) -> Frame {
        Frame::load_png(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(format!("minimap_{n}.png")),
        )
        .unwrap()
    }

    fn near(a: (f32, f32), b: (f32, f32), tol: f32) -> bool {
        (a.0 - b.0).abs() <= tol && (a.1 - b.1).abs() <= tol
    }

    /// Your 5 minimaps: heart, arrow, path ends where they are.
    #[test]
    fn reads_your_minimaps() {
        // (sample, heart, arrow, spawn) - spawn = the far end of the path
        let want = [
            (1, (19.0, 35.0), (74.5, 107.5), (140, 47)),
            (2, (44.0, 134.0), (57.0, 131.5), (85, 10)),
            (3, (97.0, 40.0), (98.0, 93.5), (103, 140)),
            (4, (17.0, 79.0), (111.5, 89.5), (140, 76)),
        ];
        for (n, heart, arrow, spawn) in want {
            let r = read(&load(n));
            let b = r.base.unwrap_or_else(|| panic!("{n}: no heart"));
            assert!(near(b, heart, 4.0), "{n}: heart {b:?}");
            assert!(
                r.players.iter().any(|p| near(p.at, arrow, 4.0)),
                "{n}: arrows {:?}",
                r.players
            );
            let s = r.spawn.unwrap();
            assert!(
                (s.0 - spawn.0).abs() <= 10 && (s.1 - spawn.1).abs() <= 10,
                "{n}: spawn {s:?}"
            );
            // the path runs spawn -> heart
            assert!(r.path.len() >= 10, "{n}: {} points", r.path.len());
            let last = *r.path.last().unwrap();
            assert!(
                near((last.0 as f32, last.1 as f32), heart, 25.0),
                "{n}: ends {last:?}"
            );
        }
        // the 5th: just that it's all there
        let r = read(&load(5));
        assert!(r.base.is_some() && !r.players.is_empty() && r.path.len() >= 10);
        // the newer ones (6 = Assassin Park, 2 lanes; 11 = a unit's range
        // ring and placed units over the path)
        let more = [
            (6, (110.0, 74.0), (81.5, 83.5), (49, 9)),
            (7, (84.0, 12.0), (81.0, 71.5), (75, 136)),
            (8, (51.0, 134.0), (79.5, 134.0), (88, 10)),
            (9, (138.0, 84.5), (51.5, 79.5), (14, 45)),
            (10, (17.0, 80.0), (13.0, 91.5), (140, 85)),
            (11, (138.0, 76.5), (78.0, 68.5), (14, 80)),
        ];
        for (n, heart, arrow, spawn) in more {
            let r = read(&load(n));
            assert!(near(r.base.unwrap(), heart, 4.0), "{n}: heart {:?}", r.base);
            assert_eq!(r.players.len(), 1, "{n}: one arrow, {:?}", r.players);
            assert!(near(r.players[0].at, arrow, 4.0), "{n}: arrow");
            let s = r.spawn.unwrap();
            assert!(
                (s.0 - spawn.0).abs() <= 12 && (s.1 - spawn.1).abs() <= 12,
                "{n}: spawn {s:?}"
            );
            assert_eq!(r.lanes.len(), usize::from(n == 6), "{n}: lanes");
        }
    }

    /// Assassin Park: the purple lane from the bottom spawn, and the stop
    /// is somewhere both lanes are in reach.
    #[test]
    fn two_lanes_one_stop() {
        let r = read(&load(6));
        let s = r.lane_spawns[0];
        assert!((s.0 - 47).abs() <= 8 && (s.1 - 136).abs() <= 8, "{s:?}");
        let d = from_minimap("Assassin Park", &load(6));
        let end = *walk_points_covering(&d, 50).last().unwrap();
        assert!(poly_dist(end, &d.path) <= COVER_PX);
        assert!(poly_dist(end, &d.lanes[0]) <= COVER_PX, "{end:?}");
        // and it did get away from the base
        let b = d.base.unwrap();
        assert!((end.0 - b[0] as f32).hypot(end.1 - b[1] as f32) > 20.0);
    }

    #[test]
    fn covering_walk_stops_where_lanes_part() {
        // main lane: spawn (100,50) -> base (0,50); a 2nd lane joins it at
        // x=30 from straight above
        let d = MapData {
            path: (0..=10).rev().map(|i| [i * 10, 50]).collect(),
            lanes: vec![vec![[30, 0], [30, 50], [0, 50]]],
            ..Default::default()
        };
        let end = *walk_points_covering(&d, 50).last().unwrap();
        // 50% = x 50, but past x 48 the 2nd lane is > 18 px away
        assert!((end.0 - 48.0).abs() <= 2.0 && end.1 == 50.0, "{end:?}");
        // one lane: the plain % walk
        let one = MapData {
            lanes: vec![],
            ..d.clone()
        };
        assert_eq!(walk_points_covering(&one, 50), walk_points(&one, 50));
    }

    /// The arrows on samples 1 and 4 point up (^).
    #[test]
    fn arrow_heading() {
        for n in [1, 4] {
            let r = read(&load(n));
            let h = r.players[0].heading.expect("heading");
            assert!(!(30.0..=330.0).contains(&h), "{n}: {h}");
        }
    }

    #[test]
    fn map_file_from_a_minimap() {
        let d = from_minimap("Navy Bay", &load(1));
        assert_eq!(d.map, "Navy Bay");
        assert!(d.base.is_some() && d.spawn.is_some() && d.path.len() >= 10);
        assert!(d.walk.is_empty());
        let t = serde_json::to_string(&d).unwrap();
        assert_eq!(serde_json::from_str::<MapData>(&t).unwrap(), d);
        assert_eq!(key("Navy Bay (Summer)"), key("navy_bay"));
    }

    #[test]
    fn walks_part_of_the_path() {
        // a straight path, stored spawn (100,10) -> base (0,10): 100 px
        let d = MapData {
            path: (0..=10).rev().map(|i| [i * 10, 10]).collect(),
            ..Default::default()
        };
        let half = walk_points(&d, 50);
        assert_eq!(half.first(), Some(&(0.0, 10.0)), "starts at the base end");
        assert_eq!(half.last(), Some(&(50.0, 10.0)), "stops half way");
        let d35 = MapData {
            walk_pct: Some(35),
            ..d.clone()
        };
        assert_eq!(
            walk_points(&d35, 50).last(),
            Some(&(35.0, 10.0)),
            "this map's own %"
        );
        // your own points win
        let mine = MapData {
            walk: vec![[5, 5], [20, 30]],
            ..d.clone()
        };
        assert_eq!(walk_points(&mine, 50), vec![(5.0, 5.0), (20.0, 30.0)]);
        assert!(walk_points(&MapData::default(), 50).is_empty());
        // standing at 22,10: no going back to the base first
        let rest = from_nearest(&half, (22.0, 10.0));
        assert_eq!(rest.first(), Some(&(20.0, 10.0)));
    }

    #[test]
    fn keys_towards_the_target() {
        // W moves the arrow up the minimap
        let k = KeyDirs::from_w_move(0.0, -4.0).unwrap();
        assert_eq!(k.towards(0.0, -10.0), vec![W]);
        assert_eq!(k.towards(10.0, 0.0), vec![D]);
        assert_eq!(k.towards(-10.0, 10.0), vec![S, A]);
        assert_eq!(k.towards(7.0, -7.0), vec![W, D]);
        // camera turned: W goes right
        let k = KeyDirs::from_w_move(5.0, 0.0).unwrap();
        assert_eq!(k.towards(10.0, 0.0), vec![W]);
        assert_eq!(k.towards(0.0, 10.0), vec![D]);
        assert!(
            KeyDirs::from_w_move(0.5, 0.5).is_none(),
            "too little to tell"
        );
    }

    #[test]
    fn walks_waypoints_and_unsticks() {
        let k = KeyDirs::from_w_move(0.0, -3.0).unwrap();
        let mut nav = Nav::new(k, vec![(50.0, 20.0), (80.0, 20.0)]);
        // heading north to the first one
        assert_eq!(
            nav.step(0, (50.0, 60.0)),
            Cmd::Moves(vec![(vec![W], PULSE_MS)])
        );
        assert_eq!(
            nav.step(200, (50.0, 57.0)),
            Cmd::Moves(vec![(vec![W], PULSE_MS)])
        );
        // a wall: the arrow stays put for 600 ms
        nav.step(400, (50.0, 55.0));
        nav.step(600, (50.0, 55.0));
        nav.step(800, (50.0, 55.0));
        let c = nav.step(1000, (50.0, 55.2));
        match c {
            Cmd::Moves(m) => {
                assert_eq!(m[0], (vec![S], BACKUP_MS), "backs up");
                assert_eq!(m[1].1, STRAFE_MS, "then strafes");
                assert!(m[1].0 == vec![A] || m[1].0 == vec![D]);
            }
            other => panic!("{other:?}"),
        }
        // there: on to the next, east
        assert_eq!(
            nav.step(1500, (50.0, 21.0)),
            Cmd::Moves(vec![(vec![D], PULSE_MS)])
        );
        assert_eq!(nav.step(1700, (79.0, 20.5)), Cmd::Arrived);
    }

    #[test]
    fn gives_up_when_it_never_moves() {
        let k = KeyDirs::from_w_move(0.0, -3.0).unwrap();
        let mut nav = Nav::new(k, vec![(50.0, 10.0)]);
        let mut t = 0;
        let mut last = Cmd::Arrived;
        for _ in 0..200 {
            last = nav.step(t, (50.0, 60.0));
            if matches!(last, Cmd::GiveUp(_)) {
                break;
            }
            t += 200;
        }
        assert!(matches!(last, Cmd::GiveUp(_)), "{last:?}");
    }
}
