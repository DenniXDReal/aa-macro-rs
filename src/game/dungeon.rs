//! Dungeon automation: rooms on the path picker read by their icons
//! (built-in pictures of Hoard, Chest, Rare Chest, Shop, Shrine and Curse,
//! from your screenshots) and taken in your room order; what to buy in the
//! dungeon shop; which shrines to switch on / off.
//!
//! The screen side lives in `RunCtx`: `enter_dungeon` (Dungeon button >
//! Enter Dungeon), `dungeon_floor` (cards, shop, shrines, the path), and the
//! end chests + loot report in `end_rewards`.

use crate::geom::Point;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoomKind {
    /// Gold coin ("HOARD").
    #[serde(alias = "Gold")]
    Hoard,
    /// Normal chest.
    #[serde(alias = "Item")]
    Chest,
    /// Rare (red / gold) chest.
    RareChest,
    Shop,
    Shrine,
    /// Purple curse mark: a room with one is ranked no better than the
    /// curse's own place on your list.
    Curse,
    Combat,
    Boss,
}

impl RoomKind {
    /// Default order: Spiral's (Rare Chest, Chest, Hoard, Shrine), then
    /// the rest.
    pub const ALL: [RoomKind; 8] = [
        RoomKind::RareChest,
        RoomKind::Chest,
        RoomKind::Hoard,
        RoomKind::Shrine,
        RoomKind::Shop,
        RoomKind::Combat,
        RoomKind::Boss,
        RoomKind::Curse,
    ];
    pub fn label(self) -> &'static str {
        match self {
            RoomKind::Hoard => "Hoard",
            RoomKind::Chest => "Chest",
            RoomKind::RareChest => "Rare Chest",
            RoomKind::Shop => "Shop",
            RoomKind::Shrine => "Shrine",
            RoomKind::Curse => "Curse",
            RoomKind::Combat => "Combat",
            RoomKind::Boss => "Boss",
        }
    }
    /// Your own picture of its icon (optional, capture with F6) - found
    /// besides the built-in one.
    pub fn door_pattern(self) -> &'static str {
        match self {
            RoomKind::Hoard => "room_hoard",
            RoomKind::Chest => "room_chest",
            RoomKind::RareChest => "room_rare_chest",
            RoomKind::Shop => "room_shop",
            RoomKind::Shrine => "room_shrine",
            RoomKind::Curse => "room_curse",
            RoomKind::Combat => "room_combat",
            RoomKind::Boss => "room_boss",
        }
    }
    /// Built-in icons of this kind.
    pub fn icons(self) -> &'static [&'static str] {
        match self {
            RoomKind::Hoard => &["icon_hoard", "icon_hoard_big"],
            RoomKind::Chest => &["icon_chest"],
            RoomKind::RareChest => &["icon_rare_chest"],
            RoomKind::Shop => &["icon_shop"],
            RoomKind::Shrine => &["icon_shrine"],
            RoomKind::Curse => &["icon_curse"],
            RoomKind::Combat | RoomKind::Boss => &[],
        }
    }
    pub fn parse(s: &str) -> Option<RoomKind> {
        let t: String = s
            .trim()
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        let alias = match t.as_str() {
            "gold" => Some(RoomKind::Hoard),
            "item" | "normalchest" => Some(RoomKind::Chest),
            "rare" => Some(RoomKind::RareChest),
            _ => None,
        };
        alias.or_else(|| {
            RoomKind::ALL.into_iter().find(|k| {
                k.label()
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>()
                    == t
            })
        })
    }
}

