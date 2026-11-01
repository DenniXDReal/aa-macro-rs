//! The input arbiter: the ONLY code that touches mouse and keyboard.
//!
//! Roblox reads real input (`SendInput`), and real input goes to whichever
//! window has focus. So with 4 clients, workers never send input themselves.
//! They submit a short [`Batch`] ("press 2, click (412,300), press q") and the
//! arbiter runs batches one at a time:
//!
//! 1. pick the highest-priority waiting batch (FIFO within a priority),
//! 2. focus that batch's window (fails cleanly if it can't),
//! 3. run the actions in client coordinates,
//! 4. release every key / button it pressed - always, even on stop or error.
//!
//! Detection (capture + vision) runs in parallel per worker; only input waits
//! its turn. Keep batches short (a few hundred ms) so clients don't starve.

use crate::control::Control;
use crate::geom::{Geometry, Hwnd, Point};
use crate::keys::Key;
use crate::platform::{MouseButton, Platform};
use std::cmp::Ordering as CmpOrdering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// A live look at the screen: where a pattern is right now (client coords),
/// or `None`. Built by the game code; the arbiter only calls it.
#[derive(Clone)]
pub struct Probe {
    pub label: String,
    pub find: Arc<dyn Fn() -> Option<Point> + Send + Sync>,
}

impl Probe {
    pub fn new(
        label: impl Into<String>,
        f: impl Fn() -> Option<Point> + Send + Sync + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            find: Arc::new(f),
        }
    }
}

impl std::fmt::Debug for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Probe({})", self.label)
    }
}

