//! Per-map unit placement spots (`placements.json` next to the exe).
//!
//! Spots are client coordinates (800x599). Units fill spots in order; a spot
//! can be reserved for one hotbar slot (1-6) or left open to any (0).

use crate::geom::Point;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spot {
    pub x: i32,
    pub y: i32,
    /// 0 = any unit, 1-6 = only that hotbar slot.
    #[serde(default)]
    pub slot: u8,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Placements {
    pub maps: BTreeMap<String, Vec<Spot>>,
}

impl Placements {
    pub fn default_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("placements.json")))
            .unwrap_or_else(|| PathBuf::from("placements.json"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let t = std::fs::read_to_string(path)?;
        serde_json::from_str(&t).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn spots(&self, map: &str) -> &[Spot] {
        self.maps.get(map).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn add(&mut self, map: &str, p: Point, slot: u8) {
        self.maps.entry(map.to_string()).or_default().push(Spot {
            x: p.x,
            y: p.y,
            slot,
        });
    }

    /// Remove the spot nearest to `p` within `radius` px. Returns true if one went.
    pub fn remove_near(&mut self, map: &str, p: Point, radius: i32) -> bool {
        let Some(v) = self.maps.get_mut(map) else {
            return false;
        };
        let best = v
            .iter()
            .enumerate()
            .map(|(i, s)| (i, (s.x - p.x).pow(2) + (s.y - p.y).pow(2)))
            .filter(|(_, d)| *d <= radius * radius)
            .min_by_key(|(_, d)| *d);
        match best {
            Some((i, _)) => {
                v.remove(i);
                true
            }
            None => false,
        }
    }

    pub fn undo(&mut self, map: &str) {
        if let Some(v) = self.maps.get_mut(map) {
            v.pop();
        }
    }

    pub fn clear(&mut self, map: &str) {
        self.maps.remove(map);
    }

    /// Spots usable by `slot`: reserved ones first, then open ones, in order.
    pub fn for_slot(&self, map: &str, slot: u8) -> Vec<Spot> {
        let s = self.spots(map);
        s.iter()
            .filter(|x| x.slot == slot)
            .chain(s.iter().filter(|x| x.slot == 0))
            .copied()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_remove_undo() {
        let mut p = Placements::default();
        p.add("Planet Greenie", Point::new(100, 100), 0);
        p.add("Planet Greenie", Point::new(200, 100), 2);
        p.add("Planet Greenie", Point::new(300, 100), 0);
        assert!(!p.remove_near("Planet Greenie", Point::new(150, 100), 10));
        assert!(p.remove_near("Planet Greenie", Point::new(205, 104), 10));
        assert_eq!(p.spots("Planet Greenie").len(), 2);
        p.undo("Planet Greenie");
        assert_eq!(
            p.spots("Planet Greenie"),
            &[Spot {
                x: 100,
                y: 100,
                slot: 0
            }]
        );
        assert!(p.spots("Nope").is_empty());
    }

    #[test]
    fn reserved_spots_first() {
        let mut p = Placements::default();
        p.add("m", Point::new(1, 1), 0);
        p.add("m", Point::new(2, 2), 3);
        p.add("m", Point::new(3, 3), 1);
        let s: Vec<i32> = p.for_slot("m", 3).iter().map(|s| s.x).collect();
        assert_eq!(s, vec![2, 1]);
    }

    #[test]
    fn save_load() {
        let path = std::env::temp_dir().join(format!("aa-place-{}.json", std::process::id()));
        let mut p = Placements::default();
        p.add("m", Point::new(5, 6), 1);
        p.save(&path).unwrap();
        let q = Placements::load(&path).unwrap();
        assert_eq!(q.spots("m"), p.spots("m"));
        std::fs::remove_file(path).ok();
    }
}
