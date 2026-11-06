//! B6 step machine, part 2: the game's steps as `StepRunner`s around the
//! existing procedures (`stage_walk`, `stage_vote`, `place_all`, ...), and
//! `RunCtx` as the runner's `Hooks`.
//!
//! Each step's `update()` runs its procedure once: Ok = Advance. The
//! procedure's errors are mapped to a `FailureReason` (`classify`), so the
//! runner's policy decides what's retried and what's Fatal. Long procedures
//! are cut off at the step's hard timeout by `RunCtx::check()` / `sleep()`
//! (the step deadline), not only between updates.
//!
//! The old procedural loop (`run_loop`) stays the default until this is
//! tested; Settings > General > "Step machine (beta)" switches.

use super::run::{RunCtx, STEP_TIMED_OUT};
use super::steps::{AbortSource, FailureReason, Hooks, RetryPolicy, Step, StepResult, StepRunner};
use std::sync::Mutex;
use std::time::Duration;

/// Limits for the wrapped procedures. The step machine's own defaults
/// (`RetryPolicy::default_for`) are for fine-grained steps; a wrapped
/// procedure has long waits of its own inside (captcha 2 min, Vote Start
/// 2 min, end screen 90 s + the Discord upload), so its hard timeout has to
/// sit above those or it'd be cut off mid-way. Place / Upgrade keep theirs.
pub fn wrapped_policy(step: Step) -> RetryPolicy {
    let p = |soft: u64, hard: u64, retries: u32, backoff_ms: u64| RetryPolicy {
        soft_timeout: Duration::from_secs(soft),
        hard_timeout: Duration::from_secs(hard),
        max_retries: retries,
        backoff: Duration::from_millis(backoff_ms),
    };
    match step {
        // walk to the portal, matchmaking, captcha (you type it: 2 min), Start
        Step::Lobby => p(300, 300, 2, 2000),
        // loading + up to 2 min for Vote Start, map, camera, Teleport to Spawn
        Step::StageSetup => p(240, 240, 2, 1000),
        // Next spam up to 90 s, pearls / drops, Discord upload (max 15 s)
        Step::End => p(180, 180, 2, 1000),
        other => RetryPolicy::default_for(other),
    }
}

/// A procedure's error text -> what the runner understands.
pub fn classify(err: &str, step: Step, elapsed: Duration) -> FailureReason {
    if err == "stopped" {
        return FailureReason::ExternalAbort {
            source: AbortSource::UserStop,
        };
    }
    if err.starts_with(super::run::DISCONNECTED) {
        return FailureReason::ExternalAbort {
            source: AbortSource::Disconnect,
        };
    }
    if err == STEP_TIMED_OUT {
        return FailureReason::StepTimeout { step, elapsed };
    }
    // the input layer: "SendInput ... (error N)", "input blocked ..."
    if err.contains("input blocked") || err.contains("SendInput") || err.contains("(error ") {
        let mut r = FailureReason::from_input_error(err);
        if err.contains("input blocked") {
            // Windows refused for the moment: always waited out
            if let FailureReason::InputDispatchFailed { last_error, .. } = &mut r {
                *last_error = 0;
            }
        }
        return r;
    }
    if err == "stage over" {
        return FailureReason::UnexpectedState {
            expected: super::steps::StateTag::InStage,
            got: super::steps::StateTag::Unknown,
        };
    }
    FailureReason::Procedure {
        err: err.to_string(),
    }
}

/// What the steps of one round share.
#[derive(Debug, Default, Clone)]
pub struct Round {
    pub in_lobby: bool,
    /// Map of this round ("" = not recognised).
    pub map: String,
    /// Map of the round before (same map = no Teleport to Spawn).
    pub prev_map: String,
    pub placed: Vec<(crate::geom::Hwnd, usize, crate::geom::Point)>,
    pub won: Option<bool>,
    /// Last run of a schedule step: back to the lobby after it.
    pub last: bool,
    /// End: a defeat with "stop after a defeat" on.
    pub stop_after: bool,
}

/// The steps' context: the game + this round's state.
pub struct Ctx<'a> {
    pub run: &'a RunCtx,
    pub round: Mutex<Round>,
}

