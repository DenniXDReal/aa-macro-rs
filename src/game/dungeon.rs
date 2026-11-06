//! Dungeon automation (skeleton - waiting on your screenshots of the
//! dungeon lobby, doors, shop and shrines).
//!
//! What's here is the decision side, tested on made-up data:
//! - which door to take, by the icon above it, in your room order
//!   (Item > Gold > Shrine > ... by default; a kind left off the list is
//!   never entered);
//! - what to buy in the dungeon shop, in your item order, while the money
//!   lasts;
//! - which shrines to switch on / off.
//!
//! The screen side (`RunCtx::dungeon_floor`) finds doors by your
//! `door_item` / `door_gold` / ... pictures and the shop / shrine by OCR;
//! it's wired into the step machine as the Dungeon step, but entering a
//! dungeon from the lobby isn't (no screenshots of it yet).

use crate::geom::Point;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoomKind {
    Item,
    Gold,
    Shrine,
    Shop,
    Combat,
    Boss,
}

impl RoomKind {
    pub const ALL: [RoomKind; 6] = [
        RoomKind::Item,
        RoomKind::Gold,
        RoomKind::Shrine,
        RoomKind::Shop,
        RoomKind::Combat,
        RoomKind::Boss,
    ];
    pub fn label(self) -> &'static str {
        match self {
            RoomKind::Item => "Item",
            RoomKind::Gold => "Gold",
            RoomKind::Shrine => "Shrine",
            RoomKind::Shop => "Shop",
            RoomKind::Combat => "Combat",
            RoomKind::Boss => "Boss",
        }
    }
    /// The picture of its door icon (capture with F6).
    pub fn door_pattern(self) -> &'static str {
        match self {
            RoomKind::Item => "door_item",
            RoomKind::Gold => "door_gold",
            RoomKind::Shrine => "door_shrine",
            RoomKind::Shop => "door_shop",
            RoomKind::Combat => "door_combat",
            RoomKind::Boss => "door_boss",
        }
    }
    pub fn parse(s: &str) -> Option<RoomKind> {
        let t = s.trim().to_lowercase();
        RoomKind::ALL
            .into_iter()
            .find(|k| k.label().to_lowercase() == t)
    }
}

/// A shrine rule: "Shrine of Greed" on / off.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShrineRule {
    pub name: String,
    pub on: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DungeonSettings {
    /// Doors in this order; a kind not listed is never entered.
    pub room_priority: Vec<RoomKind>,
    /// Shop items to buy, best first (names as the shop shows them).
    pub shop_priority: Vec<String>,
    pub shop_enabled: bool,
    /// Keep at least this much money when buying.
    pub shop_keep: u64,
    pub shrines: Vec<ShrineRule>,
}

impl Default for DungeonSettings {
    fn default() -> Self {
        Self {
            room_priority: vec![
                RoomKind::Item,
                RoomKind::Gold,
                RoomKind::Shrine,
                RoomKind::Shop,
                RoomKind::Combat,
                RoomKind::Boss,
            ],
            shop_priority: vec![],
            shop_enabled: true,
            shop_keep: 0,
            shrines: vec![],
        }
    }
}