/// The curses a room can carry ("Curse of Health", ...) - as in Spiral.
pub const CURSES: [&str; 6] = [
    "Flying",
    "Money",
    "Health",
    "Exploding",
    "Shielding",
    "Regen",
];
/// The shrines - as in Spiral.
pub const SHRINES: [&str; 5] = ["Burst", "Armor", "Chaos", "Vamparism", "Frenzy"];
/// Every shop item (Spiral's list, in its order) with what it does.
pub const SHOP_ITEMS: &[(&str, &str)] = &[
    (
        "Philosopher's Stone",
        "Instantly converts all chests to Rare",
    ),
    ("Genie Lamp", "+100% Chest Luck"),
    ("Golden Fishhook", "+40% Chest Luck"),
    ("Shimmering Moon Fragment", "+1 Star Remnant per chest"),
    ("Fishhook", "+20% Chest Luck"),
    ("Mask of Riches", "-50% Upgrade & Placement costs"),
    ("Golden Mushroom", "-30% Upgrade & Placement costs"),
    ("Golden Branch", "-15% Upgrade & Placement costs"),
    ("Archmage's Hat", "+250% Magic Damage"),
    ("Magic Tome", "+100% Magic Damage"),
    ("Scimitar", "+75% Attack"),
    ("Glacial Axe", "Enemies move 30% slower"),
    ("Ice Dagger", ""),
    ("Sylvanthorn", "+150% Range"),
    ("Stormbringer", ""),
    ("Mage Coat", "+50% Magic Damage"),
    ("Sword", "+30% Damage"),
    ("Elf Bow", "+50% Range"),
    ("Longbow", "+30% Range"),
    ("Reaper Scythe", "+75% Boss Damage"),
    ("Speedy Boots", ""),
    ("Dagger", "+15% Attack"),
    ("Ninja Belt", "-12% Attack Speed"),
    ("Nimble Cloak", ""),
    ("Aureus Maul", "+200% Physical Damage"),
    ("Headsplitter", "+75% Physical Damage"),
    ("Solar Cleaver", "+100% Physical Crit Chance"),
    ("Celestial Blade", "+777% True Damage"),
];
/// Bought by default: the loot items (chest luck, Star Remnants), then
/// the cost cutters.
pub const SHOP_DEFAULT: [&str; 8] = [
    "Philosopher's Stone",
    "Genie Lamp",
    "Shimmering Moon Fragment",
    "Golden Fishhook",
    "Fishhook",
    "Mask of Riches",
    "Golden Mushroom",
    "Golden Branch",
];

/// Shop item named in an OCR line: the longest known name in it (so
/// "Golden Fishhook" isn't read as "Fishhook").
pub fn shop_item_in(line: &str) -> Option<&'static str> {
    let low = line.to_lowercase();
    SHOP_ITEMS
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| low.contains(&n.to_lowercase()))
        .max_by_key(|n| n.len())
}

/// The curses named on screen ("Curse of Shielding" -> "Shielding").
pub fn curses_in(text: &str) -> Vec<&'static str> {
    let low = text.to_lowercase();
    CURSES
        .iter()
        .filter(|c| low.contains(&format!("curse of {}", c.to_lowercase())))
        .copied()
        .collect()
}

/// The shrine named on screen (Burst, Armor, ...), if one of the known.
pub fn shrine_in(text: &str) -> Option<&'static str> {
    let low = text.to_lowercase();
    SHRINES
        .iter()
        .find(|s| low.contains(&s.to_lowercase()))
        .copied()
}

/// Accept this shrine? Your rule for it, else accept unless `deny_all`.
pub fn accept_shrine(name: &str, deny_all: bool, rules: &[ShrineRule]) -> bool {
    rules
        .iter()
        .find(|r| {
            let r = r.name.to_lowercase();
            let n = name.to_lowercase();
            r == n || r.contains(&n) || n.contains(&r)
        })
        .map(|r| r.on)
        .unwrap_or(!deny_all)
}

/// Any of `curses` on your avoid list?
pub fn avoided(curses: &[&str], avoid: &[String]) -> usize {
    curses
        .iter()
        .filter(|c| avoid.iter().any(|a| a.trim().eq_ignore_ascii_case(c)))
        .count()
}

/// One room on the dungeon path picker: what's on it, where to click.
#[derive(Clone, Debug, PartialEq)]
pub struct Room {
    pub kinds: Vec<RoomKind>,
    pub at: Point,
}

/// Icons within this many px of each other are on the same room tile.
pub const SAME_ROOM_PX: i32 = 24;

