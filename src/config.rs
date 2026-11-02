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
    /// Shape tried when a map has no (or not enough) custom spots.
    pub place_shape: crate::game::placing::Shape,
    /// Single / Safe (hover wiggle + re-click if it didn't take) / Double.
    pub click_mode: crate::input::ClickMode,
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
}

impl Default for Keybinds {
    fn default() -> Self {
        Self {
            units: (1..=6).map(|i| i.to_string()).collect(),
            upgrade: "r".into(),
            cancel: "q".into(),
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
            step_delay_ms: 800,
            place_delay_ms: 2000,
            swap_delay_ms: 1200,
            place_shape: Default::default(),
            singleton_names: vec!["ROBLOX_singletonMutex".into()],
            run: RunSettings::default(),
            keybinds: Keybinds::default(),
            webhook: crate::webhook::WebhookSettings::default(),
            camera: crate::game::placing::CameraSettings::default(),
            placers: vec![1],
            placer_teams: Default::default(),
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
        s.remove_team(1);
        assert!(s.placer_teams.is_empty());
        assert_eq!(s.units_for(2)[0].name, "Main team unit");
    }
}