impl Ctx<'_> {
    pub fn round(&self) -> std::sync::MutexGuard<'_, Round> {
        self.round.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Arm the deadline breaker for a step with `policy`.
    fn arm(&self, policy: &RetryPolicy) {
        // real seconds (not scaled by `speed`): the procedures' own waits
        // are what's scaled in tests
        let dl = self.run.control.deadline(policy.hard_timeout.as_secs_f64());
        *self
            .run
            .step_deadline
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(dl);
    }
    fn disarm(&self) {
        *self
            .run
            .step_deadline
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }
    fn mode(&self) -> &'static str {
        self.run.settings.run.mode.label()
    }
}

/// Run a procedure once as the step's update.
fn once<'a>(
    ctx: &Ctx<'a>,
    step: Step,
    started: std::time::Instant,
    f: impl FnOnce(&Ctx<'a>) -> Result<(), String>,
) -> Result<StepResult, FailureReason> {
    match f(ctx) {
        Ok(()) => Ok(StepResult::Advance),
        Err(e) => {
            let r = classify(&e, step, started.elapsed());
            if r.is_fatal_immediately() || matches!(r, FailureReason::UnexpectedState { .. }) {
                Ok(StepResult::Fatal { reason: r })
            } else {
                Ok(StepResult::Retry { reason: r })
            }
        }
    }
}

// ---------------- Walk ----------------

/// The map's route on every mover.
pub struct Walk {
    started: std::time::Instant,
    policy: RetryPolicy,
}

impl Walk {
    pub fn new(ctx: &Ctx) -> Self {
        let map = ctx.round().map.clone();
        let movers = ctx.run.movers().len().max(1) as u64;
        let route = (!map.is_empty())
            .then(|| ctx.run.routes.get().find_map(&map).cloned())
            .flatten();
        // every mover walks the route in turn: its fixed time each, plus room
        let policy = match &route {
            Some(r) => RetryPolicy::walk(
                Some(r.duration_ms() * movers + 10_000 + 500 * r.steps.len() as u64),
                r.steps.len(),
            ),
            None => RetryPolicy::walk(None, 0),
        };
        Walk {
            started: std::time::Instant::now(),
            policy,
        }
    }
}

impl StepRunner<Ctx<'_>> for Walk {
    fn step(&self) -> Step {
        Step::Walk
    }
    fn policy(&self) -> RetryPolicy {
        self.policy
    }
    fn enter(&mut self, ctx: &Ctx) -> Result<(), FailureReason> {
        self.started = std::time::Instant::now();
        ctx.arm(&self.policy);
        let map = ctx.round().map.clone();
        super::trace::step(
            "walk",
            serde_json::json!({ "mode": ctx.mode(), "map": map }),
        );
        Ok(())
    }
    fn update(&mut self, ctx: &Ctx) -> Result<StepResult, FailureReason> {
        let map = ctx.round().map.clone();
        once(ctx, Step::Walk, self.started, |c| c.run.stage_walk(&map))
    }
    fn exit(&mut self, ctx: &Ctx) {
        ctx.disarm();
    }
}

// ---------------- the other steps ----------------

