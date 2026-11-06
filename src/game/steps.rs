//! B6 step machine, part 1: the types and the runner that drives one step
//! (enter / update / exit) under its `RetryPolicy`, with the watchdog
//! (hard timeout, N retries in a row -> Fatal) and abort handling.
//!
//! Steps don't own the world - they get a context `C` (the real `RunCtx` in
//! the game, a fake in tests) and report what happened. Everything around a
//! step - dumps, saving the state, pausing on Fatal - goes through `Hooks`,
//! so the runner itself stays testable without a game.
//!
//! Guarantees:
//! - `exit()` is called exactly once after `enter()`, whatever happens:
//!   Advance, Fatal, abort, or a panic inside the step.
//! - `ExternalAbort` and hard input failures are Fatal at once, never
//!   retried; input refused for the moment (RDP minimized, PC locked) is
//!   waited out every 10 s without counting as a retry, pausing or dumping.
//! - A retryable failure is retried (after `backoff`) up to `max_retries`
//!   times in a row, then it's Fatal. The 2nd retry onwards asks for a dump.
//! - Past `hard_timeout` the step is Fatal(StepTimeout), whatever it returns.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Step {
    Lobby,
    StageSetup,
    Walk,
    Vote,
    Place,
    Upgrade,
    End,
}

impl Step {
    pub fn name(self) -> &'static str {
        match self {
            Step::Lobby => "lobby",
            Step::StageSetup => "stage_setup",
            Step::Walk => "walk",
            Step::Vote => "vote",
            Step::Place => "place",
            Step::Upgrade => "upgrade",
            Step::End => "end",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum OcrWhat {
    UpgNum,
    UpgLine,
    MapName,
    Other(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateTag {
    InLobby,
    InStage,
    PanelOpen,
    PanelClosed,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbortSource {
    UserStop,
    FocusLost,
    Panic,
    Shutdown,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FailureReason {
    PatternNotFound {
        name: String,
        best_score: f32,
    },
    OcrUnreadable {
        what: OcrWhat,
    },
    OcrDisagreement {
        what: OcrWhat,
        reads: Vec<String>,
    },
    ClickHadNoEffect {
        what: String,
        generation_before: u64,
    },
    StepTimeout {
        step: Step,
        elapsed: Duration,
    },
    UnexpectedState {
        expected: StateTag,
        got: StateTag,
    },
    /// Sending input failed. `last_error` is GetLastError after SendInput:
    /// 0 = Windows refused for the moment (RDP minimized, session locked,
    /// focus away) - environmental, waited out; anything else is fatal.
    InputDispatchFailed {
        err: String,
        last_error: u32,
    },
    /// Navigation (used once minimap nav lands).
    NavStuck {
        waypoint_index: usize,
    },
    NavFailed {
        reason: String,
    },
    ExternalAbort {
        source: AbortSource,
    },
}

impl FailureReason {
    pub fn is_retryable(&self) -> bool {
        use FailureReason::*;
        matches!(
            self,
            PatternNotFound { .. }
                | OcrUnreadable { .. }
                | OcrDisagreement { .. }
                | ClickHadNoEffect { .. }
                | StepTimeout { .. }
                | NavStuck { .. }
        )
    }
    pub fn is_fatal_immediately(&self) -> bool {
        use FailureReason::*;
        match self {
            ExternalAbort { .. } => true,
            InputDispatchFailed { .. } => !self.is_environmental(),
            _ => false,
        }
    }
    /// Input refused for the moment (RDP minimized, PC locked): waited out
    /// every 10 s for as long as it lasts, never counted as a retry.
    pub fn is_environmental(&self) -> bool {
        matches!(
            self,
            FailureReason::InputDispatchFailed { last_error: 0, .. }
        )
    }
    /// From the input layer's error text ("SendInput sent 0/2 (error 5) ...").
    pub fn from_input_error(err: &str) -> Self {
        let last_error = err
            .split("(error ")
            .nth(1)
            .and_then(|r| r.split(')').next())
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0);
        FailureReason::InputDispatchFailed {
            err: err.to_string(),
            last_error,
        }
    }
    /// Short text for logs / dump folder reasons (stable for the 5 s dedup).
    pub fn short(&self) -> String {
        use FailureReason::*;
        match self {
            PatternNotFound { name, .. } => format!("pattern {name} not found"),
            OcrUnreadable { what } => format!("{what:?} unreadable"),
            OcrDisagreement { what, .. } => format!("{what:?} reads disagree"),
            ClickHadNoEffect { what, .. } => format!("click on {what} had no effect"),
            StepTimeout { step, .. } => format!("{} timed out", step.name()),
            UnexpectedState { expected, got } => format!("expected {expected:?}, got {got:?}"),
            InputDispatchFailed { last_error, .. } => format!("input failed (error {last_error})"),
            NavStuck { waypoint_index } => format!("stuck at waypoint {waypoint_index}"),
            NavFailed { reason } => format!("navigation failed: {reason}"),
            ExternalAbort { source } => format!("aborted ({source:?})"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RetryPolicy {
    /// Beyond this a step starts retrying its sub-operations (step-defined).
    pub soft_timeout: Duration,
    /// Beyond this: Fatal(StepTimeout), whatever the step says.
    pub hard_timeout: Duration,
    /// Retries in a row before it's Fatal.
    pub max_retries: u32,
    /// Pause before the next update() after a Retry.
    pub backoff: Duration,
}

impl RetryPolicy {
    const fn new(soft_s: u64, hard_s: u64, max_retries: u32, backoff_ms: u64) -> Self {
        Self {
            soft_timeout: Duration::from_secs(soft_s),
            hard_timeout: Duration::from_secs(hard_s),
            max_retries,
            backoff: Duration::from_millis(backoff_ms),
        }
    }

    /// Starting values per step (tune from real dumps). Walk's come from the
    /// route (`walk`); Upgrade's retries count per unit inside the step.
    pub fn default_for(step: Step) -> Self {
        match step {
            Step::Vote => Self::new(30, 90, 3, 2000),
            Step::Lobby => Self::new(15, 45, 5, 500),
            Step::StageSetup => Self::new(10, 25, 3, 1000),
            Step::Walk => Self::walk(None, 0),
            // whole step: a safety net (Place takes seconds, Upgrade the
            // whole match); per-unit limits are UNIT_PLACE / UNIT_UPGRADE
            Step::Place => Self::new(600, 600, 1, 5000),
            Step::Upgrade => Self::new(2400, 2400, 1, 5000),
            Step::End => Self::new(5, 15, 2, 1000),
        }
    }

    /// Per unit inside Place: a unit failing past this is logged + skipped.
    pub const UNIT_PLACE: Self = Self::new(8, 20, 3, 500);
    /// Per unit inside Upgrade: same.
    pub const UNIT_UPGRADE: Self = Self::new(15, 45, 3, 500);
    /// Waiting out refused input (RDP minimized / PC locked).
    pub const INPUT_BLOCKED_WAIT: Duration = Duration::from_secs(10);

    /// Walk: the route's own `timeout_ms`, else 10 s + 0.5 s per waypoint.
    /// No step-level retries: the navigator recovers by itself.
    pub fn walk(route_timeout_ms: Option<u64>, waypoints: usize) -> Self {
        let hard = route_timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_millis(10_000 + 500 * waypoints as u64));
        Self {
            soft_timeout: hard,
            hard_timeout: hard,
            max_retries: 0,
            backoff: Duration::ZERO,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StepResult {
    Continue,
    Advance,
    Retry { reason: FailureReason },
    Fatal { reason: FailureReason },
}

pub trait StepRunner<C>: Send {
    fn step(&self) -> Step;
    /// Once, before the first update(). Err = the step's first failure.
    fn enter(&mut self, ctx: &C) -> Result<(), FailureReason>;
    /// Each tick until Advance / Fatal.
    fn update(&mut self, ctx: &C) -> Result<StepResult, FailureReason>;
    /// Clean-up. Always called once after enter(), whatever happened.
    fn exit(&mut self, ctx: &C);
    fn policy(&self) -> RetryPolicy {
        RetryPolicy::default_for(self.step())
    }
    /// Where inside the step it is (unit 3 of 7...), for dumps / resume.
    fn sub(&self) -> Option<serde_json::Value> {
        None
    }
}

/// Everything around the steps: abort checks, sleeping, dumps, saving.
pub trait Hooks {
    fn should_abort(&self) -> Option<AbortSource>;
    /// Sleep (interruptible by the caller's own means).
    fn sleep(&self, d: Duration);
    /// Ask for a failure dump (the dump code applies its 5 s dedup).
    fn dump(
        &self,
        step: Step,
        reason: &FailureReason,
        attempt: u32,
        sub: Option<serde_json::Value>,
    );
    /// Persist where we are (Advance: the next step; Fatal: this step).
    fn save_state(&self, step: Step, sub: Option<serde_json::Value>);
    /// A step advanced: input generation moves on.
    fn advanced(&self, step: Step);
    /// Fatal: pause, beep, log at ERROR.
    fn fatal(&self, step: Step, reason: &FailureReason);
    /// Live info for the overlay (attempt, latest reason).
    fn status(&self, _step: Step, _attempt: u32, _last: Option<&FailureReason>) {}
    /// Dump an intentional Stop too? (Settings, default off)
    /// Input refused for the moment (`first` = first in a row: log at WARN,
    /// later ones at DEBUG). No pause, no beep, no dump.
    fn input_blocked(&self, _step: Step, _first: bool) {}
    fn dump_on_user_stop(&self) -> bool {
        false
    }
}

/// How one step ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Advanced,
    Fatal(FailureReason),
}

/// Drive one step to Advance or Fatal. Ticks every `tick` while it says
/// Continue.
pub fn run_step<C>(
    runner: &mut dyn StepRunner<C>,
    ctx: &C,
    hooks: &dyn Hooks,
    tick: Duration,
) -> Outcome {
    let step = runner.step();
    let policy = runner.policy();
    let started = Instant::now();
    let mut attempt = 0u32;
    let mut last: Option<FailureReason> = None;

    // a retryable failure: count it, maybe dump, maybe escalate
    let mut blocked = false;
    let mut on_failure =
        |r: FailureReason, sub: Option<serde_json::Value>| -> Option<FailureReason> {
            if r.is_environmental() {
                // wait it out, as long as it lasts; not counted as a retry
                hooks.input_blocked(step, !blocked);
                blocked = true;
                hooks.sleep(RetryPolicy::INPUT_BLOCKED_WAIT);
                return None;
            }
            blocked = false;
            if r.is_fatal_immediately() || !r.is_retryable() {
                return Some(r);
            }
            attempt += 1;
            hooks.status(step, attempt, Some(&r));
            if attempt > policy.max_retries {
                return Some(r);
            }
            if attempt >= 2 {
                hooks.dump(step, &r, attempt, sub);
            }
            hooks.sleep(policy.backoff);
            None
        };

    let body = catch_unwind(AssertUnwindSafe(|| -> Outcome {
        if let Some(src) = hooks.should_abort() {
            return Outcome::Fatal(FailureReason::ExternalAbort { source: src });
        }
        if let Err(r) = runner.enter(ctx) {
            last = Some(r.clone());
            if let Some(f) = on_failure(r, runner.sub()) {
                return Outcome::Fatal(f);
            }
        }
        loop {
            if let Some(src) = hooks.should_abort() {
                return Outcome::Fatal(FailureReason::ExternalAbort { source: src });
            }
            let elapsed = started.elapsed();
            if elapsed > policy.hard_timeout {
                return Outcome::Fatal(FailureReason::StepTimeout { step, elapsed });
            }
            let r = match runner.update(ctx) {
                Ok(r) => r,
                Err(e) => StepResult::Retry { reason: e },
            };
            match r {
                StepResult::Continue => hooks.sleep(tick),
                StepResult::Advance => return Outcome::Advanced,
                StepResult::Fatal { reason } => return Outcome::Fatal(reason),
                StepResult::Retry { reason } => {
                    last = Some(reason.clone());
                    if let Some(f) = on_failure(reason, runner.sub()) {
                        return Outcome::Fatal(f);
                    }
                }
            }
        }
    }));
    let outcome = match body {
        Ok(o) => o,
        Err(_) => Outcome::Fatal(FailureReason::ExternalAbort {
            source: AbortSource::Panic,
        }),
    };
    // clean-up, always (a panicking exit() must not take the runner down)
    let _ = catch_unwind(AssertUnwindSafe(|| runner.exit(ctx)));
    let sub = runner.sub();
    match &outcome {
        Outcome::Advanced => {
            hooks.status(step, 0, None);
            hooks.advanced(step);
        }
        Outcome::Fatal(r) => {
            let _ = &last;
            let user_stop = matches!(
                r,
                FailureReason::ExternalAbort {
                    source: AbortSource::UserStop
                }
            );
            if !user_stop || hooks.dump_on_user_stop() {
                hooks.dump(step, r, attempt, sub.clone());
            }
            hooks.save_state(step, sub);
            hooks.status(step, attempt, Some(r));
            hooks.fatal(step, r);
        }
    }
    outcome
}

/// Run steps in order; stops at the first Fatal. Each Advance saves the
/// next step as the place to resume from.
pub fn run_steps<C>(
    steps: &mut [Box<dyn StepRunner<C>>],
    ctx: &C,
    hooks: &dyn Hooks,
    tick: Duration,
) -> Result<(), (Step, FailureReason)> {
    for i in 0..steps.len() {
        let step = steps[i].step();
        match run_step(steps[i].as_mut(), ctx, hooks, tick) {
            Outcome::Advanced => {
                if let Some(next) = steps.get(i + 1) {
                    hooks.save_state(next.step(), next.sub());
                }
            }
            Outcome::Fatal(r) => return Err((step, r)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Mutex;

    /// Records everything the runner asks of the outside world.
    #[derive(Default)]
    struct Rec {
        abort: Mutex<Option<AbortSource>>,
        dumps: Mutex<Vec<(Step, String, u32)>>,
        saved: Mutex<Vec<(Step, Option<serde_json::Value>)>>,
        fatals: Mutex<Vec<(Step, String)>>,
        advanced: Mutex<Vec<Step>>,
        slept: Mutex<Vec<Duration>>,
        dump_stop: AtomicBool,
    }

    impl Hooks for Rec {
        fn should_abort(&self) -> Option<AbortSource> {
            *self.abort.lock().unwrap()
        }
        fn sleep(&self, d: Duration) {
            self.slept.lock().unwrap().push(d);
            std::thread::sleep(d.min(Duration::from_millis(2)));
        }
        fn dump(
            &self,
            step: Step,
            r: &FailureReason,
            attempt: u32,
            _sub: Option<serde_json::Value>,
        ) {
            self.dumps.lock().unwrap().push((step, r.short(), attempt));
        }
        fn save_state(&self, step: Step, sub: Option<serde_json::Value>) {
            self.saved.lock().unwrap().push((step, sub));
        }
        fn advanced(&self, step: Step) {
            self.advanced.lock().unwrap().push(step);
        }
        fn fatal(&self, step: Step, r: &FailureReason) {
            self.fatals.lock().unwrap().push((step, r.short()));
        }
        fn dump_on_user_stop(&self) -> bool {
            self.dump_stop.load(Ordering::Relaxed)
        }
    }

    /// A step that plays back a script of update() results.
    struct Scripted {
        step: Step,
        script: RefCell<Vec<Result<StepResult, FailureReason>>>,
        enter_err: Option<FailureReason>,
        policy: RetryPolicy,
        exits: AtomicU32,
        panic_in_update: bool,
        sub: Cell<u32>,
    }

    // SAFETY for tests: used from one thread only
    unsafe impl Send for Scripted {}

    impl Scripted {
        fn new(step: Step, script: Vec<Result<StepResult, FailureReason>>) -> Self {
            Self {
                step,
                script: RefCell::new(script),
                enter_err: None,
                policy: RetryPolicy {
                    soft_timeout: Duration::from_millis(50),
                    hard_timeout: Duration::from_secs(5),
                    max_retries: 3,
                    backoff: Duration::from_millis(1),
                },
                exits: AtomicU32::new(0),
                panic_in_update: false,
                sub: Cell::new(0),
            }
        }
    }

    impl StepRunner<()> for Scripted {
        fn step(&self) -> Step {
            self.step
        }
        fn enter(&mut self, _: &()) -> Result<(), FailureReason> {
            match self.enter_err.take() {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }
        fn update(&mut self, _: &()) -> Result<StepResult, FailureReason> {
            if self.panic_in_update {
                panic!("step blew up");
            }
            self.sub.set(self.sub.get() + 1);
            let mut s = self.script.borrow_mut();
            if s.is_empty() {
                Ok(StepResult::Continue)
            } else {
                s.remove(0)
            }
        }
        fn exit(&mut self, _: &()) {
            self.exits.fetch_add(1, Ordering::Relaxed);
        }
        fn policy(&self) -> RetryPolicy {
            self.policy
        }
        fn sub(&self) -> Option<serde_json::Value> {
            Some(serde_json::json!({ "unit": self.sub.get() }))
        }
    }

    fn pnf() -> FailureReason {
        FailureReason::PatternNotFound {
            name: "vote_start".into(),
            best_score: 0.8,
        }
    }
    fn retry() -> Result<StepResult, FailureReason> {
        Ok(StepResult::Retry { reason: pnf() })
    }
    const TICK: Duration = Duration::from_millis(1);

    #[test]
    fn advance_right_away() {
        let rec = Rec::default();
        let mut s = Scripted::new(Step::Place, vec![Ok(StepResult::Advance)]);
        assert_eq!(run_step(&mut s, &(), &rec, TICK), Outcome::Advanced);
        assert_eq!(*rec.advanced.lock().unwrap(), vec![Step::Place]);
        assert!(rec.dumps.lock().unwrap().is_empty());
        assert_eq!(s.exits.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn continue_until_advance() {
        let rec = Rec::default();
        let mut s = Scripted::new(
            Step::Lobby,
            vec![
                Ok(StepResult::Continue),
                Ok(StepResult::Continue),
                Ok(StepResult::Advance),
            ],
        );
        assert_eq!(run_step(&mut s, &(), &rec, TICK), Outcome::Advanced);
        assert_eq!(
            rec.slept.lock().unwrap().len(),
            2,
            "slept a tick per Continue"
        );
    }

    #[test]
    fn hard_timeout_is_fatal_with_dump_and_state() {
        let rec = Rec::default();
        let mut s = Scripted::new(Step::Vote, vec![]); // Continue forever
        s.policy.hard_timeout = Duration::from_millis(30);
        match run_step(&mut s, &(), &rec, TICK) {
            Outcome::Fatal(FailureReason::StepTimeout { step, .. }) => assert_eq!(step, Step::Vote),
            o => panic!("{o:?}"),
        }
        assert_eq!(rec.dumps.lock().unwrap().len(), 1, "Fatal always dumps");
        assert_eq!(
            rec.saved.lock().unwrap()[0].0,
            Step::Vote,
            "resume at the failed step"
        );
        assert_eq!(rec.fatals.lock().unwrap().len(), 1);
        assert_eq!(s.exits.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn retry_twice_then_advance_dumps_once() {
        let rec = Rec::default();
        let mut s = Scripted::new(Step::Place, vec![retry(), retry(), Ok(StepResult::Advance)]);
        assert_eq!(run_step(&mut s, &(), &rec, TICK), Outcome::Advanced);
        let d = rec.dumps.lock().unwrap();
        assert_eq!(d.len(), 1, "only the 2nd attempt dumps");
        assert_eq!(d[0].2, 2);
        assert!(rec.fatals.lock().unwrap().is_empty());
        assert!(
            rec.slept
                .lock()
                .unwrap()
                .contains(&Duration::from_millis(1)),
            "backoff"
        );
    }

    #[test]
    fn too_many_retries_escalate() {
        let rec = Rec::default();
        let mut s = Scripted::new(Step::Place, vec![retry(), retry(), retry(), retry()]);
        assert_eq!(run_step(&mut s, &(), &rec, TICK), Outcome::Fatal(pnf()));
        // dumps at attempts 2 and 3, then the Fatal one
        assert_eq!(rec.dumps.lock().unwrap().len(), 3);
    }

    #[test]
    fn attempts_reset_per_step_run() {
        let rec = Rec::default();
        let mut a = Scripted::new(Step::Place, vec![retry(), retry(), Ok(StepResult::Advance)]);
        let mut b = Scripted::new(Step::Place, vec![retry(), retry(), Ok(StepResult::Advance)]);
        assert_eq!(run_step(&mut a, &(), &rec, TICK), Outcome::Advanced);
        assert_eq!(
            run_step(&mut b, &(), &rec, TICK),
            Outcome::Advanced,
            "fresh count"
        );
    }

    #[test]
    fn unexpected_state_is_fatal_without_retries() {
        let rec = Rec::default();
        let r = FailureReason::UnexpectedState {
            expected: StateTag::InStage,
            got: StateTag::InLobby,
        };
        let mut s = Scripted::new(
            Step::Upgrade,
            vec![
                Ok(StepResult::Retry { reason: r.clone() }),
                Ok(StepResult::Advance),
            ],
        );
        assert_eq!(run_step(&mut s, &(), &rec, TICK), Outcome::Fatal(r));
        assert!(rec.slept.lock().unwrap().is_empty(), "no backoff, no retry");
    }

    #[test]
    fn input_failure_is_fatal_at_once() {
        let rec = Rec::default();
        let r = FailureReason::InputDispatchFailed {
            err: "SendInput 0/2 (error 5)".into(),
            last_error: 5,
        };
        let mut s = Scripted::new(Step::Place, vec![Err(r.clone())]);
        assert_eq!(run_step(&mut s, &(), &rec, TICK), Outcome::Fatal(r));
        assert_eq!(rec.dumps.lock().unwrap().len(), 1);
    }

    #[test]
    fn step_returning_fatal_is_fatal() {
        let rec = Rec::default();
        let r = FailureReason::NavFailed {
            reason: "no route".into(),
        };
        let mut s = Scripted::new(
            Step::Walk,
            vec![Ok(StepResult::Fatal { reason: r.clone() })],
        );
        assert_eq!(run_step(&mut s, &(), &rec, TICK), Outcome::Fatal(r));
    }

    #[test]
    fn abort_mid_step_no_dump_by_default() {
        let rec = Rec::default();
        *rec.abort.lock().unwrap() = Some(AbortSource::UserStop);
        let mut s = Scripted::new(Step::Upgrade, vec![]);
        let o = run_step(&mut s, &(), &rec, TICK);
        assert_eq!(
            o,
            Outcome::Fatal(FailureReason::ExternalAbort {
                source: AbortSource::UserStop
            })
        );
        assert!(
            rec.dumps.lock().unwrap().is_empty(),
            "a Stop isn't a failure"
        );
        assert_eq!(rec.saved.lock().unwrap().len(), 1, "state still saved");
        assert_eq!(s.exits.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn abort_dumps_when_asked() {
        let rec = Rec::default();
        rec.dump_stop.store(true, Ordering::Relaxed);
        *rec.abort.lock().unwrap() = Some(AbortSource::UserStop);
        let mut s = Scripted::new(Step::Upgrade, vec![]);
        let _ = run_step(&mut s, &(), &rec, TICK);
        assert_eq!(rec.dumps.lock().unwrap().len(), 1);
    }

    #[test]
    fn other_aborts_always_dump() {
        let rec = Rec::default();
        *rec.abort.lock().unwrap() = Some(AbortSource::Shutdown);
        let mut s = Scripted::new(Step::End, vec![]);
        let _ = run_step(&mut s, &(), &rec, TICK);
        assert_eq!(rec.dumps.lock().unwrap().len(), 1);
    }

    #[test]
    fn panic_in_step_still_calls_exit() {
        let rec = Rec::default();
        let mut s = Scripted::new(Step::Place, vec![]);
        s.panic_in_update = true;
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {})); // keep test output clean
        let o = run_step(&mut s, &(), &rec, TICK);
        std::panic::set_hook(prev);
        assert_eq!(
            o,
            Outcome::Fatal(FailureReason::ExternalAbort {
                source: AbortSource::Panic
            })
        );
        assert_eq!(s.exits.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn enter_error_counts_as_first_retry() {
        let rec = Rec::default();
        let mut s = Scripted::new(Step::StageSetup, vec![Ok(StepResult::Advance)]);
        s.enter_err = Some(pnf());
        assert_eq!(run_step(&mut s, &(), &rec, TICK), Outcome::Advanced);
        let mut s2 = Scripted::new(Step::StageSetup, vec![]);
        s2.enter_err = Some(FailureReason::InputDispatchFailed {
            err: "x".into(),
            last_error: 87,
        });
        assert!(matches!(
            run_step(&mut s2, &(), &rec, TICK),
            Outcome::Fatal(_)
        ));
        assert_eq!(
            s2.exits.load(Ordering::Relaxed),
            1,
            "exit after a failed enter too"
        );
    }

    #[test]
    fn sub_state_goes_into_fatal_save() {
        let rec = Rec::default();
        let mut s = Scripted::new(
            Step::Upgrade,
            vec![Ok(StepResult::Continue), Ok(StepResult::Continue)],
        );
        s.policy.hard_timeout = Duration::from_millis(20);
        let _ = run_step(&mut s, &(), &rec, TICK);
        let saved = rec.saved.lock().unwrap();
        assert_eq!(saved[0].0, Step::Upgrade);
        assert!(saved[0].1.as_ref().unwrap()["unit"].as_u64().unwrap() >= 2);
    }

    #[test]
    fn sequence_saves_next_and_stops_at_fatal() {
        let rec = Rec::default();
        let mut steps: Vec<Box<dyn StepRunner<()>>> = vec![
            Box::new(Scripted::new(
                Step::StageSetup,
                vec![Ok(StepResult::Advance)],
            )),
            Box::new(Scripted::new(Step::Place, vec![Ok(StepResult::Advance)])),
            Box::new(Scripted::new(
                Step::Upgrade,
                vec![Ok(StepResult::Fatal {
                    reason: FailureReason::OcrUnreadable {
                        what: OcrWhat::UpgNum,
                    },
                })],
            )),
            Box::new(Scripted::new(Step::End, vec![Ok(StepResult::Advance)])),
        ];
        let r = run_steps(&mut steps, &(), &rec, TICK);
        assert_eq!(r.unwrap_err().0, Step::Upgrade);
        assert_eq!(
            *rec.advanced.lock().unwrap(),
            vec![Step::StageSetup, Step::Place]
        );
        let saved: Vec<Step> = rec.saved.lock().unwrap().iter().map(|s| s.0).collect();
        assert_eq!(saved, vec![Step::Place, Step::Upgrade, Step::Upgrade]);
    }

    #[test]
    fn retry_classification() {
        assert!(pnf().is_retryable());
        assert!(FailureReason::NavStuck { waypoint_index: 2 }.is_retryable());
        assert!(!FailureReason::NavFailed { reason: "x".into() }.is_retryable());
        assert!(!FailureReason::UnexpectedState {
            expected: StateTag::PanelOpen,
            got: StateTag::PanelClosed
        }
        .is_retryable());
        assert!(FailureReason::ExternalAbort {
            source: AbortSource::FocusLost
        }
        .is_fatal_immediately());
    }

    #[test]
    fn input_refused_is_waited_out_not_retried() {
        let rec = Rec::default();
        let blocked =
            FailureReason::from_input_error("SendInput sent 0/2 (error 0) - input blocked");
        assert!(blocked.is_environmental() && !blocked.is_fatal_immediately());
        // more refusals than max_retries: still not Fatal, never dumped
        let mut s = Scripted::new(
            Step::Upgrade,
            vec![
                Err(blocked.clone()),
                Err(blocked.clone()),
                Err(blocked.clone()),
                Err(blocked.clone()),
                Err(blocked.clone()),
                Ok(StepResult::Advance),
            ],
        );
        assert_eq!(run_step(&mut s, &(), &rec, TICK), Outcome::Advanced);
        assert!(rec.dumps.lock().unwrap().is_empty());
        assert!(rec.fatals.lock().unwrap().is_empty());
        let waits = rec
            .slept
            .lock()
            .unwrap()
            .iter()
            .filter(|d| **d == RetryPolicy::INPUT_BLOCKED_WAIT)
            .count();
        assert_eq!(waits, 5, "10 s each");
    }

    #[test]
    fn hard_input_error_is_fatal() {
        let r = FailureReason::from_input_error("SendInput sent 0/2 (error 5) - access denied");
        assert_eq!(
            r,
            FailureReason::InputDispatchFailed {
                err: "SendInput sent 0/2 (error 5) - access denied".into(),
                last_error: 5
            }
        );
        assert!(r.is_fatal_immediately() && !r.is_environmental());
        assert!(FailureReason::from_input_error("no code at all").is_environmental());
    }

    #[test]
    fn step_level_policies() {
        let p = RetryPolicy::default_for(Step::Place);
        assert_eq!(
            (p.hard_timeout, p.max_retries, p.backoff),
            (Duration::from_secs(600), 1, Duration::from_secs(5))
        );
        let u = RetryPolicy::default_for(Step::Upgrade);
        assert_eq!(
            (u.hard_timeout, u.max_retries),
            (Duration::from_secs(2400), 1)
        );
        assert_eq!(
            RetryPolicy::UNIT_UPGRADE.hard_timeout,
            Duration::from_secs(45)
        );
        assert_eq!(RetryPolicy::UNIT_PLACE.max_retries, 3);
    }

    #[test]
    fn walk_policy_from_route() {
        let p = RetryPolicy::walk(Some(45_000), 12);
        assert_eq!(p.hard_timeout, Duration::from_secs(45));
        assert_eq!(p.max_retries, 0);
        let q = RetryPolicy::walk(None, 12);
        assert_eq!(q.hard_timeout, Duration::from_millis(16_000));
        assert_eq!(
            RetryPolicy::default_for(Step::Vote).hard_timeout,
            Duration::from_secs(90)
        );
    }
}
