//! Settings file (`aa-macro.json` next to the exe). Missing keys get defaults.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Roblox process to look for.
    pub process_name: String,
    /// Client size every pattern / spot is captured for.
    pub client_w: i32,
    pub client_h: i32,
    /// Max instances driven at once.
    pub max_instances: usize,
    /// Arbiter: warn when one input batch holds focus longer than this.
    pub max_lease_ms: u64,
    /// Arbiter: give up when focus can't be taken within this.
    pub focus_timeout_ms: u64,
    /// Delay after a cursor move before clicking (Roblox hover needs it).
    pub mouse_settle_ms: u64,
    /// How long a click holds the button.
    pub click_hold_ms: u64,
    /// Pause after every click the game loop / routes make (menus, settings).
    pub step_delay_ms: u64,
    /// Pause after switching to another account before clicking there.
    pub swap_delay_ms: u64,
    /// Pause between unit placements so the game can register each one.
    pub place_delay_ms: u64,
    /// Pause between each step of a placement: select > click > check >
    /// swap to the next account (ms). (key kept for old saves)
    #[serde(rename = "place_step_ms")]
    pub place_settle_ms: u64,
    /// How long the placed check (unit_placed / "UPG [0]" OCR) may look (ms).
    pub place_check_ms: u64,
    /// Optional safety: after this many upgrade presses on one unit without
    /// seeing MAX / its level, count it as done (0 = off).
    pub upgrade_press_cap: u32,
    /// Open a unit with a right-click first (left-click is the default and
    /// what the game documents; in Roblox a right-drag turns the camera).
    pub upgrade_right_click: bool,
    /// Upgrades: wait this long (ms) after opening a unit / pressing upgrade
    /// before reading its level (OCR).
    pub upgrade_read_ms: u64,
    /// Shape tried when a map has no (or not enough) custom spots.
    pub place_shape: crate::game::placing::Shape,
    /// How far the shape spreads out (1.0 = AHK / v10 size).
    pub shape_scale: f32,
    /// Once every unit is upgraded: click an empty spot every 5 s until the
    /// end (keeps the PC awake, no AFK kick).
    pub idle_click: bool,
    /// Where that click goes (client px) - pick a spot with nothing on it.
    pub idle_click_at: (i32, i32),
    /// Every this many minutes, 5 clicks on each alt so it isn't kicked for
    /// being idle (0 = off).
    pub afk_every_min: u64,
    /// When the loop is stuck for good: close all Roblox, relaunch the
    /// accounts (main first) and start again.
    pub auto_relaunch: bool,
    /// Where Next sits below the VICTORY / DEFEAT title (px), for when the
    /// Next pattern isn't recognised.
    pub next_below_title: i32,
    /// Skip spots where units failed to go down before (per map + client).
    pub spot_memory: bool,
    /// Single / Safe (hover wiggle + re-click if it didn't take) / Double.
    pub click_mode: crate::input::ClickMode,
    /// How a placement is confirmed: the unit panel (UPG), the hotbar
    /// counter ("1/3", read with OCR), or panel first and counter as backup.
    /// (renamed from placement_detect so old saves start on Panel again)
    #[serde(rename = "placement_check")]
    pub placement_detect: PlacementDetect,
    /// Megumi's Mahoraga "Choose an Adaptation" picks.
    pub adaptations: Adaptations,
    /// After a restart mid-stage (within 5 min): pick up at the saved step
    /// (Auto), ask first (Ask) or always start from the lobby (Off).
    pub resume_mode: ResumeMode,
    /// Debug overlay: also draw patterns looked for and not found (red).
    pub overlay_misses: bool,
    /// Save a failure dump (debug/ folder) whenever a waited-for pattern
    /// never shows (every pattern; or tick dump_on_miss on single ones).
    pub dump_on_miss: bool,
    /// Beta: run rounds with the step machine (Lobby > StageSetup > Walk >
    /// Vote > Place > Upgrade > End, retries + Fatal pause) instead of the
    /// old loop.
    pub step_machine: bool,
    /// Farm one thing after another (each step: mode + map + team, for a
    /// number of runs or minutes).
    pub schedule: Schedule,
    /// Multi-instance unlock object names (held while `unlock` runs).
    pub singleton_names: Vec<String>,
    pub instances: Vec<InstanceProfile>,
    /// Hotbar slots 1-6 of the team in use (shared by every instance for now).
    pub units: Vec<UnitSlot>,
    /// Saved team loadouts; `team` is the one `units` belongs to.
    pub teams: Vec<Team>,
    pub team: usize,
    /// Mode label -> team to switch to when that mode is picked.
    pub mode_teams: std::collections::BTreeMap<String, usize>,
    /// What the main client farms (Home page).
    pub run: RunSettings,
    pub keybinds: Keybinds,
    pub webhook: crate::webhook::WebhookSettings,
    pub camera: crate::game::placing::CameraSettings,
    /// Clients that place units (1 = main, 2 = second client...). Each has
    /// its own spot set per map.
    pub placers: Vec<usize>,
    /// Client number -> team it places with (missing = the team in use).
    pub placer_teams: std::collections::BTreeMap<usize, usize>,
    /// Saved per account (username, lower case): does it place, and with
    /// which team. Applied to whatever client number the account gets.
    pub account_placing: std::collections::BTreeMap<String, AccountPlacing>,
    /// Use the spiral when a map has no custom spots (or they run out).
    pub spiral_fallback: bool,
    /// Dock the main client into the app's preview box; alts stack behind it.
    pub dock: bool,
    /// Fine-tune the docked position (screen px) if Roblox sits a bit off.
    pub dock_dx: i32,
    pub dock_dy: i32,
    /// The app itself lets several Roblox clients run (holds Roblox's
    /// single-instance lock), no RAM "Multi Roblox" needed.
    pub multi_roblox: bool,
    /// Roblox Account Manager web server port.
    pub ram_port: u16,
    /// RAM accounts: one main + alts, launch order = main first.
    pub accounts: Vec<crate::accounts::AccountSlot>,
    /// Built-in account manager or Roblox Account Manager.
    pub account_source: crate::accounts::AccountSource,
    /// Alts: private server link, or follow the main into its server.
    pub alt_join: crate::accounts::AltJoin,
    /// Seconds to let the main load in before alts follow it.
    pub follow_delay_s: u64,
    /// GitHub repo the updater reads releases from.
    pub update_repo: String,
    /// Check for a new build when the app opens.
    pub auto_update_check: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Mode {
    Story,
    Legend,
    Raid,
    Dungeon,
    #[serde(rename = "Infinity Castle")]
    InfinityCastle,
    Challenge,
    #[serde(rename = "Summer Hunt")]
    SummerHunt,
}