/// A step whose update() runs one procedure; `trace` sets the step's
/// params for dumps / resume on enter.
pub struct Simple<'a> {
    step: Step,
    started: std::time::Instant,
    policy: RetryPolicy,
    #[allow(clippy::type_complexity)]
    body: Box<dyn FnMut(&Ctx<'a>) -> Result<(), String> + Send + 'a>,
    trace: Box<dyn Fn(&Ctx<'a>) -> serde_json::Value + Send + 'a>,
}

impl<'a> Simple<'a> {
    fn new(
        step: Step,
        body: impl FnMut(&Ctx<'a>) -> Result<(), String> + Send + 'a,
        trace: impl Fn(&Ctx<'a>) -> serde_json::Value + Send + 'a,
    ) -> Self {
        Simple {
            step,
            started: std::time::Instant::now(),
            policy: wrapped_policy(step),
            body: Box::new(body),
            trace: Box::new(trace),
        }
    }
}

impl<'a> StepRunner<Ctx<'a>> for Simple<'a> {
    fn step(&self) -> Step {
        self.step
    }
    fn policy(&self) -> RetryPolicy {
        self.policy
    }
    fn enter(&mut self, ctx: &Ctx<'a>) -> Result<(), FailureReason> {
        self.started = std::time::Instant::now();
        ctx.arm(&self.policy);
        super::trace::step(self.step.name(), (self.trace)(ctx));
        Ok(())
    }
    fn update(&mut self, ctx: &Ctx<'a>) -> Result<StepResult, FailureReason> {
        // every try gets the step's full time again
        ctx.arm(&self.policy);
        let body = &mut self.body;
        once(ctx, self.step, self.started, |c| body(c))
    }
    fn exit(&mut self, ctx: &Ctx<'a>) {
        ctx.disarm();
    }
}

fn base(ctx: &Ctx) -> serde_json::Value {
    let map = ctx.round().map.clone();
    serde_json::json!({ "mode": ctx.mode(), "map": map })
}

/// Lobby: from the lobby into a stage (mode / map / act / Start).
pub fn lobby<'a>() -> Simple<'a> {
    Simple::new(
        Step::Lobby,
        |c| {
            c.run.enter_stage()?;
            let mut r = c.round();
            r.map.clear();
            r.in_lobby = false;
            Ok(())
        },
        |c| serde_json::json!({ "mode": c.mode() }),
    )
}

/// StageSetup: stage loaded, map spotted, camera, Teleport to Spawn.
pub fn stage_setup<'a>() -> Simple<'a> {
    Simple::new(
        Step::StageSetup,
        |c| {
            let prev = c.round().prev_map.clone();
            let map = c.run.stage_prepare(&prev)?;
            c.round().map = map;
            Ok(())
        },
        base,
    )
}

/// Vote: every client votes Start.
pub fn vote<'a>() -> Simple<'a> {
    Simple::new(Step::Vote, |c| c.run.stage_vote(), base)
}

/// Place: card pick, then every unit (per-unit limits inside place_all).
pub fn place<'a>() -> Simple<'a> {
    Simple::new(
        Step::Place,
        |c| {
            c.run.cards_all()?;
            let map = c.round().map.clone();
            let started = std::time::Instant::now();
            let placed = c.run.place_all(&map)?;
            c.run.log(format!(
                "placed {} unit(s) in {:.0}s",
                placed.len(),
                started.elapsed().as_secs_f32()
            ));
            c.round().placed = placed;
            Ok(())
        },
        base,
    )
}

/// Upgrade: until VICTORY / DEFEAT.
pub fn upgrade<'a>() -> Simple<'a> {
    Simple::new(
        Step::Upgrade,
        |c| {
            let placed = c.round().placed.clone();
            // placement mode closed, UI settled, what's on screen logged
            c.run.place_to_upgrade(&placed)?;
            let won = c.run.upgrade_until_end(&placed)?;
            c.round().won = Some(won);
            Ok(())
        },
        |c| {
            let r = c.round();
            serde_json::json!({
                "mode": c.mode(),
                "map": r.map.clone(),
                "placed": super::run::placed_json(&c.run.clients, &r.placed),
            })
        },
    )
}

/// End: drops + pearls, Win / Loss, Next > reward screen (loot report to
/// Discord, waited for; item clicked again) > Replay / Return. Retries
/// never count the run twice (EndOnce).
pub fn end<'a>() -> Simple<'a> {
    let once = super::run::EndOnce::default();
    Simple::new(
        Step::End,
        move |c| {
            let (won, map, last) = {
                let r = c.round();
                // a Challenge reset passed: back to the lobby for it
                (
                    r.won.unwrap_or(false),
                    r.map.clone(),
                    r.last || c.run.challenge_owed(),
                )
            };
            match c.run.end_round(won, &map, last, &once) {
                Ok(lobby) => {
                    c.round().in_lobby = lobby;
                    Ok(())
                }
                Err(e) if e.starts_with("stopped after") => {
                    c.round().stop_after = true;
                    Ok(())
                }
                Err(e) => Err(e),
            }
        },
        |c| {
            let r = c.round();
            serde_json::json!({ "mode": c.mode(), "map": r.map.clone(), "won": r.won })
        },
    )
}

/// Dungeon: the whole floor (doors by your room order, shop, shrines) -
/// a skeleton until the dungeon screens are known.
pub fn dungeon<'a>() -> Simple<'a> {
    Simple::new(
        Step::Dungeon,
        |c| {
            let won = c.run.dungeon_floor()?;
            c.round().won = Some(won);
            Ok(())
        },
        base,
    )
}

/// One round's steps, from `from` on (resume skips the ones before; Walk is
/// never redone).
pub fn round_steps<'a>(from: Step) -> Vec<Box<dyn StepRunner<Ctx<'a>> + 'a>> {
    round_steps_for(crate::config::Mode::SummerHunt, from)
}

/// `round_steps` for `mode`: Dungeon runs its floor in place of
/// Walk / Vote / Place / Upgrade.
pub fn round_steps_for<'a>(
    mode: crate::config::Mode,
    from: Step,
) -> Vec<Box<dyn StepRunner<Ctx<'a>> + 'a>> {
    let all: &[Step] = if mode == crate::config::Mode::Dungeon {
        &[Step::Lobby, Step::StageSetup, Step::Dungeon, Step::End]
    } else {
        &[
            Step::Lobby,
            Step::StageSetup,
            Step::Walk,
            Step::Vote,
            Step::Place,
            Step::Upgrade,
            Step::End,
        ]
    };
    let i = all.iter().position(|s| *s == from).unwrap_or(0);
    all[i..]
        .iter()
        .map(|s| -> Box<dyn StepRunner<Ctx<'a>> + 'a> {
            match s {
                Step::Lobby => Box::new(lobby()),
                Step::StageSetup => Box::new(stage_setup()),
                Step::Walk => Box::new(WalkLazy(None)),
                Step::Vote => Box::new(vote()),
                Step::Place => Box::new(place()),
                Step::Upgrade => Box::new(upgrade()),
                Step::Dungeon => Box::new(dungeon()),
                Step::End => Box::new(end()),
            }
        })
        .collect()
}