/// Which door to take: the kind highest on `priority` (ties: the first
/// one seen). None = no door worth entering.
pub fn choose_door(doors: &[(RoomKind, Point)], priority: &[RoomKind]) -> Option<usize> {
    doors
        .iter()
        .enumerate()
        .filter_map(|(i, (k, _))| priority.iter().position(|p| p == k).map(|r| (r, i)))
        .min()
        .map(|(_, i)| i)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShopItem {
    pub name: String,
    pub price: u64,
    pub at: Point,
}

/// What to buy: items on your list, best first, while the money (above
/// `keep`) lasts; an item you can't afford is skipped, cheaper ones lower
/// on the list can still be bought.
pub fn choose_buys(items: &[ShopItem], priority: &[String], money: u64, keep: u64) -> Vec<usize> {
    let mut ranked: Vec<(usize, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| {
            priority
                .iter()
                .position(|p| p.eq_ignore_ascii_case(it.name.trim()))
                .map(|r| (r, i))
        })
        .collect();
    ranked.sort();
    let mut left = money.saturating_sub(keep);
    let mut out = vec![];
    for (_, i) in ranked {
        if items[i].price <= left {
            left -= items[i].price;
            out.push(i);
        }
    }
    out
}

/// Shop rows from OCR lines: "Lucky Coin 1,200" -> (name, price, where).
/// A line without a price (or without a name) isn't an item.
pub fn shop_items(lines: &[Vec<(String, crate::geom::Rect)>]) -> Vec<ShopItem> {
    let mut out = vec![];
    for l in lines {
        let words: Vec<&str> = l.iter().map(|(w, _)| w.as_str()).collect();
        let Some(pi) = words.iter().rposition(|w| {
            let d: String = w.chars().filter(|c| c.is_ascii_digit()).collect();
            !d.is_empty() && w.chars().all(|c| c.is_ascii_digit() || ",.$¥".contains(c))
        }) else {
            continue;
        };
        let price: u64 = words[pi]
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        let name = words[..pi].join(" ").trim().to_string();
        if name.is_empty() || price == 0 {
            continue;
        }
        let r = l[0].1;
        out.push(ShopItem {
            name,
            price,
            at: Point::new(r.x + r.w / 2, r.y + r.h / 2),
        });
    }
    out
}

/// A shrine `name` that's on (`is_on`): switch it? Some(true) = turn on,
/// Some(false) = turn off, None = leave it (no rule, or already right).
pub fn shrine_action(name: &str, is_on: bool, rules: &[ShrineRule]) -> Option<bool> {
    let want = rules
        .iter()
        .find(|r| r.name.trim().eq_ignore_ascii_case(name.trim()))?
        .on;
    (want != is_on).then_some(want)
}

/// The settings editor's text for shrine rules: "Shrine of Greed = on".
pub fn shrines_from_text(t: &str) -> Vec<ShrineRule> {
    t.lines()
        .filter_map(|l| {
            let (n, v) = l.split_once('=')?;
            let v = v.trim().to_lowercase();
            let on = matches!(v.as_str(), "on" | "yes" | "true" | "1");
            let off = matches!(v.as_str(), "off" | "no" | "false" | "0");
            (!n.trim().is_empty() && (on || off)).then(|| ShrineRule {
                name: n.trim().to_string(),
                on,
            })
        })
        .collect()
}

pub fn shrines_to_text(r: &[ShrineRule]) -> String {
    r.iter()
        .map(|s| format!("{} = {}", s.name, if s.on { "on" } else { "off" }))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Room order from the editor's text (one kind per line); unknown lines
/// are dropped.
pub fn rooms_from_text(t: &str) -> (Vec<RoomKind>, Vec<String>) {
    let mut out = vec![];
    let mut bad = vec![];
    for l in t.lines().map(str::trim).filter(|l| !l.is_empty()) {
        match RoomKind::parse(l) {
            Some(k) if !out.contains(&k) => out.push(k),
            Some(_) => {}
            None => bad.push(l.to_string()),
        }
    }
    (out, bad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;

    #[test]
    fn door_by_your_room_order() {
        let prio = DungeonSettings::default().room_priority;
        let doors = vec![
            (RoomKind::Combat, Point::new(100, 300)),
            (RoomKind::Gold, Point::new(400, 300)),
            (RoomKind::Shrine, Point::new(700, 300)),
        ];
        assert_eq!(
            choose_door(&doors, &prio),
            Some(1),
            "Gold beats Shrine, Combat"
        );
        let doors = vec![
            (RoomKind::Shrine, Point::new(100, 300)),
            (RoomKind::Item, Point::new(400, 300)),
        ];
        assert_eq!(choose_door(&doors, &prio), Some(1), "Item first");
        // Combat left off your list: never entered
        let only_shrine_gold = vec![RoomKind::Shrine, RoomKind::Gold];
        assert_eq!(
            choose_door(&[(RoomKind::Combat, Point::new(1, 1))], &only_shrine_gold),
            None
        );
    }

    #[test]
    fn shop_buys_in_order_while_money_lasts() {
        let it = |n: &str, p: u64| ShopItem {
            name: n.into(),
            price: p,
            at: Point::new(0, 0),
        };
        let items = vec![
            it("Potion", 300),
            it("Lucky Coin", 1200),
            it("Sword", 900),
            it("Junk", 10),
        ];
        let prio: Vec<String> = ["Lucky Coin", "Sword", "Potion"].map(String::from).to_vec();
        // 1500: Lucky Coin (1200), Sword too dear now, Potion (300)
        assert_eq!(choose_buys(&items, &prio, 1500, 0), vec![1, 0]);
        // keep 500: 1000 to spend - Lucky Coin too dear, Sword (900), 100 left
        assert_eq!(choose_buys(&items, &prio, 1500, 500), vec![2]);
        assert!(choose_buys(&items, &prio, 100, 0).is_empty());
    }

    #[test]
    fn shop_rows_from_ocr() {
        let w = |t: &str, x: i32, y: i32| (t.to_string(), Rect::new(x, y, 40, 14));
        let lines = vec![
            vec![w("DUNGEON", 300, 50), w("SHOP", 360, 50)],
            vec![
                w("Lucky", 200, 150),
                w("Coin", 245, 150),
                w("1,200", 400, 150),
            ],
            vec![w("Potion", 200, 190), w("300", 400, 190)],
        ];
        let items = shop_items(&lines);
        assert_eq!(items.len(), 2);
        assert_eq!(
            (items[0].name.as_str(), items[0].price),
            ("Lucky Coin", 1200)
        );
        assert_eq!((items[1].name.as_str(), items[1].price), ("Potion", 300));
    }

    #[test]
    fn shrines_switch_to_your_setting() {
        let rules = shrines_from_text("Shrine of Greed = on\nShrine of Pain = off\nbad line");
        assert_eq!(rules.len(), 2);
        assert_eq!(shrine_action("Shrine of Greed", false, &rules), Some(true));
        assert_eq!(
            shrine_action("shrine of greed", true, &rules),
            None,
            "already on"
        );
        assert_eq!(shrine_action("Shrine of Pain", true, &rules), Some(false));
        assert_eq!(
            shrine_action("Shrine of Luck", true, &rules),
            None,
            "no rule"
        );
        assert_eq!(shrines_from_text(&shrines_to_text(&rules)), rules);
        let (rooms, bad) = rooms_from_text("Item\ngold\nTreasure\nItem");
        assert_eq!(rooms, vec![RoomKind::Item, RoomKind::Gold]);
        assert_eq!(bad, vec!["Treasure"]);
    }
}
