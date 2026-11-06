//! Data for the debug overlay. The macro publishes what it just did (pattern
//! hits / misses, OCR boxes + text, clicks) here; the overlay thread reads
//! the latest `DebugSnapshot` ~30x a second. Last value wins (`ArcSwap`): no
//! queue grows, the macro never waits for drawing, and while the overlay is
//! off nothing is recorded at all.

use crate::geom::Hwnd;
use crate::geom::{Point, Rect};
use arc_swap::ArcSwap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How long a mark stays on the overlay.
pub const MARK_TTL: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug, PartialEq)]
pub enum MarkKind {
    /// Pattern found (green): name + score.
    Hit,
    /// Pattern looked for and not found (red), shown when misses are on.
    Miss,
    /// OCR read: box + recognised text.
    Ocr,
    /// Click target: crosshair + why.
    Click,
}

#[derive(Clone, Debug)]
pub struct Mark {
    pub kind: MarkKind,
    pub window: Hwnd,
    /// Client px (for a click: a 1x1 rect at the point).
    pub rect: Rect,
    pub text: String,
    pub at: Instant,
}

#[derive(Clone, Debug)]
pub struct ClientInfo {
    pub window: Hwnd,
    /// 1 = main.
    pub n: usize,
    pub generation: u64,
    /// Age of this client's newest picture.
    pub capture_age_ms: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct DebugSnapshot {
    pub clients: Vec<ClientInfo>,
    pub marks: Vec<Mark>,
    pub step: String,
    pub step_ms: u64,
    pub show_misses: bool,
}

static ON: AtomicBool = AtomicBool::new(false);
static MISSES: AtomicBool = AtomicBool::new(false);
static SNAP: LazyLock<ArcSwap<DebugSnapshot>> =
    LazyLock::new(|| ArcSwap::from_pointee(DebugSnapshot::default()));

struct Store {
    marks: Vec<Mark>,
    clients: Vec<Hwnd>,
    captured: Vec<(Hwnd, Instant)>,
}

static STORE: Mutex<Store> = Mutex::new(Store {
    marks: Vec::new(),
    clients: Vec::new(),
    captured: Vec::new(),
});

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
    if !on {
        let mut s = STORE.lock().unwrap_or_else(|e| e.into_inner());
        s.marks.clear();
    }
}

pub fn set_show_misses(on: bool) {
    MISSES.store(on, Ordering::Relaxed);
}

/// The clients of the run, in client-number order.
pub fn set_clients(c: &[Hwnd]) {
    STORE.lock().unwrap_or_else(|e| e.into_inner()).clients = c.to_vec();
}

/// A client's picture was just taken.
pub fn captured(window: Hwnd) {
    if !enabled() {
        return;
    }
    let mut s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    s.captured.retain(|(w, _)| *w != window);
    s.captured.push((window, Instant::now()));
}

fn push(kind: MarkKind, window: Hwnd, rect: Rect, text: String) {
    if !enabled() {
        return;
    }
    if kind == MarkKind::Miss && !MISSES.load(Ordering::Relaxed) {
        return;
    }
    let mut s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    s.marks.retain(|m| now.duration_since(m.at) < MARK_TTL);
    // the same box again (polling): refresh it instead of piling up
    s.marks
        .retain(|m| !(m.kind == kind && m.window == window && m.rect == rect));
    if s.marks.len() > 400 {
        s.marks.drain(..100);
    }
    s.marks.push(Mark {
        kind,
        window,
        rect,
        text,
        at: now,
    });
}

pub fn hit(window: Hwnd, rect: Rect, name: &str, score: f32) {
    push(MarkKind::Hit, window, rect, format!("{name} {score:.2}"));
}

pub fn miss(window: Hwnd, rect: Rect, name: &str) {
    push(MarkKind::Miss, window, rect, format!("{name} ✗"));
}

pub fn ocr(window: Hwnd, rect: Rect, text: &str) {
    let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    push(MarkKind::Ocr, window, rect, t.chars().take(60).collect());
}

pub fn click(window: Hwnd, at: Point, why: &str) {
    push(
        MarkKind::Click,
        window,
        Rect::new(at.x, at.y, 1, 1),
        why.to_string(),
    );
}

/// Build the snapshot from what's been recorded (the overlay calls this
/// through `latest`; cheap, ~30x a second).
fn rebuild() {
    let s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let gens = crate::input::generation_all();
    let clients = s
        .clients
        .iter()
        .enumerate()
        .map(|(i, w)| ClientInfo {
            window: *w,
            n: i + 1,
            generation: gens
                .iter()
                .find(|(h, _)| h == w)
                .map(|(_, g)| *g)
                .unwrap_or(0),
            capture_age_ms: s
                .captured
                .iter()
                .find(|(h, _)| h == w)
                .map(|(_, t)| now.duration_since(*t).as_millis() as u64),
        })
        .collect();
    let step = crate::game::trace::current();
    let snap = DebugSnapshot {
        clients,
        marks: s
            .marks
            .iter()
            .filter(|m| now.duration_since(m.at) < MARK_TTL)
            .cloned()
            .collect(),
        step: step
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "idle".into()),
        step_ms: step
            .map(|s| s.since.elapsed().as_millis() as u64)
            .unwrap_or(0),
        show_misses: MISSES.load(Ordering::Relaxed),
    };
    drop(s);
    SNAP.store(Arc::new(snap));
}

/// The newest snapshot (rebuilt on each call - the overlay is the only
/// reader and calls it at its own pace).
pub fn latest() -> Arc<DebugSnapshot> {
    rebuild();
    SNAP.load_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_only_while_on() {
        let w: Hwnd = Hwnd(0x77);
        set_enabled(false);
        hit(w, Rect::new(1, 2, 3, 4), "off", 1.0);
        assert!(latest().marks.iter().all(|m| m.window != w));
        set_enabled(true);
        set_clients(&[w]);
        hit(w, Rect::new(1, 2, 3, 4), "vote_start", 0.91);
        hit(w, Rect::new(1, 2, 3, 4), "vote_start", 0.93); // same box: replaced
        miss(w, Rect::new(9, 9, 9, 9), "hidden"); // misses off: not kept
        ocr(w, Rect::new(5, 5, 50, 10), "UPG  [3]");
        click(w, Point::new(10, 20), "vote start");
        let s = latest();
        let mine: Vec<&Mark> = s.marks.iter().filter(|m| m.window == w).collect();
        assert_eq!(mine.len(), 3, "{mine:?}");
        assert!(mine.iter().any(|m| m.text == "vote_start 0.93"));
        assert!(mine.iter().any(|m| m.text == "UPG [3]"));
        assert_eq!(s.clients[0].n, 1);
        set_enabled(false);
    }
}