/// Walk's policy depends on the map, known only after StageSetup: built
/// on enter.
struct WalkLazy(Option<Walk>);

impl StepRunner<Ctx<'_>> for WalkLazy {
    fn step(&self) -> Step {
        Step::Walk
    }
    fn policy(&self) -> RetryPolicy {
        self.0
            .as_ref()
            .map(|w| w.policy)
            .unwrap_or_else(|| RetryPolicy::walk(None, 0))
    }
    fn enter(&mut self, ctx: &Ctx) -> Result<(), FailureReason> {
        let mut w = Walk::new(ctx);
        let r = w.enter(ctx);
        self.0 = Some(w);
        r
    }
    fn update(&mut self, ctx: &Ctx) -> Result<StepResult, FailureReason> {
        match self.0.as_mut() {
            Some(w) => w.update(ctx),
            None => Ok(StepResult::Advance),
        }
    }
    fn exit(&mut self, ctx: &Ctx) {
        if let Some(w) = self.0.as_mut() {
            w.exit(ctx);
        }
    }
}

// ---------------- the round loop ----------------

/// Rounds with the step machine until Stop / an error, or `limit` (then
/// back in the lobby, Ok). A Fatal step: dump (runner), pause, ping; after
/// F4 it looks at the screen and carries on from there.
pub(crate) fn run_rounds(
    run: &RunCtx,
    limit: Option<super::run::StepLimit>,
    start_in_lobby: bool,
) -> Result<(), String> {
    use super::run::Event;
    let mut in_lobby = start_in_lobby;
    let mut prev_map = String::new();
    let mut rounds = 0u32;
    let mut fatals = 0u32;
    let reached = |rounds: u32| {
        limit.as_ref().is_some_and(|l| {
            (l.runs > 0 && rounds >= l.runs)
                || l.until.is_some_and(|t| std::time::Instant::now() >= t)
        })
    };
    // where to start: a saved step (restart / after a Fatal) or the top
    let mut resume = run.resume.clone();
    run.log("step machine: on");
    loop {
        // Challenge reset passed: one Challenge run from the lobby
        if in_lobby && resume.is_none() && run.challenge_owed() {
            run.run_challenge_now()?;
            in_lobby = true;
            prev_map.clear();
        }
        let mut round = Round {
            in_lobby,
            prev_map: prev_map.clone(),
            last: reached(rounds + 1),
            ..Default::default()
        };
        let mut from = if in_lobby {
            Step::Lobby
        } else {
            Step::StageSetup
        };
        if let Some(r) = resume.take() {
            if run.where_am_i() == "stage" {
                let vote_up = run.screen(run.main).find_vote_yes().is_some();
                if let Some(at) = super::resume::resume_at(&r.step, vote_up) {
                    round.map = r.step_params["map"].as_str().unwrap_or("").to_string();
                    round.placed = super::resume::placed(&r)
                        .into_iter()
                        .filter_map(|(n, slot, x, y)| {
                            run.clients
                                .get(n.checked_sub(1)?)
                                .map(|c| (c.0, slot, crate::geom::Point::new(x, y)))
                        })
                        .collect();
                    if at == Step::Upgrade && round.placed.is_empty() {
                        from = Step::Place;
                    } else {
                        from = at;
                    }
                    run.log(format!(
                        "step machine: picking up at {} on '{}'",
                        from.name(),
                        round.map
                    ));
                }
            }
        }
        let ctx = Ctx {
            run,
            round: Mutex::new(round),
        };
        let mut steps = round_steps_for(run.settings.run.mode, from);
        let res = super::steps::run_steps(&mut steps, &ctx, run, Duration::from_millis(200));
        let round = ctx.round().clone();
        drop(steps);
        match res {
            Ok(()) => {
                fatals = 0;
                rounds += 1;
                in_lobby = round.in_lobby;
                prev_map = round.map.clone();
                if round.stop_after {
                    return Err("stopped after a defeat".into());
                }
                if reached(rounds) {
                    if !in_lobby {
                        let _ = run.return_to_lobby_now();
                    }
                    return Ok(());
                }
            }
            Err((
                _,
                FailureReason::ExternalAbort {
                    source: AbortSource::UserStop,
                },
            )) => {
                return Err("stopped".into());
            }
            Err((step, reason)) => {
                // a client dropped out: reconnect (the stage is lost - an
                // aborted run, no Discord report), next round from the
                // lobby; can't = relaunch (if it's on)
                let dropped = matches!(
                    reason,
                    FailureReason::ExternalAbort {
                        source: AbortSource::Disconnect
                    }
                ) || run.disconnected().is_some();
                if dropped {
                    match run.recover_disconnect() {
                        Ok(()) => {
                            run.log(format!(
                                "step machine: {} aborted by the disconnect - next round from the lobby",
                                step.name()
                            ));
                            in_lobby = true;
                            prev_map.clear();
                            fatals = 0;
                            run.sleep(500)?;
                            continue;
                        }
                        Err(e) => {
                            return Err(format!(
                                "disconnected from the game - reconnect failed: {e}"
                            ))
                        }
                    }
                }
                fatals += 1;
                if fatals > 3 {
                    return Err(format!(
                        "{} failed 3 rounds in a row: {}",
                        step.name(),
                        reason.short()
                    ));
                }
                let text = format!("{}: {}", step.name(), reason.short());
                run.control.set_paused(true);
                let _ = run.tx.send(Event::Fatal(text));
                // paused here until F4 (or Stop)
                run.check()?;
                let at = run.where_am_i();
                run.log(format!(
                    "step machine: resumed after the failure - on screen: {}",
                    if at.is_empty() { "unknown" } else { at }
                ));
                prev_map = round.map.clone();
                match at {
                    "end" => in_lobby = run.after_end().unwrap_or(in_lobby),
                    "lobby" => in_lobby = true,
                    _ => {
                        in_lobby = false;
                        resume = super::resume::from_trace();
                    }
                }
            }
        }
        run.sleep(500)?;
    }
}