/// Group icon hits into rooms (icons close together share a tile).
pub fn group_rooms(hits: &[(RoomKind, Point)]) -> Vec<Room> {
    let n = hits.len();
    let mut root: Vec<usize> = (0..n).collect();
    fn find(r: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while r[i] != i {
            r[i] = r[r[i]];
            i = r[i];
        }
        i
    }
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (hits[i].1, hits[j].1);
            if (a.x - b.x).abs() <= SAME_ROOM_PX && (a.y - b.y).abs() <= SAME_ROOM_PX {
                let (ri, rj) = (find(&mut root, i), find(&mut root, j));
                root[ri] = rj;
            }
        }
    }
    let mut groups: Vec<(usize, Vec<usize>)> = vec![];
    for i in 0..n {
        let r = find(&mut root, i);
        match groups.iter_mut().find(|g| g.0 == r) {
            Some(g) => g.1.push(i),
            None => groups.push((r, vec![i])),
        }
    }
    groups
        .into_iter()
        .map(|(_, idx)| {
            let k = idx.len() as i32;
            let mut kinds: Vec<RoomKind> = vec![];
            for i in &idx {
                if !kinds.contains(&hits[*i].0) {
                    kinds.push(hits[*i].0);
                }
            }
            Room {
                kinds,
                at: Point::new(
                    idx.iter().map(|i| hits[*i].1.x).sum::<i32>() / k,
                    idx.iter().map(|i| hits[*i].1.y).sum::<i32>() / k,
                ),
            }
        })
        .collect()
}

/// Every room icon on screen (built-in icons).
pub fn icon_hits(f: &crate::capture::Frame) -> Vec<(RoomKind, Point)> {
    let all = crate::geom::Rect::new(0, 0, f.width as i32, f.height as i32);
    let mut out = vec![];
    for k in RoomKind::ALL {
        for name in k.icons() {
            if let Some(icon) = super::icons::get(name) {
                for (p, _) in icon.find_all(f, all, ICON_TOL, ICON_MATCH) {
                    out.push((k, p));
                }
            }
        }
    }
    out
}

/// Room icons: per-channel tolerance and share of pixels that must match.
pub const ICON_TOL: u8 = 40;
pub const ICON_MATCH: f32 = 0.8;

/// A room's place on your list: its best kind, but never better than a
/// curse on it. None = nothing on it is on your list.
pub fn room_rank(r: &Room, priority: &[RoomKind]) -> Option<usize> {
    let pos = |k: &RoomKind| priority.iter().position(|p| p == k);
    let best = r
        .kinds
        .iter()
        .filter(|k| **k != RoomKind::Curse)
        .filter_map(pos)
        .min();
    let curse = r.kinds.contains(&RoomKind::Curse);
    match (best, curse) {
        (b, false) => b,
        (b, true) => {
            let c = pos(&RoomKind::Curse)?;
            Some(b.map_or(c, |b| b.max(c)))
        }
    }
}

/// The room to take: highest on your list (ties: the first seen).
pub fn choose_room(rooms: &[Room], priority: &[RoomKind]) -> Option<usize> {
    rooms
        .iter()
        .enumerate()
        .filter_map(|(i, r)| room_rank(r, priority).map(|k| (k, i)))
        .min()
        .map(|(_, i)| i)
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
    /// Per shrine: accept (on) / deny (off); a shrine not listed is
    /// accepted unless `deny_shrines`.
    pub shrines: Vec<ShrineRule>,
    /// Deny every shrine not listed as on.
    pub deny_shrines: bool,
    /// Curses ("Health", "Shielding", ...): a room with one of these is
    /// only entered when every room has one.
    pub avoid_curses: Vec<String>,
    /// Card picks in a dungeon, best first (empty = the normal card list).
    pub card_priority: Vec<String>,
    /// After a dungeon: open the end chests and report the loot.
    pub open_chests: bool,
}

impl Default for DungeonSettings {
    fn default() -> Self {
        Self {
            room_priority: RoomKind::ALL.to_vec(),
            shop_priority: SHOP_DEFAULT.iter().map(|s| s.to_string()).collect(),
            shop_enabled: true,
            shop_keep: 0,
            shrines: vec![],
            deny_shrines: false,
            avoid_curses: vec![],
            card_priority: vec![],
            open_chests: true,
        }
    }
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
        let said = words[..pi].join(" ");
        let name = shop_item_in(&said)
            .map(str::to_string)
            .unwrap_or_else(|| said.trim().to_string());
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