impl PartialEq for Probe {
    fn eq(&self, o: &Self) -> bool {
        self.label == o.label
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Press and release, holding `hold_ms`.
    KeyTap {
        key: Key,
        hold_ms: u64,
    },
    KeyDown(Key),
    KeyUp(Key),
    /// Cursor to a client point (with the hover wiggle).
    MoveTo(Point),
    /// Move, wiggle, settle, press, hold, release.
    Click {
        at: Point,
        button: MouseButton,
    },
    /// Hover `at`, then wheel `notches` one at a time (+ = up / zoom in).
    Scroll {
        at: Point,
        notches: i32,
    },
    /// Relative motion split into `steps` (first-person camera).
    MoveRel {
        dx: i32,
        dy: i32,
        steps: u32,
    },
    /// Hold `button` while moving relatively (right-drag rotates the camera).
    Drag {
        button: MouseButton,
        dx: i32,
        dy: i32,
        steps: u32,
    },
    Wait(u64),
    /// Wait until `probe` sees something. `required`: error when it never shows.
    WaitFor {
        probe: Probe,
        timeout_ms: u64,
        required: bool,
    },
    /// Wait for `probe`, then click where it is (or `fallback` after the timeout).
    ClickWhen {
        probe: Probe,
        timeout_ms: u64,
        fallback: Option<Point>,
        button: MouseButton,
    },
    /// Hover `at` and wheel one notch at a time until `probe` sees something,
    /// at most `max` notches.
    ScrollUntil {
        probe: Probe,
        at: Point,
        /// Scroll where this pattern is (+ `offset`) instead of `at`, when it's visible.
        anchor: Option<Probe>,
        offset: Point,
        up: bool,
        max: u32,
        /// Reaching `max` is fine (plain "scroll N notches here").
        max_ok: bool,
    },
    /// Run `then` when `probe` sees something right now, else `otherwise`.
    Branch {
        probe: Probe,
        then: Vec<Action>,
        otherwise: Vec<Action>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low = 0,
    Normal = 1,
    /// End screens, rejoin, anything time-critical.
    High = 2,
}

#[derive(Clone, Debug)]
pub struct Batch {
    pub window: Hwnd,
    pub priority: Priority,
    pub label: String,
    pub actions: Vec<Action>,
    /// Movement / navigation: holds focus for seconds on purpose (no warning).
    pub long: bool,
}

impl Batch {
    pub fn new(window: Hwnd, label: impl Into<String>) -> Self {
        Self {
            window,
            priority: Priority::Normal,
            label: label.into(),
            actions: vec![],
            long: false,
        }
    }
    pub fn priority(mut self, p: Priority) -> Self {
        self.priority = p;
        self
    }
    pub fn tap(mut self, key: Key) -> Self {
        self.actions.push(Action::KeyTap { key, hold_ms: 40 });
        self
    }
    pub fn click(mut self, x: i32, y: i32) -> Self {
        self.actions.push(Action::Click {
            at: Point::new(x, y),
            button: MouseButton::Left,
        });
        self
    }
    pub fn move_to(mut self, x: i32, y: i32) -> Self {
        self.actions.push(Action::MoveTo(Point::new(x, y)));
        self
    }
    pub fn wait(mut self, ms: u64) -> Self {
        self.actions.push(Action::Wait(ms));
        self
    }
    pub fn then(mut self, a: Action) -> Self {
        self.actions.push(a);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputError {
    Stopped,
    WindowGone,
    Focus(String),
    OutsideClient(Point),
    Platform(String),
    Shutdown,
    /// A pattern the batch waited for never showed up.
    NotSeen(String),
}

impl std::fmt::Display for InputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InputError::Stopped => f.write_str("stopped"),
            InputError::WindowGone => f.write_str("Roblox window is gone"),
            InputError::Focus(e) => write!(f, "could not focus Roblox: {e}"),
            InputError::OutsideClient(p) => {
                write!(f, "point ({}, {}) is outside the client", p.x, p.y)
            }
            InputError::Platform(e) => write!(f, "input failed: {e}"),
            InputError::Shutdown => f.write_str("input arbiter shut down"),
            InputError::NotSeen(p) => write!(f, "'{p}' never showed up"),
        }
    }
}
impl std::error::Error for InputError {}

#[derive(Clone, Copy, Debug)]
pub struct Report {
    /// Time spent queued behind other windows.
    pub waited: Duration,
    /// Time this batch held focus.
    pub held: Duration,
}

#[derive(Clone, Copy, Debug)]
pub struct ArbiterOptions {
    pub mouse_settle_ms: u64,
    pub click_hold_ms: u64,
    pub max_lease_ms: u64,
}

impl Default for ArbiterOptions {
    fn default() -> Self {
        Self {
            mouse_settle_ms: 35,
            click_hold_ms: 40,
            max_lease_ms: 400,
        }
    }
}

struct Queued {
    batch: Batch,
    seq: u64,
    enqueued: Instant,
    reply: Sender<Result<Report, InputError>>,
}

impl PartialEq for Queued {
    fn eq(&self, o: &Self) -> bool {
        self.seq == o.seq
    }
}
impl Eq for Queued {}
impl PartialOrd for Queued {
    fn partial_cmp(&self, o: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(o))
    }
}
impl Ord for Queued {
    /// Max-heap: higher priority first, then lower seq (older) first.
    fn cmp(&self, o: &Self) -> CmpOrdering {
        self.batch
            .priority
            .cmp(&o.batch.priority)
            .then_with(|| o.seq.cmp(&self.seq))
    }
}

/// A pending batch; `wait()` blocks until it ran.
pub struct Ticket(Receiver<Result<Report, InputError>>);

impl Ticket {
    pub fn wait(self) -> Result<Report, InputError> {
        self.0.recv().unwrap_or(Err(InputError::Shutdown))
    }
}

/// Clonable submit side, handed to every worker.
#[derive(Clone)]
pub struct ArbiterHandle {
    tx: Sender<Queued>,
    seq: Arc<AtomicU64>,
}

