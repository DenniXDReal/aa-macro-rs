//! Spot memory: spots where a unit didn't go down (twice) are skipped next
//! time on that map, so placing doesn't waste time on walls / the path.
//! A success at a spot clears it again. Saved next to the exe.

use crate::geom::Point;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Fails at a spot before it's skipped (one fail can just be no money yet).
pub const BAD_AFTER: u32 = 2;
/// Spots this close count as the same spot.
const NEAR: i32 = 10;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SpotMemory {
    /// spot set key ("Map @1") -> "x,y" -> fails
    pub maps: BTreeMap<String, BTreeMap<String, u32>>,
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

fn k(p: Point) -> String {
    format!("{},{}", p.x, p.y)
}

fn parse(s: &str) -> Option<Point> {
    let (a, b) = s.split_once(',')?;
    Some(Point::new(a.trim().parse().ok()?, b.trim().parse().ok()?))
}

impl SpotMemory {
    pub fn default_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("spot-memory.json")))
            .unwrap_or_else(|| PathBuf::from("spot-memory.json"))
    }

    pub fn load(path: &Path) -> Self {
        let mut m: SpotMemory = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        m.path = Some(path.to_path_buf());
        m
    }

    pub fn save(&self) -> Result<()> {
        if let Some(p) = &self.path {
            std::fs::write(p, serde_json::to_string_pretty(self)?)?;
        }
        Ok(())
    }

    fn entry(&self, set: &str, p: Point) -> Option<(String, u32)> {
        self.maps.get(set)?.iter().find_map(|(s, n)| {
            let q = parse(s)?;
            ((q.x - p.x).abs() <= NEAR && (q.y - p.y).abs() <= NEAR).then(|| (s.clone(), *n))
        })
    }

    /// Skip this spot?
    pub fn is_bad(&self, set: &str, p: Point) -> bool {
        self.entry(set, p).is_some_and(|(_, n)| n >= BAD_AFTER)
    }

    /// A unit didn't go down here.
    pub fn failed(&mut self, set: &str, p: Point) {
        let key = self.entry(set, p).map(|e| e.0).unwrap_or_else(|| k(p));
        *self
            .maps
            .entry(set.to_string())
            .or_default()
            .entry(key)
            .or_insert(0) += 1;
        let _ = self.save();
    }

    /// A unit went down here: it's a good spot.
    pub fn placed(&mut self, set: &str, p: Point) {
        if let Some((key, _)) = self.entry(set, p) {
            if let Some(m) = self.maps.get_mut(set) {
                m.remove(&key);
            }
            let _ = self.save();
        }
    }

    pub fn bad_count(&self, set: &str) -> usize {
        self.maps
            .get(set)
            .map(|m| m.values().filter(|n| **n >= BAD_AFTER).count())
            .unwrap_or(0)
    }

    pub fn bad_spots(&self, set: &str) -> Vec<Point> {
        self.maps
            .get(set)
            .map(|m| {
                m.iter()
                    .filter(|(_, n)| **n >= BAD_AFTER)
                    .filter_map(|(s, _)| parse(s))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn forget(&mut self, set: &str) {
        self.maps.remove(set);
        let _ = self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_fails_skip_success_clears() {
        let mut m = SpotMemory::default();
        let p = Point::new(300, 200);
        m.failed("Map @1", p);
        assert!(!m.is_bad("Map @1", p), "one fail may just be no money");
        m.failed("Map @1", Point::new(305, 198)); // same spot, a bit off
        assert!(m.is_bad("Map @1", p));
        assert!(!m.is_bad("Map @2", p), "per map + client");
        assert_eq!(m.bad_count("Map @1"), 1);
        m.placed("Map @1", p);
        assert!(!m.is_bad("Map @1", p));
    }
}
