//! Multi-Roblox without RAM: the app holds Roblox's single-instance lock
//! (`ROBLOX_singletonMutex`) so every client that starts afterwards can stay
//! open. Roblox clients that were already open before the lock was taken
//! fight over it, so they are closed first.

use crate::platform::Platform;
use anyhow::Result;
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Held for as long as the app runs; dropping it lets go of the lock.
pub struct MultiLock {
    #[cfg(windows)]
    _held: Vec<crate::platform::win32::SingletonHandle>,
    /// The lock already existed when we took it (RAM's Multi Roblox, say).
    pub shared: bool,
}

impl MultiLock {
    #[cfg(windows)]
    pub fn acquire(names: &[String]) -> Result<Self> {
        let (held, shared) = crate::platform::win32::hold_singleton(names)?;
        Ok(Self {
            _held: held,
            shared,
        })
    }

    #[cfg(not(windows))]
    pub fn acquire(_names: &[String]) -> Result<Self> {
        anyhow::bail!("multi-Roblox needs Windows")
    }
}

/// Before opening `adding` more clients: if that makes more than one and we
/// don't hold the lock yet, close every open Roblox, wait for them to go,
/// then take the lock. `Ok(None)` = nothing to do.
pub fn ensure(
    platform: &dyn Platform,
    process: &str,
    names: &[String],
    have_lock: bool,
    adding: usize,
    log: &dyn Fn(String),
) -> Result<Option<MultiLock>> {
    let open = platform.list_windows(process).len();
    if have_lock || open + adding <= 1 {
        return Ok(None);
    }
    if open > 0 {
        log(format!(
            "multi-Roblox: closing {open} open Roblox client(s) first so several can run"
        ));
        for _ in 0..2 {
            platform.kill_process(process)?;
            let t = Instant::now();
            while !platform.list_windows(process).is_empty()
                && t.elapsed() < Duration::from_secs(15)
            {
                std::thread::sleep(Duration::from_millis(300));
            }
            if platform.list_windows(process).is_empty() {
                break;
            }
        }
        // let the old processes release the lock completely
        std::thread::sleep(Duration::from_millis(1500));
    }
    let lock = MultiLock::acquire(names)?;
    log(if lock.shared {
        "multi-Roblox on (the lock was already there - RAM's Multi Roblox is fine too)".into()
    } else {
        "multi-Roblox on".into()
    });
    Ok(Some(lock))
}

/// Launch accounts one after another, waiting for each new Roblox window so
/// we know which pid is which username. `pids` lists the Roblox pids that
/// have a window right now; `found(pid, name)` is told each match.
pub fn launch_each(
    names: &[String],
    pids: &dyn Fn() -> Vec<u32>,
    launch: &mut dyn FnMut(&str) -> Result<String>,
    found: &mut dyn FnMut(u32, &str),
    log: &dyn Fn(String),
    wait: Duration,
    poll: Duration,
) {
    let mut taken: HashSet<u32> = HashSet::new();
    for n in names {
        let before: HashSet<u32> = pids().into_iter().collect();
        match launch(n) {
            Ok(m) => log(format!("{n}: {m}")),
            Err(e) => {
                log(format!("{n}: launch failed - {e}"));
                continue;
            }
        }
        let t = Instant::now();
        let new = loop {
            if let Some(p) = pids()
                .into_iter()
                .find(|p| !before.contains(p) && !taken.contains(p))
            {
                break Some(p);
            }
            if t.elapsed() >= wait {
                break None;
            }
            std::thread::sleep(poll);
        };
        match new {
            Some(p) => {
                taken.insert(p);
                found(p, n);
                log(format!("{n}: Roblox window is up"));
            }
            None => log(format!(
                "{n}: no new Roblox window after {}s - check RAM / Roblox",
                wait.as_secs()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn each_launch_gets_its_own_pid() {
        let open = RefCell::new(vec![100u32]);
        let next = RefCell::new(200u32);
        let mut got = vec![];
        let logs = RefCell::new(vec![]);
        launch_each(
            &["Main".into(), "AltA".into(), "AltB".into()],
            &|| open.borrow().clone(),
            &mut |n| {
                if n == "AltA" {
                    anyhow::bail!("RAM said no");
                }
                let mut nx = next.borrow_mut();
                open.borrow_mut().push(*nx);
                *nx += 1;
                Ok("launched".into())
            },
            &mut |p, n| got.push((p, n.to_string())),
            &|s| logs.borrow_mut().push(s),
            Duration::from_millis(50),
            Duration::from_millis(5),
        );
        assert_eq!(got, vec![(200, "Main".into()), (201, "AltB".into())]);
        assert!(logs
            .borrow()
            .iter()
            .any(|l| l.contains("AltA: launch failed")));
    }

    #[test]
    fn launch_without_window_times_out() {
        let logs = RefCell::new(vec![]);
        let mut got = vec![];
        launch_each(
            &["Main".into()],
            &|| vec![1],
            &mut |_| Ok("ok".into()),
            &mut |p, n| got.push((p, n.to_string())),
            &|s| logs.borrow_mut().push(s),
            Duration::from_millis(20),
            Duration::from_millis(5),
        );
        assert!(got.is_empty());
        assert!(logs
            .borrow()
            .iter()
            .any(|l| l.contains("no new Roblox window")));
    }
}