impl ArbiterHandle {
    pub fn submit(&self, batch: Batch) -> Ticket {
        let (reply, rx) = mpsc::channel();
        let q = Queued {
            batch,
            seq: self.seq.fetch_add(1, Ordering::SeqCst),
            enqueued: Instant::now(),
            reply,
        };
        if let Err(mpsc::SendError(q)) = self.tx.send(q) {
            let _ = q.reply.send(Err(InputError::Shutdown));
        }
        Ticket(rx)
    }
    pub fn run(&self, batch: Batch) -> Result<Report, InputError> {
        self.submit(batch).wait()
    }
}

pub struct Arbiter {
    handle: Option<ArbiterHandle>,
    join: Option<JoinHandle<()>>,
}

impl Arbiter {
    pub fn start(platform: Arc<dyn Platform>, control: Control, opts: ArbiterOptions) -> Self {
        let (tx, rx) = mpsc::channel::<Queued>();
        let join = std::thread::Builder::new()
            .name("input-arbiter".into())
            .spawn(move || arbiter_loop(rx, platform, control, opts))
            .expect("spawn arbiter");
        Self {
            handle: Some(ArbiterHandle {
                tx,
                seq: Arc::new(AtomicU64::new(0)),
            }),
            join: Some(join),
        }
    }
    pub fn handle(&self) -> ArbiterHandle {
        self.handle.clone().expect("arbiter running")
    }
    pub fn submit(&self, batch: Batch) -> Ticket {
        self.handle.as_ref().expect("arbiter running").submit(batch)
    }
    pub fn run(&self, batch: Batch) -> Result<Report, InputError> {
        self.submit(batch).wait()
    }
}