    /// Your room tiles pasted onto a dark screen, as on the path picker.
    fn path_screen() -> crate::capture::Frame {
        use crate::capture::Frame;
        let load = |n: &str| {
            Frame::load_png(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(n),
            )
            .unwrap()
        };
        let mut px = vec![0u8; 800 * 600 * 4];
        for p in px.chunks_exact_mut(4) {
            p.copy_from_slice(&[40, 24, 22, 255]); // bgra, the dark backdrop
        }
        let mut f = Frame::new(800, 600, px);
        for (name, x, y) in [
            ("room_hoard_chest.png", 200, 250),
            ("room_shop_shrine.png", 380, 250),
            ("room_curse_rare.png", 560, 250),
            ("room_hoard.png", 380, 400),
        ] {
            let t = load(name);
            for ty in 0..t.height {
                for tx in 0..t.width {
                    let si = ((ty * t.width + tx) * 4) as usize;
                    let di = (((y + ty) * 800 + x + tx) * 4) as usize;
                    f.bgra[di..di + 4].copy_from_slice(&t.bgra[si..si + 4]);
                }
            }
        }
        f
    }

    #[test]
    fn rooms_from_your_tiles() {
        let mut rooms = group_rooms(&icon_hits(&path_screen()));
        rooms.sort_by_key(|r| (r.at.y / 100, r.at.x));
        let kinds: Vec<Vec<RoomKind>> = rooms
            .iter()
            .map(|r| {
                let mut k = r.kinds.clone();
                k.sort_by_key(|k| k.label());
                k
            })
            .collect();
        use RoomKind::*;
        assert_eq!(
            kinds,
            vec![
                vec![Chest, Hoard],
                vec![Shop, Shrine],
                vec![Curse, RareChest, Shop],
                vec![Hoard],
            ],
            "{rooms:?}"
        );
        // each room's click point is on its tile
        assert!(rooms[0].at.x > 200 && rooms[0].at.x < 258);
        assert!(rooms[2].at.x > 560 && rooms[2].at.x < 618);
    }

    #[test]
    fn room_by_your_order() {
        use RoomKind::*;
        let prio = DungeonSettings::default().room_priority;
        let r = |kinds: Vec<RoomKind>, x| Room {
            kinds,
            at: Point::new(x, 300),
        };
        let rooms = vec![
            r(vec![Shop, Shrine], 100),
            r(vec![Curse, RareChest, Shop], 300),
            r(vec![Hoard, Chest], 500),
        ];
        // the rare chest is cursed: ranked as the curse (last) - Hoard wins
        assert_eq!(choose_room(&rooms, &prio), Some(2));
        // curses wanted (top of the list): the cursed room it is
        let mut want = prio.clone();
        want.retain(|k| *k != Curse);
        want.insert(0, Curse);
        assert_eq!(choose_room(&rooms, &want), Some(1));
        // only Shrine on the list: just that room
        assert_eq!(choose_room(&rooms, &[Shrine]), Some(0));
        assert_eq!(choose_room(&rooms, &[Boss]), None);
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
        let (rooms, bad) = rooms_from_text("Rare Chest\ngold\nTreasure\nrare chest");
        assert_eq!(rooms, vec![RoomKind::RareChest, RoomKind::Hoard]);
        assert_eq!(bad, vec!["Treasure"]);
    }

    #[test]
    fn spiral_lists() {
        assert_eq!(
            shop_item_in("Golden Fishhook +40% Chest Luck"),
            Some("Golden Fishhook")
        );
        assert_eq!(shop_item_in("Fishhook 300"), Some("Fishhook"));
        assert_eq!(
            curses_in("Curse of Shielding  Curse of Health"),
            vec!["Health", "Shielding"]
        );
        assert_eq!(avoided(&["Health", "Shielding"], &["shielding".into()]), 1);
        assert_eq!(shrine_in("Shrine of Frenzy  Accept"), Some("Frenzy"));
        let rules = shrines_from_text("Frenzy = off");
        assert!(!accept_shrine("Frenzy", false, &rules));
        assert!(accept_shrine("Burst", false, &rules));
        assert!(!accept_shrine("Burst", true, &rules));
        assert_eq!(
            DungeonSettings::default().room_priority[..4],
            [
                RoomKind::RareChest,
                RoomKind::Chest,
                RoomKind::Hoard,
                RoomKind::Shrine
            ]
        );
        assert!(SHOP_DEFAULT
            .iter()
            .all(|d| SHOP_ITEMS.iter().any(|i| i.0 == *d)));
    }
}
