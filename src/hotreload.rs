//! Hot-reload of patterns (`patterns/`) and your route files (the routes
//! folder next to the exe): edit / capture / delete one and running macros
//! pick it up within a second - no restart.
//!
//! - Watched with `notify`, debounced 500 ms (editors touch files several
//!   times per save).
//! - Swapped atomically (`ArcSwap`): a match already running finishes with
//!   the old set, the next one uses the new set. No lock on the hot path.
//! - Fail-soft: a file that doesn't load (bad JSON, missing picture) keeps
//!   the old set in use and is logged as a warning - you're likely mid-edit.
//! - Settings are NOT reloaded (restart needed), and neither is code.

use crate::movement::Routes;
use crate::vision::{Library, Pattern};
use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

pub const DEBOUNCE: Duration = Duration::from_millis(500);

pub type PatternMap = HashMap<String, Pattern>;

/// Patterns as one swappable set (one per running set of screens).
pub type LivePatterns = Arc<ArcSwap<PatternMap>>;

/// Routes as one swappable set.
#[derive(Clone)]
pub struct LiveRoutes(Arc<ArcSwap<Routes>>);

impl LiveRoutes {
    pub fn new(r: Routes) -> Self {
        let me = Self(Arc::new(ArcSwap::from_pointee(r)));
        ROUTE_SETS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::downgrade(&me.0));
        me
    }
    /// The current routes (a reload swaps in a new set for later calls).
    pub fn get(&self) -> Arc<Routes> {
        self.0.load_full()
    }
}

impl From<Routes> for LiveRoutes {
    fn from(r: Routes) -> Self {
        Self::new(r)
    }
}

static PATTERN_SETS: Mutex<Vec<Weak<ArcSwap<PatternMap>>>> = Mutex::new(Vec::new());
static ROUTE_SETS: Mutex<Vec<Weak<ArcSwap<Routes>>>> = Mutex::new(Vec::new());

/// A new swappable pattern set (kept up to date while it's in use).
pub fn live_patterns(m: PatternMap) -> LivePatterns {
    let a = Arc::new(ArcSwap::from_pointee(m));
    PATTERN_SETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Arc::downgrade(&a));
    a
}

pub fn pattern_map(lib: &Library) -> PatternMap {
    lib.patterns()
        .map(|p| (crate::vision::key(&p.name), p.clone()))
        .collect()
}

/// The patterns folder as a new set (Err = a file didn't load).
pub fn load_patterns(dir: &Path) -> anyhow::Result<Arc<PatternMap>> {
    let (lib, _) = Library::load_with_defaults(dir)?;
    Ok(Arc::new(pattern_map(&lib)))
}

/// Reload the patterns folder into every set in use. Err = kept the old.
pub fn reload_patterns(dir: &Path) -> anyhow::Result<usize> {
    let m = load_patterns(dir)?;
    let n = m.len();
    let mut sets = PATTERN_SETS.lock().unwrap_or_else(|e| e.into_inner());
    sets.retain(|w| w.strong_count() > 0);
    for s in sets.iter().filter_map(|w| w.upgrade()) {
        s.store(m.clone());
    }
    Ok(n)
}

/// Reload your route files into every set in use. Err = kept the old.
pub fn reload_routes(dir: &Path) -> anyhow::Result<usize> {
    let r = Arc::new(Routes::try_load(dir)?);
    let n = r.user_maps.len();
    let mut sets = ROUTE_SETS.lock().unwrap_or_else(|e| e.into_inner());
    sets.retain(|w| w.strong_count() > 0);
    for s in sets.iter().filter_map(|w| w.upgrade()) {
        s.store(r.clone());
    }
    Ok(n)
}

/// Watch both folders for the rest of the app's life (call once).
pub fn watch(patterns: PathBuf, routes: PathBuf) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("hot-reload".into())
            .spawn(move || run(patterns, routes));
    });
}

fn run(patterns: PathBuf, routes: PathBuf) {
    use notify::{RecursiveMode, Watcher};
    let (tx, rx) = std::sync::mpsc::channel::<PathBuf>();
    let mut w = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            for p in ev.paths {
                let _ = tx.send(p);
            }
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("hot-reload off: {e}");
            return;
        }
    };
    for d in [&patterns, &routes] {
        let _ = std::fs::create_dir_all(d);
        if let Err(e) = w.watch(d, RecursiveMode::Recursive) {
            log::warn!("hot-reload: can't watch {}: {e}", d.display());
        }
    }
    crate::game::trace::log(
        "hot-reload on for patterns and routes (settings still need a restart)",
    );
    let (mut pat_dirty, mut route_dirty): (Option<Instant>, Option<Instant>) = (None, None);
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(p) => {
                if p.starts_with(&patterns) {
                    pat_dirty = Some(Instant::now());
                } else if p.starts_with(&routes) {
                    route_dirty = Some(Instant::now());
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return,
        }
        if pat_dirty.is_some_and(|t| t.elapsed() >= DEBOUNCE) {
            pat_dirty = None;
            match reload_patterns(&patterns) {
                Ok(n) => say(&format!("patterns reloaded ({n})")),
                Err(e) => warn(&format!(
                    "patterns not reloaded, keeping the old ones: {e:#}"
                )),
            }
        }
        if route_dirty.is_some_and(|t| t.elapsed() >= DEBOUNCE) {
            route_dirty = None;
            match reload_routes(&routes) {
                Ok(n) => say(&format!("routes reloaded ({n} of yours)")),
                Err(e) => warn(&format!("routes not reloaded, keeping the old ones: {e:#}")),
            }
        }
    }
}

fn say(s: &str) {
    log::info!("{s}");
    crate::game::trace::log(s);
}

fn warn(s: &str) {
    log::warn!("{s}");
    crate::game::trace::log(&format!("WARN {s}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    // (only the loading half is tested: broadcasting into every set in use
    // would swap the patterns of other tests running at the same time)
    #[test]
    fn swaps_in_place_and_keeps_old_on_error() {
        let dir = std::env::temp_dir().join(format!("aa-hot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let live: LivePatterns = Arc::new(ArcSwap::from_pointee(PatternMap::new()));
        let held = live.load_full(); // a match in flight keeps this one
        let m = load_patterns(&dir).unwrap();
        let n = m.len();
        live.store(m);
        assert!(held.is_empty(), "in-flight set unchanged");
        assert_eq!(live.load().len(), n, "new matches see the reloaded set");
        // a broken file: error, nothing to swap in - the old set stays
        std::fs::write(dir.join("broken.json"), "{ not json").unwrap();
        assert!(load_patterns(&dir).is_err());
        assert_eq!(live.load().len(), n);
        // routes: a broken route file of yours is an error too
        let rdir = dir.join("routes");
        std::fs::create_dir_all(&rdir).unwrap();
        assert!(Routes::try_load(&rdir).is_ok());
        std::fs::write(rdir.join("Broken Map.json"), "nope").unwrap();
        assert!(Routes::try_load(&rdir).is_err());
        assert!(
            !Routes::load(&rdir).maps.contains_key("Broken Map"),
            "plain load skips it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