impl Drop for Arbiter {
    fn drop(&mut self) {
        // Arbiter exits once every handle (incl. workers' clones) is dropped.
        self.handle.take();
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn arbiter_loop(
    rx: Receiver<Queued>,
    platform: Arc<dyn Platform>,
    control: Control,
    opts: ArbiterOptions,
) {
    let mut heap = BinaryHeap::new();
    loop {
        if heap.is_empty() {
            match rx.recv() {
                Ok(q) => heap.push(q),
                Err(_) => return,
            }
        }
        while let Ok(q) = rx.try_recv() {
            heap.push(q);
        }
        let q = heap.pop().expect("non-empty");
        let waited = q.enqueued.elapsed();
        let mut runner = Runner {
            p: platform.as_ref(),
            ctl: &control,
            opts,
            keys: vec![],
            buttons: vec![],
        };
        let t = Instant::now();
        let res = runner.run(&q.batch);
        runner.release_all();
        let held = t.elapsed();
        if held > Duration::from_millis(opts.max_lease_ms) && !q.batch.long {
            log::warn!(
                "batch '{}' held focus {} ms (> {} ms): split it so other clients don't wait",
                q.batch.label,
                held.as_millis(),
                opts.max_lease_ms
            );
        }
        let _ = q.reply.send(res.map(|_| Report { waited, held }));
    }
}

struct Runner<'a> {
    p: &'a dyn Platform,
    ctl: &'a Control,
    opts: ArbiterOptions,
    keys: Vec<Key>,
    buttons: Vec<MouseButton>,
}

fn perr(e: anyhow::Error) -> InputError {
    InputError::Platform(e.to_string())
}

impl Runner<'_> {
    fn run(&mut self, b: &Batch) -> Result<(), InputError> {
        self.ctl.checkpoint().map_err(|_| InputError::Stopped)?;
        let geo = self.p.geometry(b.window).ok_or(InputError::WindowGone)?;
        // Validate every point before taking focus.
        fn check(geo: &Geometry, acts: &[Action]) -> Result<(), InputError> {
            for a in acts {
                match a {
                    Action::MoveTo(p)
                    | Action::Click { at: p, .. }
                    | Action::Scroll { at: p, .. }
                    | Action::ScrollUntil { at: p, .. }
                    | Action::ClickWhen {
                        fallback: Some(p), ..
                    } => {
                        if !geo.in_client(*p) {
                            return Err(InputError::OutsideClient(*p));
                        }
                    }
                    Action::Branch {
                        then, otherwise, ..
                    } => {
                        check(geo, then)?;
                        check(geo, otherwise)?;
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        check(&geo, &b.actions)?;
        self.focus(b.window)?;
        for a in &b.actions {
            if self.ctl.is_stopped() {
                return Err(InputError::Stopped);
            }
            self.act(&geo, b.window, a)?;
        }
        Ok(())
    }

    fn focus(&self, w: Hwnd) -> Result<(), InputError> {
        if self.p.foreground() == Some(w) && !self.p.is_minimized(w) {
            return Ok(());
        }
        self.p
            .focus(w)
            .map_err(|e| InputError::Focus(e.to_string()))
    }

    /// Pause-aware sleep. On pause: let go of everything, wait, re-focus and
    /// press back what was held.
    fn sleep(&mut self, ms: u64, w: Hwnd) -> Result<(), InputError> {
        if self.ctl.is_paused() {
            let keys = self.keys.clone();
            let buttons = self.buttons.clone();
            self.release_all();
            self.ctl.checkpoint().map_err(|_| InputError::Stopped)?;
            self.focus(w)?;
            for k in keys {
                self.key(k, true)?;
            }
            for b in buttons {
                self.button(b, true)?;
            }
        }
        self.ctl.sleep(ms).map_err(|_| InputError::Stopped)
    }

    fn key(&mut self, k: Key, down: bool) -> Result<(), InputError> {
        self.p.key(k, down).map_err(perr)?;
        if down {
            if !self.keys.contains(&k) {
                self.keys.push(k);
            }
        } else {
            self.keys.retain(|x| *x != k);
        }
        Ok(())
    }

    fn button(&mut self, b: MouseButton, down: bool) -> Result<(), InputError> {
        self.p.mouse_button(b, down).map_err(perr)?;
        if down {
            if !self.buttons.contains(&b) {
                self.buttons.push(b);
            }
        } else {
            self.buttons.retain(|x| *x != b);
        }
        Ok(())
    }

    /// Real move + 1 px wiggle so Roblox registers the hover, ending exactly on target.
    fn hover(&mut self, geo: &Geometry, at: Point) -> Result<(), InputError> {
        let s = geo.client_to_screen(at);
        self.p.mouse_move_abs(s.x + 1, s.y).map_err(perr)?;
        self.p.mouse_move_abs(s.x, s.y).map_err(perr)?;
        Ok(())
    }

    fn rel(&mut self, dx: i32, dy: i32, steps: u32, w: Hwnd) -> Result<(), InputError> {
        let steps = steps.max(1) as i32;
        let (mut sx, mut sy) = (0, 0);
        for i in 1..=steps {
            let (tx, ty) = (dx * i / steps, dy * i / steps);
            self.p.mouse_move_rel(tx - sx, ty - sy).map_err(perr)?;
            sx = tx;
            sy = ty;
            self.sleep(5, w)?;
        }
        Ok(())
    }

    fn act(&mut self, geo: &Geometry, w: Hwnd, a: &Action) -> Result<(), InputError> {
        match a {
            Action::KeyTap { key, hold_ms } => {
                self.key(*key, true)?;
                self.sleep(*hold_ms, w)?;
                self.key(*key, false)?;
            }
            Action::KeyDown(k) => self.key(*k, true)?,
            Action::KeyUp(k) => self.key(*k, false)?,
            Action::MoveTo(p) => self.hover(geo, *p)?,
            Action::Click { at, button } => {
                self.hover(geo, *at)?;
                self.sleep(self.opts.mouse_settle_ms, w)?;
                self.button(*button, true)?;
                self.sleep(self.opts.click_hold_ms, w)?;
                self.button(*button, false)?;
            }
            Action::Scroll { at, notches } => {
                self.hover(geo, *at)?;
                for _ in 0..notches.unsigned_abs() {
                    // Each notch preceded by a wiggle, or scroll lists ignore the wheel.
                    self.hover(geo, *at)?;
                    self.p.wheel(notches.signum()).map_err(perr)?;
                    self.sleep(30, w)?;
                }
            }
            Action::MoveRel { dx, dy, steps } => self.rel(*dx, *dy, *steps, w)?,
            Action::Drag {
                button,
                dx,
                dy,
                steps,
            } => {
                self.button(*button, true)?;
                self.sleep(20, w)?;
                self.rel(*dx, *dy, *steps, w)?;
                self.button(*button, false)?;
            }
            Action::Wait(ms) => self.sleep(*ms, w)?,
            Action::WaitFor {
                probe,
                timeout_ms,
                required,
            } => {
                if self.wait_probe(probe, *timeout_ms, w)?.is_none() && *required {
                    return Err(InputError::NotSeen(probe.label.clone()));
                }
            }
            Action::ClickWhen {
                probe,
                timeout_ms,
                fallback,
                button,
            } => {
                let at = match self.wait_probe(probe, *timeout_ms, w)? {
                    Some(p) => p,
                    None => fallback.ok_or_else(|| InputError::NotSeen(probe.label.clone()))?,
                };
                if !geo.in_client(at) {
                    return Err(InputError::OutsideClient(at));
                }
                self.act(
                    geo,
                    w,
                    &Action::Click {
                        at,
                        button: *button,
                    },
                )?;
            }
            Action::ScrollUntil {
                probe,
                at,
                anchor,
                offset,
                up,
                max,
                max_ok,
            } => {
                let at = anchor
                    .as_ref()
                    .and_then(|a| (a.find)())
                    .map(|p| Point::new(p.x + offset.x, p.y + offset.y))
                    .filter(|p| geo.in_client(*p))
                    .unwrap_or(*at);
                self.hover(geo, at)?;
                let mut n = 0;
                while (probe.find)().is_none() {
                    if n >= *max {
                        if *max_ok {
                            break;
                        }
                        return Err(InputError::NotSeen(probe.label.clone()));
                    }
                    self.hover(geo, at)?;
                    self.p.wheel(if *up { 1 } else { -1 }).map_err(perr)?;
                    self.sleep(220, w)?;
                    n += 1;
                }
            }
            Action::Branch {
                probe,
                then,
                otherwise,
            } => {
                let branch = if (probe.find)().is_some() {
                    then
                } else {
                    otherwise
                };
                for a in branch {
                    if self.ctl.is_stopped() {
                        return Err(InputError::Stopped);
                    }
                    self.act(geo, w, a)?;
                }
            }
        }
        Ok(())
    }

    /// Poll `probe` every ~80 ms for up to `timeout_ms` (pause-aware).
    fn wait_probe(
        &mut self,
        probe: &Probe,
        timeout_ms: u64,
        w: Hwnd,
    ) -> Result<Option<Point>, InputError> {
        let mut dl = self.ctl.deadline(timeout_ms as f64 / 1000.0);
        loop {
            if let Some(p) = (probe.find)() {
                return Ok(Some(p));
            }
            if dl.expired() {
                return Ok(None);
            }
            self.sleep(80, w)?;
        }
    }

    fn release_all(&mut self) {
        for k in std::mem::take(&mut self.keys) {
            let _ = self.p.key(k, false);
        }
        for b in std::mem::take(&mut self.buttons) {
            let _ = self.p.mouse_button(b, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::mock::{Call, MockPlatform};

    fn setup(n: usize) -> (Arc<MockPlatform>, Control, Arbiter) {
        let mock = Arc::new(MockPlatform::with_windows(n));
        let ctl = Control::new();
        let opts = ArbiterOptions {
            mouse_settle_ms: 1,
            click_hold_ms: 1,
            max_lease_ms: 400,
        };
        let arb = Arbiter::start(mock.clone(), ctl.clone(), opts);
        (mock, ctl, arb)
    }

    #[test]
    fn click_focuses_then_converts_to_screen() {
        let (mock, _c, arb) = setup(2);
        arb.run(Batch::new(Hwnd(101), "t").click(10, 20)).unwrap();
        let calls = mock.take_calls();
        // window 101 client starts at (816+8, 31)
        assert_eq!(
            calls,
            vec![
                Call::Focus(Hwnd(101)),
                Call::MoveAbs(835, 51),
                Call::MoveAbs(834, 51),
                Call::Button(MouseButton::Left, true),
                Call::Button(MouseButton::Left, false),
            ]
        );
    }

    #[test]
    fn already_focused_window_is_not_refocused() {
        let (mock, _c, arb) = setup(1);
        arb.run(Batch::new(Hwnd(100), "a").tap(Key::new(0x10)))
            .unwrap();
        arb.run(Batch::new(Hwnd(100), "b").tap(Key::new(0x10)))
            .unwrap();
        let focuses = mock
            .take_calls()
            .into_iter()
            .filter(|c| matches!(c, Call::Focus(_)))
            .count();
        assert_eq!(focuses, 1);
    }

    #[test]
    fn high_priority_jumps_the_queue() {
        let (mock, _c, arb) = setup(3);
        let a = arb.submit(Batch::new(Hwnd(100), "long").wait(120));
        std::thread::sleep(Duration::from_millis(30)); // A is running
        let low = arb.submit(
            Batch::new(Hwnd(101), "low")
                .priority(Priority::Low)
                .tap(Key::new(0x10)),
        );
        let high = arb.submit(
            Batch::new(Hwnd(102), "high")
                .priority(Priority::High)
                .tap(Key::new(0x10)),
        );
        a.wait().unwrap();
        high.wait().unwrap();
        low.wait().unwrap();
        let order: Vec<Hwnd> = mock
            .take_calls()
            .into_iter()
            .filter_map(|c| {
                if let Call::Focus(h) = c {
                    Some(h)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(order, vec![Hwnd(100), Hwnd(102), Hwnd(101)]);
    }

    #[test]
    fn stop_mid_batch_releases_held_keys() {
        let (mock, ctl, arb) = setup(1);
        let w = Key::parse("w").unwrap();
        let t = arb.submit(
            Batch::new(Hwnd(100), "walk")
                .then(Action::KeyDown(w))
                .wait(5_000),
        );
        std::thread::sleep(Duration::from_millis(40));
        ctl.stop();
        assert_eq!(t.wait().unwrap_err(), InputError::Stopped);
        let calls = mock.take_calls();
        assert_eq!(calls.last(), Some(&Call::Key(w, false)));
    }

    #[test]
    fn refused_focus_sends_no_input() {
        let (mock, _c, arb) = setup(1);
        mock.refuse_focus.lock().unwrap().insert(Hwnd(100));
        let e = arb.run(Batch::new(Hwnd(100), "x").click(5, 5)).unwrap_err();
        assert!(matches!(e, InputError::Focus(_)));
        assert!(mock.take_calls().is_empty());
    }

    #[test]
    fn outside_point_rejected_before_focus() {
        let (mock, _c, arb) = setup(1);
        let e = arb
            .run(Batch::new(Hwnd(100), "x").click(900, 5))
            .unwrap_err();
        assert_eq!(e, InputError::OutsideClient(Point::new(900, 5)));
        assert!(mock.take_calls().is_empty());
    }

    #[test]
    fn missing_window_is_reported() {
        let (_m, _c, arb) = setup(1);
        assert_eq!(
            arb.run(Batch::new(Hwnd(999), "x").click(1, 1)).unwrap_err(),
            InputError::WindowGone
        );
    }

    #[test]
    fn rel_motion_sums_exactly() {
        let (mock, _c, arb) = setup(1);
        arb.run(Batch::new(Hwnd(100), "cam").then(Action::MoveRel {
            dx: 100,
            dy: -37,
            steps: 7,
        }))
        .unwrap();
        let (sx, sy) = mock
            .take_calls()
            .into_iter()
            .fold((0, 0), |(x, y), c| match c {
                Call::MoveRel(dx, dy) => (x + dx, y + dy),
                _ => (x, y),
            });
        assert_eq!((sx, sy), (100, -37));
    }

    #[test]
    fn click_when_waits_for_the_probe() {
        let (mock, _c, arb) = setup(1);
        let t0 = Instant::now();
        let probe = Probe::new("btn", move || {
            (t0.elapsed() > Duration::from_millis(150)).then_some(Point::new(50, 60))
        });
        arb.run(Batch::new(Hwnd(100), "x").then(Action::ClickWhen {
            probe,
            timeout_ms: 2000,
            fallback: None,
            button: MouseButton::Left,
        }))
        .unwrap();
        assert!(mock.take_calls().contains(&Call::MoveAbs(58, 91)));
    }

    #[test]
    fn missing_pattern_is_reported_and_fallback_used() {
        let (mock, _c, arb) = setup(1);
        let never = Probe::new("ghost", || None);
        let e = arb
            .run(Batch::new(Hwnd(100), "x").then(Action::WaitFor {
                probe: never.clone(),
                timeout_ms: 100,
                required: true,
            }))
            .unwrap_err();
        assert_eq!(e, InputError::NotSeen("ghost".into()));
        arb.run(Batch::new(Hwnd(100), "x").then(Action::ClickWhen {
            probe: never,
            timeout_ms: 50,
            fallback: Some(Point::new(1, 2)),
            button: MouseButton::Left,
        }))
        .unwrap();
        assert!(mock.take_calls().contains(&Call::MoveAbs(9, 33)));
    }

    #[test]
    fn scroll_until_and_branch() {
        let (mock, _c, arb) = setup(1);
        let hits = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let h2 = hits.clone();
        let probe = Probe::new("world", move || {
            (h2.fetch_add(1, Ordering::SeqCst) >= 3).then_some(Point::new(5, 5))
        });
        let w = Key::parse("w").unwrap();
        arb.run(
            Batch::new(Hwnd(100), "x")
                .then(Action::ScrollUntil {
                    probe,
                    at: Point::new(10, 10),
                    anchor: None,
                    offset: Point::default(),
                    up: false,
                    max: 10,
                    max_ok: false,
                })
                .then(Action::Branch {
                    probe: Probe::new("no", || None),
                    then: vec![],
                    otherwise: vec![Action::KeyDown(w)],
                }),
        )
        .unwrap();
        let calls = mock.take_calls();
        assert_eq!(
            calls
                .iter()
                .filter(|c| matches!(c, Call::Wheel(-1)))
                .count(),
            3
        );
        assert!(calls.contains(&Call::Key(w, true)));
        assert_eq!(
            calls.last(),
            Some(&Call::Key(w, false)),
            "held key released at the end"
        );
    }

    #[test]
    fn many_workers_share_one_arbiter() {
        let (mock, _c, arb) = setup(4);
        let hs: Vec<_> = (0..4)
            .map(|i| {
                let h = arb.handle();
                std::thread::spawn(move || {
                    for _ in 0..5 {
                        h.run(Batch::new(Hwnd(100 + i), "w").click(1, 1)).unwrap();
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        // Every click is a contiguous down/up pair: no interleaving between windows.
        let calls = mock.take_calls();
        let buttons: Vec<_> = calls
            .iter()
            .filter(|c| matches!(c, Call::Button(..)))
            .collect();
        assert_eq!(buttons.len(), 40);
        for pair in buttons.chunks(2) {
            assert_eq!(pair[0], &Call::Button(MouseButton::Left, true));
            assert_eq!(pair[1], &Call::Button(MouseButton::Left, false));
        }
    }
}
