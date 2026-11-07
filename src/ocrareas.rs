//! Where on the client each fixed read happens (OCR boxes, the minimap
//! crop). Every area has a built-in default; Placement > "OCR areas" lets
//! you drag one to where it is on your screen - those overrides live in
//! Settings (`ocr_areas`) and are put here when the run starts / you edit.

use crate::geom::Rect;
use std::collections::BTreeMap;
use std::sync::RwLock;

/// One area: its key in Settings, what it is, its default.
pub struct Area {
    pub key: &'static str,
    pub label: &'static str,
    pub default: Rect,
}

/// Every area you can move.
pub fn all() -> Vec<Area> {
    use crate::game::{cards, loot, minimap};
    vec![
        Area {
            key: "pearls_box",
            label: "Pearls amount on the end panel (when the icon isn't found)",
            default: loot::PEARL_RECT,
        },
        Area {
            key: "pearls_search",
            label: "Where the pearl icon is looked for on the end panel",
            default: loot::PEARL_SEARCH,
        },
        Area {
            key: "pearls_hud",
            label: "Pearls counter, top left",
            default: loot::PEARL_HUD,
        },
        Area {
            key: "reward_area",
            label: "Rewards (charm search)",
            default: loot::REWARD_AREA,
        },
        Area {
            key: "card_1",
            label: "Card 1 title",
            default: cards::title_box(0),
        },
        Area {
            key: "card_2",
            label: "Card 2 title",
            default: cards::title_box(1),
        },
        Area {
            key: "card_3",
            label: "Card 3 title",
            default: cards::title_box(2),
        },
        Area {
            key: "end_title",
            label: "VICTORY / DEFEAT text",
            default: Rect::new(0, 0, 800, 360),
        },
        Area {
            key: "dungeon_money",
            label: "Dungeon shop money",
            default: Rect::new(300, 470, 200, 30),
        },
        Area {
            key: "minimap",
            label: "Minimap (bottom right)",
            default: minimap::MINIMAP,
        },
    ]
}

static OVERRIDES: RwLock<BTreeMap<String, Rect>> = RwLock::new(BTreeMap::new());

/// Put your saved areas in place (Settings' `ocr_areas`).
pub fn set_all(saved: &BTreeMap<String, [i32; 4]>) {
    let m = saved
        .iter()
        .filter(|(_, r)| r[2] > 0 && r[3] > 0)
        .map(|(k, r)| (k.clone(), Rect::new(r[0], r[1], r[2], r[3])))
        .collect();
    *OVERRIDES.write().unwrap_or_else(|e| e.into_inner()) = m;
}

/// The area for `key`: yours if you moved it, else `default`.
pub fn get(key: &str, default: Rect) -> Rect {
    OVERRIDES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .copied()
        .unwrap_or(default)
}

/// The area for `key` by its built-in default (unknown key: an empty rect).
pub fn area(key: &str) -> Rect {
    let d = all()
        .into_iter()
        .find(|a| a.key == key)
        .map(|a| a.default)
        .unwrap_or_default();
    get(key, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        assert_eq!(area("pearls_hud"), crate::game::loot::PEARL_HUD);
        let mut m = BTreeMap::new();
        m.insert("dungeon_money".to_string(), [10, 20, 30, 40]);
        m.insert("card_1".to_string(), [1, 1, 0, 5]); // empty: ignored
        set_all(&m);
        assert_eq!(area("dungeon_money"), Rect::new(10, 20, 30, 40));
        assert_eq!(area("card_1"), crate::game::cards::title_box(0));
        set_all(&BTreeMap::new());
        assert_eq!(area("dungeon_money"), Rect::new(300, 470, 200, 30));
        // every key is unique
        let keys: std::collections::HashSet<_> = all().iter().map(|a| a.key).collect();
        assert_eq!(keys.len(), all().len());
    }
}