impl Mode {
    pub const ALL: [Mode; 7] = [
        Mode::SummerHunt,
        Mode::Story,
        Mode::Legend,
        Mode::Raid,
        Mode::InfinityCastle,
        Mode::Dungeon,
        Mode::Challenge,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Mode::Story => "Story",
            Mode::Legend => "Legend",
            Mode::Raid => "Raid",
            Mode::Dungeon => "Dungeon",
            Mode::InfinityCastle => "Infinity Castle",
            Mode::Challenge => "Challenge",
            Mode::SummerHunt => "Summer Event",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlayChoice {
    /// Your own portal / lobby (no captcha).
    PlayHere,
    /// Public matchmaking: shows a captcha you type yourself; the macro waits.
    FindMatch,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RunSettings {
    pub mode: Mode,
    pub play: PlayChoice,
    /// Alts join the main's portal / party; only the main places units.
    pub own_team: bool,
    pub return_to_lobby: bool,
    pub stop_after_defeat: bool,
    /// Story / Legend / Raid: map + act.
    pub map: String,
    pub act: String,
    /// Story acts: Next Level after a win (else Replay).
    pub next_level: bool,
    // Quick toggles
    pub priority_upgrade: bool,
    pub one_at_a_time: bool,
    pub auto_ability: bool,
    pub challenge_every_30: bool,
    /// Host invites alts through the in-game Party (+ > Invite).
    pub party_invite: bool,
    /// Card pick order (first offered one wins); empty = the default order.
    pub card_priority: Vec<String>,
    /// Infinity Castle: Hard instead of Normal.
    pub castle_hard: bool,
}

impl Default for RunSettings {
    fn default() -> Self {
        Self {
            mode: Mode::SummerHunt,
            play: PlayChoice::PlayHere,
            own_team: false,
            return_to_lobby: false,
            stop_after_defeat: false,
            map: "Planet Greenie".into(),
            act: "Act 1".into(),
            next_level: true,
            priority_upgrade: true,
            one_at_a_time: false,
            auto_ability: true,
            challenge_every_30: false,
            party_invite: true,
            castle_hard: false,
            card_priority: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Keybinds {
    /// Hotbar keys for slots 1-6.
    pub units: Vec<String>,
    /// Upgrade the open unit.
    pub upgrade: String,
    /// Close the open unit panel / cancel placement.
    pub cancel: String,
    /// Debug overlay on / off (F5, F7-F12; read at app start).
    #[serde(default = "d_overlay_key")]
    pub overlay: String,
}

fn d_overlay_key() -> String {
    "F9".into()
}

impl Keybinds {
    /// The overlay key as an F-key number (5, 7-12), F9 if not valid.
    pub fn overlay_fkey(&self) -> u32 {
        let k = self.overlay.trim().to_uppercase();
        k.strip_prefix('F')
            .and_then(|n| n.parse::<u32>().ok())
            .filter(|n| matches!(n, 5 | 7..=12))
            .unwrap_or(9)
    }
}

impl Default for Keybinds {
    fn default() -> Self {
        Self {
            units: (1..=6).map(|i| i.to_string()).collect(),
            upgrade: "r".into(),
            cancel: "q".into(),
            overlay: d_overlay_key(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum Upgrade {
    /// Don't upgrade.
    None,
    /// Upgrade until MAX shows.
    Max,
    /// Stop at this upgrade level.
    Level(u8),
}

/// What one account does in a stage.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct AccountPlacing {
    pub places: bool,
    /// Team index (None = the team in use).
    pub team: Option<usize>,
}

/// A named loadout of the 6 hotbar slots ("Summer farm", "Raid team"...).
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Team {
    pub name: String,
    pub units: Vec<UnitSlot>,
}

impl Settings {
    /// Keep `teams` valid: at least one, `team` in range, current units saved.
    pub fn fix_teams(&mut self) {
        if self.teams.is_empty() {
            self.teams.push(Team {
                name: "Team 1".into(),
                units: self.units.clone(),
            });
            self.team = 0;
        }
        self.team = self.team.min(self.teams.len() - 1);
        self.teams[self.team].units = self.units.clone();
    }

    /// Save the current slots into their team, then load team `i`.
    pub fn switch_team(&mut self, i: usize) {
        self.fix_teams();
        if i >= self.teams.len() || i == self.team {
            return;
        }
        self.team = i;
        self.units = self.teams[i].units.clone();
        let n = self.units.len();
        if n < 6 {
            self.ensure(0);
        }
    }

    /// New team copied from the current one; returns its index.
    pub fn add_team(&mut self, name: &str) -> usize {
        self.fix_teams();
        self.teams.push(Team {
            name: name.into(),
            units: self.units.clone(),
        });
        let i = self.teams.len() - 1;
        self.switch_team(i);
        i
    }

    pub fn remove_team(&mut self, i: usize) {
        self.fix_teams();
        if self.teams.len() <= 1 || i >= self.teams.len() {
            return;
        }
        self.teams.remove(i);
        self.mode_teams.retain(|_, t| *t != i);
        self.placer_teams.retain(|_, t| *t != i);
        for a in self.account_placing.values_mut() {
            match a.team {
                Some(t) if t == i => a.team = None,
                Some(t) if t > i => a.team = Some(t - 1),
                _ => {}
            }
        }
        for t in self.placer_teams.values_mut() {
            if *t > i {
                *t -= 1;
            }
        }
        for t in self.mode_teams.values_mut() {
            if *t > i {
                *t -= 1;
            }
        }
        let next = if self.team == i {
            i.saturating_sub(1)
        } else if self.team > i {
            self.team - 1
        } else {
            self.team
        };
        self.team = next.min(self.teams.len() - 1);
        self.units = self.teams[self.team].units.clone();
    }

    /// Put each account's saved placing / team onto its current client
    /// number. `names[i]` = username of client i+1 (if known).
    pub fn apply_account_placing(&mut self, names: &[Option<String>]) {
        for (i, n) in names.iter().enumerate() {
            let Some(n) = n else { continue };
            let Some(a) = self.account_placing.get(&n.to_lowercase()).cloned() else {
                continue;
            };
            let no = i + 1;
            if a.places && !self.placers.contains(&no) {
                self.placers.push(no);
                self.placers.sort();
            } else if !a.places {
                self.placers.retain(|x| *x != no);
            }
            match a.team {
                Some(t) if t < self.teams.len() => {
                    self.placer_teams.insert(no, t);
                }
                _ => {
                    self.placer_teams.remove(&no);
                }
            }
        }
    }

    /// Unit slots client `n` (1 = first) places with.
    pub fn units_for(&self, n: usize) -> Vec<UnitSlot> {
        match self.placer_teams.get(&n) {
            Some(&t) if t != self.team && t < self.teams.len() => {
                let mut u = self.teams[t].units.clone();
                u.resize_with(6, UnitSlot::default);
                u
            }
            _ => self.units.clone(),
        }
    }

    /// Picking a mode loads the team linked to it (if any).
    pub fn team_for_mode(&mut self, mode: Mode) {
        if let Some(&t) = self.mode_teams.get(mode.label()) {
            self.switch_team(t);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UnitSlot {
    pub enabled: bool,
    /// Your own note, e.g. "Erwin".
    pub name: String,
    /// How many copies to place.
    pub place: u8,
    pub upgrade: Upgrade,
    /// Lower = upgraded first.
    pub priority: u8,
    /// Buff unit: turn Auto ability on one copy at a time.
    pub buff: bool,
    /// Farm unit (money maker): placed first, upgraded early.
    pub farm: bool,
    /// Hill unit: only goes on high ground - its good / bad spots are
    /// remembered separately from ground units.
    pub hill: bool,
    /// Alternate this slot's upgrades with placement instead of waiting.
    pub upg_immediately: bool,
}

impl Default for UnitSlot {
    fn default() -> Self {
        Self {
            enabled: false,
            name: String::new(),
            place: 1,
            upgrade: Upgrade::Max,
            priority: 1,
            buff: false,
            farm: false,
            hill: false,
            upg_immediately: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Drives its own farm.
    Independent,
    /// Party host: drives the stage, waits for followers in the lobby.
    Host,
    /// Party follower: only walks into the host's lobby, then idles.
    Follower,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct InstanceProfile {
    pub label: String,
    pub role: Role,
    pub enabled: bool,
    pub mode: Mode,
    /// Map / act name as shown in game, e.g. "Planet Greenie".
    pub map: String,
}

impl Default for InstanceProfile {
    fn default() -> Self {
        Self {
            label: String::new(),
            role: Role::Independent,
            enabled: true,
            mode: Mode::Story,
            map: String::new(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            process_name: "RobloxPlayerBeta.exe".into(),
            client_w: 800,
            client_h: 599,
            max_instances: 3,
            max_lease_ms: 400,
            focus_timeout_ms: 600,
            mouse_settle_ms: 35,
            click_hold_ms: 40,
            click_mode: crate::input::ClickMode::Safe,
            placement_detect: PlacementDetect::Panel,
            adaptations: Adaptations::default(),
            schedule: Schedule::default(),
            dump_on_miss: false,
            step_machine: false,
            overlay_misses: false,
            resume_mode: ResumeMode::Ask,
            step_delay_ms: 800,
            place_delay_ms: 2000,
            place_settle_ms: 2000,
            place_check_ms: 3000,
            upgrade_read_ms: 800,
            upgrade_press_cap: 0,
            upgrade_right_click: false,
            swap_delay_ms: 1200,
            place_shape: Default::default(),
            shape_scale: 1.8,
            idle_click: true,
            next_below_title: 205,
            auto_relaunch: true,
            afk_every_min: 5,
            idle_click_at: (400, 110),
            spot_memory: true,
            singleton_names: vec!["ROBLOX_singletonMutex".into()],
            run: RunSettings::default(),
            keybinds: Keybinds::default(),
            webhook: crate::webhook::WebhookSettings::default(),
            camera: crate::game::placing::CameraSettings::default(),
            placers: vec![1],
            placer_teams: Default::default(),
            account_placing: Default::default(),
            spiral_fallback: true,
            teams: vec![],
            team: 0,
            mode_teams: Default::default(),
            dock: true,
            dock_dx: 0,
            dock_dy: 0,
            multi_roblox: true,
            ram_port: 7963,
            accounts: vec![],
            alt_join: Default::default(),
            account_source: Default::default(),
            follow_delay_s: 30,
            update_repo: "DenniXDReal/aa-macro-rs".into(),
            auto_update_check: true,
            instances: (1..=3)
                .map(|i| InstanceProfile {
                    label: format!("Client {i}"),
                    ..Default::default()
                })
                .collect(),
            units: (1..=6)
                .map(|i| UnitSlot {
                    priority: i,
                    ..Default::default()
                })
                .collect(),
        }
    }
}

impl Settings {
    /// Make sure there is a profile for client `i` (0-based) and 6 unit slots.
    pub fn ensure(&mut self, clients: usize) {
        while self.instances.len() < clients {
            let n = self.instances.len() + 1;
            self.instances.push(InstanceProfile {
                label: format!("Client {n}"),
                ..Default::default()
            });
        }
        while self.units.len() < 6 {
            let n = self.units.len() as u8 + 1;
            self.units.push(UnitSlot {
                priority: n,
                ..Default::default()
            });
        }
    }

    pub fn default_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("aa-macro.json")))
            .unwrap_or_else(|| PathBuf::from("aa-macro.json"))
    }

    /// Load, or write defaults when the file doesn't exist.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if !path.exists() {
            let s = Settings::default();
            s.save(path)?;
            return Ok(s);
        }
        let txt =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut s: Settings =
            serde_json::from_str(&txt).with_context(|| format!("parsing {}", path.display()))?;
        s.ensure(0);
        s.fix_teams();
        Ok(s)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut me = self.clone();
        me.fix_teams();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&me)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_json_gets_defaults() {
        let s: Settings = serde_json::from_str(r#"{"max_instances": 2}"#).unwrap();
        assert_eq!(s.max_instances, 2);
        assert_eq!(s.client_w, 800);
        assert_eq!(s.instances.len(), 3);
        assert_eq!(s.units.len(), 6);
    }

    #[test]
    fn modes_and_upgrades_serialise() {
        let mut s = Settings::default();
        s.instances[0].mode = Mode::SummerHunt;
        s.units[0].upgrade = Upgrade::Level(5);
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("\"Summer Hunt\""));
        let back: Settings = serde_json::from_str(&j).unwrap();
        assert_eq!(back.instances[0].mode, Mode::SummerHunt);
        assert_eq!(back.units[0].upgrade, Upgrade::Level(5));
        let mut short = Settings {
            instances: vec![],
            units: vec![],
            ..Default::default()
        };
        short.ensure(3);
        assert_eq!((short.instances.len(), short.units.len()), (3, 6));
    }

    #[test]
    fn roundtrip_file() {
        let dir = std::env::temp_dir().join(format!("aa-macro-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("aa-macro.json");
        let s = Settings::load_or_create(&p).unwrap();
        assert!(p.exists());
        let again = Settings::load_or_create(&p).unwrap();
        assert_eq!(s.process_name, again.process_name);
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod team_tests {
    use super::*;

    #[test]
    fn two_clients_two_teams() {
        let mut s = Settings::default();
        s.ensure(0);
        s.fix_teams();
        s.units[0].name = "Main team unit".into();
        s.add_team("Second");
        s.units[0].name = "Alt team unit".into();
        s.switch_team(0);
        s.placers = vec![1, 2];
        s.placer_teams.insert(2, 1);
        assert_eq!(s.units_for(1)[0].name, "Main team unit");
        assert_eq!(s.units_for(2)[0].name, "Alt team unit");
        // saved per account: follows the account to another client number
        s.account_placing.insert(
            "altacc".into(),
            AccountPlacing {
                places: true,
                team: Some(1),
            },
        );
        s.placers = vec![1];
        s.placer_teams.clear();
        s.apply_account_placing(&[Some("Main".into()), None, Some("AltAcc".into())]);
        assert_eq!(s.placers, vec![1, 3]);
        assert_eq!(s.units_for(3)[0].name, "Alt team unit");
        s.remove_team(1);
        assert!(s.placer_teams.is_empty());
        assert_eq!(s.account_placing["altacc"].team, None);
        assert_eq!(s.units_for(2)[0].name, "Main team unit");
    }
}

/// How a placement is confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlacementDetect {
    /// The unit panel (unit_placed / UPG) shows after the click.
    #[default]
    Panel,
    /// The unit's hotbar counter ("1/3") went up.
    Hotbar,
    /// Panel first; when it isn't seen, check the counter.
    Both,
}

impl PlacementDetect {
    pub const ALL: [PlacementDetect; 3] = [Self::Panel, Self::Both, Self::Hotbar];
    pub fn label(self) -> &'static str {
        match self {
            Self::Panel => "Unit panel",
            Self::Hotbar => "Hotbar counter (1/3)",
            Self::Both => "Panel, counter as backup",
        }
    }
}

/// Mahoraga's adaptation choices, top one first, per badge (ATK/DEF/BOSS).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Adaptations {
    pub enabled: bool,
    pub atk: Vec<String>,
    pub def: Vec<String>,
    pub boss: Vec<String>,
}

pub const ADAPTATION_NAMES: [&str; 3] = ["Divine Wings", "Heavenly Force", "Adapted Slash"];

impl Default for Adaptations {
    fn default() -> Self {
        let v: Vec<String> = ADAPTATION_NAMES.iter().map(|s| s.to_string()).collect();
        Self {
            enabled: true,
            atk: v.clone(),
            def: v.clone(),
            boss: v,
        }
    }
}

/// Farm list: the macro works through the steps in order.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    pub enabled: bool,
    /// After the last step, start again at the first.
    pub repeat: bool,
    pub steps: Vec<ScheduleStep>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ScheduleStep {
    pub enabled: bool,
    pub mode: Mode,
    pub play: PlayChoice,
    /// Story / Legend / Raid.
    pub map: String,
    pub act: String,
    pub castle_hard: bool,
    /// Team to use (index), None = the team saved for that mode.
    pub team: Option<usize>,
    /// Move on after this many runs (0 = no limit).
    pub runs: u32,
    /// ... or after this many minutes (0 = no limit). Whichever comes first;
    /// the current run is always finished.
    pub minutes: u32,
}

impl Default for ScheduleStep {
    fn default() -> Self {
        let r = RunSettings::default();
        Self {
            enabled: true,
            mode: r.mode,
            play: r.play,
            map: r.map,
            act: r.act,
            castle_hard: false,
            team: None,
            runs: 10,
            minutes: 0,
        }
    }
}

impl ScheduleStep {
    /// A step from what's set up now.
    pub fn from_run(s: &Settings) -> Self {
        Self {
            mode: s.run.mode,
            play: s.run.play,
            map: s.run.map.clone(),
            act: s.run.act.clone(),
            castle_hard: s.run.castle_hard,
            team: Some(s.team),
            ..Default::default()
        }
    }

    /// The settings for this step, starting from `base`.
    pub fn apply(&self, base: &Settings) -> Settings {
        let mut s = base.clone();
        s.run.mode = self.mode;
        s.run.play = self.play;
        s.run.map = self.map.clone();
        s.run.act = self.act.clone();
        s.run.castle_hard = self.castle_hard;
        match self.team {
            Some(t) => s.switch_team(t),
            None => s.team_for_mode(self.mode),
        }
        s
    }

    pub fn label(&self) -> String {
        let what = match self.mode {
            Mode::Story | Mode::Legend | Mode::Raid => {
                format!("{} {} {}", self.mode.label(), self.map, self.act)
            }
            m => m.label().to_string(),
        };
        let until = match (self.runs, self.minutes) {
            (0, 0) => "no limit".to_string(),
            (r, 0) => format!("{r} runs"),
            (0, m) => format!("{m} min"),
            (r, m) => format!("{r} runs or {m} min"),
        };
        format!("{what} ({until})")
    }
}

#[cfg(test)]
mod schedule_tests {
    use super::*;

    #[test]
    fn step_applies_mode_and_team() {
        let mut base = Settings::default();
        let t = base.add_team("Raid team");
        base.switch_team(0);
        let st = ScheduleStep {
            mode: Mode::Raid,
            map: "Ruined City".into(),
            act: "Act 2".into(),
            team: Some(t),
            runs: 5,
            minutes: 30,
            ..Default::default()
        };
        let s = st.apply(&base);
        assert_eq!(s.run.mode, Mode::Raid);
        assert_eq!(s.run.map, "Ruined City");
        assert_eq!(s.team, t);
        assert_eq!(st.label(), "Raid Ruined City Act 2 (5 runs or 30 min)");
        assert_eq!(base.run.mode, Mode::SummerHunt, "base untouched");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ResumeMode {
    Off,
    #[default]
    Ask,
    Auto,
}