// ---------------- Hooks ----------------

impl Hooks for RunCtx {
    fn should_abort(&self) -> Option<AbortSource> {
        if self.control.is_stopped() {
            Some(AbortSource::UserStop)
        } else {
            // the watchdog saw the disconnect box: the loop reconnects
            self.watch
                .stopping_disconnect()
                .map(|_| AbortSource::Disconnect)
        }
    }
    fn sleep(&self, d: Duration) {
        // a Stop shows up in should_abort() right after
        let _ = self
            .control
            .sleep((d.as_millis() as f32 * self.speed) as u64);
    }
    fn dump(
        &self,
        step: Step,
        reason: &FailureReason,
        attempt: u32,
        sub: Option<serde_json::Value>,
    ) {
        let why = match &sub {
            Some(v) => format!("{} (attempt {attempt}) {v}", reason.short()),
            None => format!("{} (attempt {attempt})", reason.short()),
        };
        self.dump_failure(step.name(), &why, None, vec![]);
    }
    fn save_state(&self, _step: Step, _sub: Option<serde_json::Value>) {
        // every step's enter() calls trace::step with its params, and the
        // resume hook saves runtime/last_state.json from that: nothing to add
    }
    fn advanced(&self, _step: Step) {}
    fn fatal(&self, step: Step, reason: &FailureReason) {
        self.log(format!("FATAL {}: {}", step.name(), reason.short()));
    }
    fn status(&self, _step: Step, attempt: u32, _last: Option<&FailureReason>) {
        if attempt == 0 {
            crate::status::clear_attempt(self.main);
        } else {
            crate::status::attempt(self.main, attempt + 1, 0);
        }
    }
    fn input_blocked(&self, step: Step, first: bool) {
        if first {
            self.log(format!(
                "WARN input refused by Windows during {} (PC locked / RDP minimized?) - waiting it out",
                step.name()
            ));
        } else {
            log::debug!("input still refused during {}", step.name());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_order_and_resume_points() {
        let names = |from| {
            round_steps(from)
                .iter()
                .map(|s| s.step().name())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(Step::Lobby),
            [
                "lobby",
                "stage_setup",
                "walk",
                "vote",
                "place",
                "upgrade",
                "end"
            ]
        );
        // resume never walks again
        assert_eq!(names(Step::Vote), ["vote", "place", "upgrade", "end"]);
        assert_eq!(names(Step::Upgrade), ["upgrade", "end"]);
        // a dungeon: the floor instead of walk / vote / place / upgrade
        let d: Vec<&str> = round_steps_for(crate::config::Mode::Dungeon, Step::Lobby)
            .iter()
            .map(|s| s.step().name())
            .collect();
        assert_eq!(d, ["lobby", "stage_setup", "dungeon", "end"]);
    }

    #[test]
    fn wrapped_limits_sit_above_the_procedures_own_waits() {
        // captcha 120 s, Vote Start 120 s, end screen 90 s + Discord 15 s
        assert!(wrapped_policy(Step::Lobby).hard_timeout >= Duration::from_secs(120 + 60));
        assert!(wrapped_policy(Step::StageSetup).hard_timeout >= Duration::from_secs(120 + 60));
        assert!(
            wrapped_policy(Step::End).hard_timeout
                >= Duration::from_secs(90 + super::super::loot::WEBHOOK_WAIT_SECS + 30)
        );
        // the approved ones stay
        assert_eq!(
            wrapped_policy(Step::Place),
            RetryPolicy::default_for(Step::Place)
        );
        assert_eq!(
            wrapped_policy(Step::Upgrade),
            RetryPolicy::default_for(Step::Upgrade)
        );
    }

    #[test]
    fn errors_map_to_reasons() {
        let d = Duration::from_secs(3);
        assert_eq!(
            classify("stopped", Step::Place, d),
            FailureReason::ExternalAbort {
                source: AbortSource::UserStop
            }
        );
        assert_eq!(
            classify(STEP_TIMED_OUT, Step::Walk, d),
            FailureReason::StepTimeout {
                step: Step::Walk,
                elapsed: d
            }
        );
        let blocked = classify(
            "upgrade: input blocked by Windows (error 5)",
            Step::Upgrade,
            d,
        );
        assert!(blocked.is_environmental(), "{blocked:?}");
        let hard = classify("next: SendInput sent 0/2 (error 87)", Step::End, d);
        assert!(hard.is_fatal_immediately(), "{hard:?}");
        let other = classify("vote: Vote Start not seen", Step::Vote, d);
        assert!(other.is_retryable() && !other.is_fatal_immediately());
        assert!(matches!(
            classify("stage over", Step::Upgrade, d),
            FailureReason::UnexpectedState { .. }
        ));
    }
}
