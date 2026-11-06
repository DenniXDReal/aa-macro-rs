//! What the game loop is doing right now, for failure dumps, the debug
//! overlay and resuming: the current step (+ its params, since when) and a
//! ring buffer of the last log lines.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

/// Log lines kept in memory.
pub const LOG_RING: usize = 600;

#[derive(Clone, Debug)]
pub struct StepNow {
    pub name: String,
    pub params: serde_json::Value,
    pub since: Instant,
    pub since_wall: SystemTime,
}

static STEP: Mutex<Option<StepNow>> = Mutex::new(None);
static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// Called on every step change (save/resume hooks in here).
static ON_STEP: Mutex<Vec<fn(&StepNow)>> = Mutex::new(Vec::new());

/// The loop moved on to `name` (e.g. "stage_setup", "place", "upgrade").
pub fn step(name: &str, params: serde_json::Value) {
    let s = StepNow {
        name: name.to_string(),
        params,
        since: Instant::now(),
        since_wall: SystemTime::now(),
    };
    *STEP.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
    let hooks: Vec<fn(&StepNow)> = ON_STEP.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for h in hooks {
        h(&s);
    }
}

pub fn on_step(f: fn(&StepNow)) {
    ON_STEP.lock().unwrap_or_else(|e| e.into_inner()).push(f);
}

pub fn current() -> Option<StepNow> {
    STEP.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn current_name() -> String {
    current().map(|s| s.name).unwrap_or_else(|| "idle".into())
}

/// Keep a log line (timestamped).
pub fn log(line: &str) {
    let t = chrono::Local::now().format("%H:%M:%S%.3f");
    let mut l = LOG.lock().unwrap_or_else(|e| e.into_inner());
    if l.len() >= LOG_RING {
        l.pop_front();
    }
    l.push_back(format!("[{t}] {line}"));
}

/// The last `n` lines; with `client` only lines about that client (or no
/// client at all).
pub fn last_lines(n: usize, client: Option<usize>) -> Vec<String> {
    let l = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let mine = |s: &str| match client {
        None => true,
        Some(c) => {
            let tags = [format!("client {c}"), format!("c{c} "), format!("c{c}:")];
            let any_client = s.contains("client ") || has_short_tag(s);
            !any_client || tags.iter().any(|t| s.contains(t.as_str()))
        }
    };
    let v: Vec<String> = l.iter().filter(|s| mine(s)).cloned().collect();
    v[v.len().saturating_sub(n)..].to_vec()
}

fn has_short_tag(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(2)).any(|i| {
        b[i] == b'c'
            && (i == 0 || b[i - 1] == b' ')
            && b[i + 1].is_ascii_digit()
            && (b[i + 2] == b' ' || b[i + 2] == b':')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_client_lines() {
        log("trace-test general line");
        log("trace-test client 1: slot 2 placed");
        log("trace-test client 2: slot 1 placed");
        log("trace-test c2 slot 3 (prio 1)");
        let c1: Vec<String> = last_lines(LOG_RING, Some(1))
            .into_iter()
            .filter(|l| l.contains("trace-test"))
            .collect();
        assert_eq!(c1.len(), 2, "{c1:?}");
        assert!(c1
            .iter()
            .all(|l| !l.contains("client 2") && !l.contains("c2 ")));
    }
}
