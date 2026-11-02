//! The game loop (v1): lobby walk -> matchmaking / portal -> load in ->
//! map walk -> top-down camera -> place (every placing client) -> upgrade by
//! priority until the end screen -> Next -> Replay or back to the lobby.
//!
//! Everything it looks for is one of your patterns; [`plan`] lists them per
//! step so the Home page can show what's still missing before you Start.

use super::placing::{camera_batch, place_slot, spots_key, Keys};
use super::LiveScreen;
use crate::config::{Mode, PlayChoice, Settings, Upgrade};
use crate::control::Control;
use crate::geom::{Hwnd, Point};
use crate::input::{Action, ArbiterHandle, Batch};
use crate::keys::Key;
use crate::movement::{Routes, Vars};
use crate::placement::Placements;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// Lobby walk used by a mode.
pub fn lobby_route(mode: Mode) -> &'static str {
    match mode {
        Mode::SummerHunt => "summer",
        Mode::Story => "story",
        Mode::Legend => "legend",
        Mode::Raid => "raid",
        Mode::InfinityCastle => "castle",
        Mode::Challenge | Mode::Dungeon => "challenge",
    }
}

/// Map-detect patterns are called `loading_<map>` (or `loading_<map>_2`
/// for a second look of the same map). This turns one back into a map name.
pub fn map_from_pattern(p: &str) -> String {
    let rest = p.trim_start_matches("loading_");
    let rest = match rest.rsplit_once('_') {
        Some((a, n)) if n.chars().all(|c| c.is_ascii_digit()) => a,
        _ => rest,
    };
    rest.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Pattern name used to recognise `map` while it loads.
pub fn loading_pattern(map: &str) -> String {
    format!("loading_{}", crate::vision::key(&route_map(map)))
}

/// Map name as the per-map routes know it ("Ruined City (Midnight)" -> "Ruined City").
pub fn route_map(name: &str) -> String {
    name.split(" (").next().unwrap_or(name).trim().to_string()
}

fn vars(s: &Settings) -> Vars {
    Vars {
        map: s.run.map.clone(),
        act: s.run.act.clone(),
        click_wait_ms: s.step_delay_ms,
    }
}

/// One step of the sequence and the patterns it needs (`optional` = it
/// carries on without them).
#[derive(Clone, Debug)]
pub struct PlanStep {
    pub label: String,
    pub needs: Vec<String>,
    pub optional: bool,
}

/// The whole sequence for the chosen mode, step by step.
pub fn plan(s: &Settings, routes: &Routes) -> Vec<PlanStep> {
    let mode = s.run.mode;
    let v = vars(s);
    let step = |label: &str, needs: &[&str], optional: bool| PlanStep {
        label: label.into(),
        needs: needs.iter().map(|n| n.to_string()).collect(),
        optional,
    };
    let mut out = vec![];
    if s.run.own_team && s.run.party_invite {
        out.push(step(
            "Party: main + > spam Invite until none left > alt 1 Accept > alt 2 Accept ... > back to main > close party menu > game mode",
            &["party_plus", "party_invite", "party_invited", "party_accept", "party_close"],
            true,
        ));
    }
    let lobby = lobby_route(mode);
    match routes.lobby.get(lobby) {
        Some(r) => out.push(PlanStep {
            label: format!("Walk: {}", r.map),
            needs: r.patterns(&v),
            optional: false,
        }),
        None => out.push(step(&format!("Walk: no '{lobby}' lobby route"), &[], false)),
    }
    match s.run.play {
        PlayChoice::PlayHere => out.push(step(
            "Join Matchmaking > Play Here",
            &["join_matchmaking", "play_here"],
            false,
        )),
        PlayChoice::FindMatch => out.push(step(
            "Join Matchmaking > Find Match (you type the captcha)",
            &["join_matchmaking", "find_match", "captcha"],
            false,
        )),
    }
    if mode == Mode::SummerHunt && s.run.play == PlayChoice::PlayHere {
        let open = if s.run.own_team {
            "open_friends_only"
        } else {
            "open_button"
        };
        out.push(step("Open Portal? > Open", &["open_portal", open], false));
        if s.run.own_team && !s.run.party_invite {
            out.push(step(
                "Alts walk in and press E at the portal",
                &["portal_join_e"],
                false,
            ));
        }
        out.push(step("Portal lobby > Start", &["start_button"], false));
    } else if s.run.play == PlayChoice::PlayHere {
        out.push(step("Start", &["start_button"], false));
    }
    let fixed = matches!(mode, Mode::Story | Mode::Legend | Mode::Raid);
    if fixed {
        out.push(step(
            &format!("Loading: check it's {}", route_map(&s.run.map)),
            &[],
            true,
        ));
    } else {
        out.push(step(
            "Loading: work out the map (random each hour) from its loading screen",
            &[],
            true,
        ));
    }
    out.push(step("Camera top-down", &[], true));
    out.push(step(
        "Settings > Teleport to Spawn > close",
        &[
            "settings_button",
            "tp_spawn",
            "tp_spawn_button",
            "settings_close",
        ],
        true,
    ));
    let walk = if fixed {
        let m = route_map(&s.run.map);
        if routes.find_map(&m).is_some() {
            format!("Map walk: {m}")
        } else {
            format!("Map walk: none for {m} (stays at spawn)")
        }
    } else {
        "Map walk: the detected map's route (none = stays at spawn)".into()
    };
    out.push(step(&walk, &[], true));
    let units = s.units.iter().filter(|u| u.enabled && u.place > 0).count();
    out.push(step("Vote start", &["vote_start"], true));
    out.push(step(
        &format!(
            "Place {units} unit slot(s) on {} client(s): FARM first, UPG [0] badge check, Auto ability on BUFF, cards picked, stops if VICTORY / DEFEAT",
            s.placers.len().max(1)
        ),
        &["unit_placed", "auto_ability"],
        true,
    ));
    out.push(step(
        "Upgrade by priority (R): counts only when UPG [n] goes up, 1-8 or until MAX, FARM first; VICTORY / DEFEAT watched all the time; cards picked",
        &["choose_card", "card_confirm", "upgrade_max", "upg_1", "upg_2"],
        true,
    ));
    out.push(step(
        "End: VICTORY / DEFEAT > Next",
        &["victory", "defeat", "next_button"],
        false,
    ));
    if s.run.return_to_lobby {
        out.push(step(
            "Return to lobby, then repeat",
            &["return_to_lobby"],
            false,
        ));
    } else {
        out.push(step(
            "Main: Next > Replay, wait for Vote start, repeat from Teleport to Spawn",
            &["replay"],
            false,
        ));
    }
    out
}

pub enum Event {
    Log(String),
    Win,
    Loss,
    /// Something needs the user (captcha) - ping them.
    Attention(String),
    Done(String),
}

pub struct RunCtx {
    pub arb: ArbiterHandle,
    pub control: Control,
    pub main: Hwnd,
    /// Every client: (hwnd, screen). Index = client number - 1.
    pub clients: Vec<(Hwnd, LiveScreen)>,
    pub settings: Settings,
    pub routes: Routes,
    pub placements: Placements,
    pub tx: Sender<Event>,
    /// 1.0 = real timing; tests use a small value.
    pub speed: f32,
    /// Spots that failed before (shared with the app, saved to disk).
    pub memory: std::sync::Arc<std::sync::Mutex<crate::spotmem::SpotMemory>>,
}

type R<T> = Result<T, String>;

/// OCR of the whole screen costs a moment: at most every 2 s per client.
fn ocr_due(h: Hwnd) -> bool {
    static LAST: std::sync::Mutex<Vec<(Hwnd, Instant)>> = std::sync::Mutex::new(Vec::new());
    let mut v = LAST.lock().unwrap();
    match v.iter_mut().find(|e| e.0 == h) {
        Some(e) if e.1.elapsed() < Duration::from_secs(2) => false,
        Some(e) => {
            e.1 = Instant::now();
            true
        }
        None => {
            v.push((h, Instant::now()));
            true
        }
    }
}

/// When a card was last picked (any client) - one pick does it for all.
static LAST_PICK: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

impl RunCtx {
    fn log(&self, s: impl Into<String>) {
        let _ = self.tx.send(Event::Log(s.into()));
    }
    fn screen(&self, h: Hwnd) -> &LiveScreen {
        &self.clients.iter().find(|c| c.0 == h).expect("client").1
    }
    fn check(&self) -> R<()> {
        self.control.checkpoint().map_err(|_| "stopped".to_string())
    }
    fn sleep(&self, ms: u64) -> R<()> {
        self.control
            .sleep((ms as f32 * self.speed) as u64)
            .map_err(|_| "stopped".to_string())
    }
    fn secs(&self, s: f64) -> f64 {
        s * self.speed as f64
    }
    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    /// Wait up to `ms` for any of `names` on `h`; returns (name, centre).
    fn wait_any(&self, h: Hwnd, names: &[&str], ms: u64) -> R<Option<(String, Point)>> {
        let s = self.screen(h);
        let mut dl = self
            .control
            .deadline(self.secs(ms as f64 / 1000.0).max(0.05));
        loop {
            self.check()?;
            for n in names {
                if let Some(p) = s.find(&[n.to_string()], None) {
                    return Ok(Some((n.to_string(), p)));
                }
            }
            if dl.expired() {
                return Ok(None);
            }
            self.sleep(150)?;
        }
    }
    fn click(&self, h: Hwnd, p: Point, why: &str) -> R<()> {
        self.arb
            .run(Batch::new(h, why).click(p.x, p.y))
            .map_err(|e| format!("{why}: {e}"))?;
        // give the game time to react before the next look / click
        self.sleep(self.settings.step_delay_ms)?;
        Ok(())
    }
    /// Wait for `name` then click it; error when it never shows.
    fn click_when(&self, h: Hwnd, name: &str, ms: u64) -> R<()> {
        match self.wait_any(h, &[name], ms)? {
            Some((_, p)) => {
                self.click(h, p, name)?;
                if self.settings.click_mode == crate::input::ClickMode::Safe {
                    // still there, same place = the click didn't register
                    self.sleep(400)?;
                    if let Some(q) = self.screen(h).find(&[name.to_string()], None) {
                        if (q.x - p.x).abs() <= 6 && (q.y - p.y).abs() <= 6 {
                            self.log(format!("'{name}' click didn't take - clicking again"));
                            self.click(h, q, name)?;
                        }
                    }
                }
                Ok(())
            }
            None => Err(format!("'{name}' never showed up")),
        }
    }

    /// Lobby -> in the stage.
    pub fn enter_stage(&self) -> R<()> {
        let s = &self.settings;
        let mode = s.run.mode;
        let key = lobby_route(mode);
        let route = self
            .routes
            .lobby
            .get(key)
            .ok_or(format!("no lobby route '{key}'"))?;
        self.log(format!("walking: {}", route.map));
        let batch = route
            .to_batch(self.main, self.screen(self.main), &vars(s))
            .map_err(|e| e.to_string())?;
        self.arb
            .run(batch)
            .map_err(|e| format!("lobby walk: {e}"))?;
        // Join Matchmaking?
        match s.run.play {
            PlayChoice::PlayHere => self.click_when(self.main, "play_here", 6000)?,
            PlayChoice::FindMatch => {
                self.click_when(self.main, "find_match", 6000)?;
                if self.wait_any(self.main, &["captcha"], 3000)?.is_some() {
                    let _ = self.tx.send(Event::Attention(
                        "Captcha - type it in Roblox, the macro waits".into(),
                    ));
                    // wait until the captcha is gone (typed) - up to 2 min
                    let mut dl = self.control.deadline(self.secs(120.0));
                    while self
                        .screen(self.main)
                        .find(&Self::names(&["captcha"]), None)
                        .is_some()
                    {
                        if dl.expired() {
                            return Err("captcha not typed in time".into());
                        }
                        self.sleep(500)?;
                    }
                }
                self.log("matchmaking - waiting to load");
                return Ok(());
            }
        }
        if mode == Mode::SummerHunt {
            let open = if s.run.own_team {
                "open_friends_only"
            } else {
                "open_button"
            };
            if self.wait_any(self.main, &["open_portal"], 5000)?.is_some() {
                self.click_when(self.main, open, 4000)?;
            }
            // in a party the alts come along by themselves - they don't move
            if s.run.own_team && !s.run.party_invite {
                self.alts_join_portal()?;
                self.to_main()?;
            }
        }
        self.click_when(self.main, "start_button", 10000)?;
        self.log("started - loading");
        Ok(())
    }

    /// Alts walk into the main's portal and press E.
    fn alts_join_portal(&self) -> R<()> {
        let e = Key::parse("e").expect("e");
        let w = Key::parse("w").expect("w");
        for (h, scr) in self.clients.iter().filter(|c| c.0 != self.main) {
            self.log("alt joining the portal");
            let probe = {
                let s = scr.clone();
                crate::input::Probe::new("portal_join_e", move || {
                    s.find(&["portal_join_e".to_string()], None)
                })
            };
            let b = Batch::new(*h, "alt: join portal")
                .then(Action::KeyDown(w))
                .then(Action::WaitFor {
                    probe,
                    timeout_ms: 12000,
                    required: false,
                })
                .then(Action::KeyUp(w))
                .then(Action::KeyTap {
                    key: e,
                    hold_ms: 80,
                });
            let mut b = b;
            b.long = true;
            self.arb.run(b).map_err(|e| format!("alt join: {e}"))?;
        }
        Ok(())
    }

    /// Card pick screen open on `h`? Take the green card, else the middle
    /// one, then Confirm. Returns true when it picked.
    pub fn handle_cards(&self, h: Hwnd) -> R<bool> {
        let scr = self.screen(h);
        // card screen: your choose_card capture, or OCR reading "Choose a card"
        let mut weather = String::new();
        let seen =
            if scr.has("choose_card") && scr.find(&Self::names(&["choose_card"]), None).is_some() {
                true
            } else if let Some(w) = ocr_due(h)
                .then(|| scr.frame().and_then(|f| scr.ocr_card_screen(&f)))
                .flatten()
            {
                weather = w;
                true
            } else {
                false
            };
        if !seen {
            return Ok(false);
        }
        // one pick is enough for everyone: skip if another client just did it
        {
            let mut last = LAST_PICK.lock().unwrap();
            if last.is_some_and(|t| t.elapsed() < Duration::from_secs(6)) {
                return Ok(false);
            }
            *last = Some(Instant::now());
        }
        // close the unit panel first if one is open (its X is on screen);
        // no panel = straight to the screenshot
        if let Some(p) = scr
            .has("unit_close")
            .then(|| scr.find_wide(&Self::names(&["unit_close"])))
            .flatten()
        {
            self.click(h, p, "close unit panel")?;
            self.sleep(400)?;
        }
        let Some(f) = scr.frame() else {
            return Ok(false);
        };
        // keep a picture of every different card screen (cards/ folder)
        if weather.is_empty() {
            weather = scr.ocr_card_screen(&f).unwrap_or_default();
        }
        if !weather.is_empty() {
            self.log(format!("card screen - weather: {weather}"));
        }
        if let Some(path) = super::cardlog::save_if_new(&f, &super::cardlog::dir(), &weather) {
            self.log(format!("card screen saved: {}", path.display()));
        }
        let (p, green) = super::cards::pick(&f);
        self.log(if green {
            "card: green one"
        } else {
            "card: no green - middle one"
        });
        self.click(h, p, "card")?;
        self.sleep(500)?;
        match self.wait_any(
            h,
            &["card_confirm"],
            if scr.has("card_confirm") { 2500 } else { 0 },
        )? {
            Some((_, c)) => self.click(h, c, "confirm card")?,
            // no confirm pattern: the button sits under the card
            None => self.click(h, Point::new(p.x, (p.y + 120).min(580)), "confirm card")?,
        }
        let mut dl = self.control.deadline(self.secs(3.0));
        while (scr.find(&Self::names(&["choose_card"]), None).is_some()
            || scr.frame().and_then(|f| scr.ocr_card_screen(&f)).is_some())
            && !dl.expired()
        {
            self.sleep(300)?;
        }
        Ok(true)
    }

    /// Card screen on any client (alts first); back to the main after an alt.
    fn cards_all(&self) -> R<()> {
        let all: Vec<Hwnd> = self.clients.iter().map(|c| c.0).collect();
        let mut alt_done = false;
        for h in self.main_last(all) {
            if self.handle_cards(h)? {
                alt_done = h != self.main;
                break; // one pick is enough
            }
        }
        if alt_done {
            self.to_main()?;
        }
        Ok(())
    }

    /// Main invites every alt into its party, each alt accepts.
    pub fn party_up(&self) -> R<()> {
        let alts: Vec<Hwnd> = self
            .clients
            .iter()
            .map(|c| c.0)
            .filter(|h| *h != self.main)
            .collect();
        if alts.is_empty() {
            return Ok(());
        }
        let main = self.screen(self.main);
        if !main.has("party_plus") || !main.has("party_invite") {
            self.log("party: capture party_plus + party_invite first - skipping");
            return Ok(());
        }
        self.log(format!("party: inviting {} alt(s)", alts.len()));
        // 1. click + once, then stop looking for it
        self.click_when(self.main, "party_plus", 5000)?;
        // 2. wait for the first Invite button
        let mut dl = self.control.deadline(self.secs(8.0));
        while main.find_all_wide("party_invite").is_empty() && !dl.expired() {
            self.sleep(400)?;
        }
        if main.find_all_wide("party_invite").is_empty() {
            self.log("party: no Invite button showed up - carrying on without a party");
            return Ok(());
        }
        // 3. spam-click every Invite until none is left (each turns into Invited)
        let sent = self.spam_invites()?;
        let invited = !main.has("party_invited")
            || main.find(&Self::names(&["party_invited"]), None).is_some();
        self.log(if invited {
            format!("party: {sent} invite click(s), no Invite left, Invited shown")
        } else {
            "party: no Invite left but no 'Invited' either - trying the alts anyway".into()
        });
        // 4. one alt at a time: swap to it and Accept, through every alt.
        //    Only if one missed the invite: re-invite on the main, retry once.
        let mut waiting: Vec<Hwnd> = alts.clone();
        for round in 1..=2 {
            let mut missed = vec![];
            for (i, h) in waiting.iter().enumerate() {
                self.log(format!(
                    "party: alt {} of {} - accepting",
                    i + 1,
                    waiting.len()
                ));
                match self.wait_any(*h, &["party_accept"], 15000)? {
                    Some((_, _)) => {
                        // click_when re-clicks if the pop-up is still there
                        if let Err(e) = self.click_when(*h, "party_accept", 3000) {
                            self.log(format!("party: {e}"));
                        }
                        self.sleep(1000)?;
                        if self
                            .screen(*h)
                            .find(&Self::names(&["party_accept"]), None)
                            .is_some()
                        {
                            missed.push(*h);
                        }
                    }
                    None => missed.push(*h),
                }
            }
            if missed.is_empty() {
                break;
            }
            if round == 2 {
                self.log(format!(
                    "party: {} alt(s) still not in the party",
                    missed.len()
                ));
                break;
            }
            // only if an alt missed it: back to the main, invite again, retry those
            self.to_main()?;
            self.spam_invites()?;
            self.log(format!(
                "party: {} alt(s) didn't get the invite{}",
                missed.len(),
                if round == 1 { " - inviting again" } else { "" }
            ));
            waiting = missed;
        }
        // 5. back to the main, close the party menu, then Play / Areas / Create lobby
        self.to_main()?;
        if main.has("party_close") {
            match self.wait_any(self.main, &["party_close"], 3000)? {
                Some((_, p)) => self.click(self.main, p, "close party menu")?,
                None => self.log("party: close button not seen - menu left open"),
            }
        } else {
            self.log("party: capture party_close to close the party menu");
        }
        self.log("party: done - back on the main, going to the game mode");
        Ok(())
    }

    /// On the main: click every Invite button until none is left for a
    /// moment. Returns how many were clicked.
    fn spam_invites(&self) -> R<u32> {
        let main = self.screen(self.main);
        let mut sent = 0;
        let mut empty_scans = 0;
        // scan the whole screen for every Invite (one per alt), click them all,
        // scan again - until a couple of scans in a row find none
        for _ in 0..10 {
            self.check()?;
            let all = main.find_all_wide("party_invite");
            if all.is_empty() {
                empty_scans += 1;
                if empty_scans >= 2 {
                    break;
                }
                self.sleep(700)?;
                continue;
            }
            empty_scans = 0;
            self.log(format!("party: {} Invite button(s) on screen", all.len()));
            for p in all {
                self.arb
                    .run(Batch::new(self.main, "invite").click(p.x, p.y))
                    .map_err(|e| format!("invite: {e}"))?;
                sent += 1;
                self.sleep(400)?;
            }
            self.sleep(600)?;
        }
        Ok(sent)
    }

    /// Bring the main client back to the front (after working on alts).
    fn to_main(&self) -> R<()> {
        self.arb
            .run(Batch::new(self.main, "back to main"))
            .map_err(|e| format!("back to main: {e}"))?;
        self.sleep(300)?;
        Ok(())
    }

    /// Alts first, the main last - so the sequence ends on the main.
    fn main_last(&self, v: Vec<Hwnd>) -> Vec<Hwnd> {
        let mut v: Vec<Hwnd> = v.into_iter().filter(|h| *h != self.main).collect();
        v.push(self.main);
        v
    }

    /// Clients that move / place: the main, then every placing client.
    fn movers(&self) -> Vec<Hwnd> {
        let mut v = vec![self.main];
        for n in &self.settings.placers {
            if let Some((h, _)) = self.clients.get(n.saturating_sub(1)) {
                if !v.contains(h) {
                    v.push(*h);
                }
            }
        }
        v
    }

    /// From the lobby leaving (or Replay) until Vote Start is pressed:
    /// work out the map while it loads, top-down camera, Teleport to Spawn,
    /// walk the map's route, then Vote Start. Returns the map ("" = unknown).
    pub fn stage_setup(&self, prev_map: &str) -> R<String> {
        let s = &self.settings;
        let main = self.screen(self.main);
        let gone = Self::names(&[
            "victory",
            "defeat",
            "replay",
            "start_button",
            "waiting_for_players",
            "join_matchmaking",
            "captcha",
        ]);
        let mut dl = self.control.deadline(self.secs(90.0));
        while main.find(&gone, None).is_some() {
            if dl.expired() {
                return Err("the end / lobby screen is still up after 90 s".into());
            }
            self.sleep(500)?;
        }
        let fixed = matches!(s.run.mode, Mode::Story | Mode::Legend | Mode::Raid);
        let detect = main.with_prefix("loading_");
        let vote = Self::names(&["vote_start"]);
        let mut map: Option<String> = None;
        if main.has("vote_start") || !detect.is_empty() {
            // poll until Vote Start shows; spot the loading screen meanwhile
            let mut dl = self.control.deadline(self.secs(120.0));
            loop {
                self.check()?;
                if map.is_none() && !detect.is_empty() {
                    for p in &detect {
                        if main.find(std::slice::from_ref(p), None).is_some() {
                            let m = map_from_pattern(p);
                            self.log(format!("map: {m}"));
                            map = Some(m);
                            break;
                        }
                    }
                }
                if main.has("vote_start") && main.find(&vote, None).is_some() {
                    break;
                }
                if !main.has("vote_start") && map.is_some() {
                    self.sleep(6000)?;
                    break;
                }
                if dl.expired() {
                    self.log("no Vote Start after 2 min - carrying on");
                    break;
                }
                self.sleep(300)?;
            }
        } else {
            self.sleep(6000)?;
        }
        let map = match (map, fixed) {
            (Some(m), true) if route_map(&m) != route_map(&s.run.map) => {
                self.log(format!(
                    "loaded {m}, expected {} - using {m}",
                    route_map(&s.run.map)
                ));
                m
            }
            (Some(m), _) => m,
            (None, true) => route_map(&s.run.map),
            (None, false) if !prev_map.is_empty() => prev_map.to_string(),
            (None, false) => {
                self.log("map not recognised - staying at spawn, spiral placing (capture its loading screen as loading_<map>)");
                String::new()
            }
        };
        for h in self.main_last(self.movers()) {
            self.arb
                .run(camera_batch(h, &s.camera))
                .map_err(|e| format!("camera: {e}"))?;
        }
        for h in self.main_last(self.movers()) {
            self.tp_spawn(h)?;
        }
        if let Some(r) = (!map.is_empty())
            .then(|| self.routes.find_map(&map))
            .flatten()
        {
            for h in self.main_last(self.movers()) {
                self.log(format!("map walk: {}", r.map));
                let b = r
                    .to_batch(h, self.screen(h), &vars(s))
                    .map_err(|e| e.to_string())?;
                if let Err(e) = self.arb.run(b) {
                    self.log(format!("map walk: {e} - placing from here"));
                }
            }
        }
        // every client votes (alts first), then back on the main
        let all: Vec<Hwnd> = self.clients.iter().map(|c| c.0).collect();
        for h in self.main_last(all) {
            if let Some((_, p)) =
                self.wait_any(h, &["vote_start"], if h == self.main { 3000 } else { 1000 })?
            {
                self.click(h, p, "vote start")?;
            }
        }
        self.to_main()?;
        Ok(map)
    }

    /// Settings > Teleport to Spawn > close (skipped until tp_spawn is captured).
    fn tp_spawn(&self, h: Hwnd) -> R<()> {
        let scr = self.screen(h);
        if !scr.has("tp_spawn") {
            return Ok(());
        }
        let Some(r) = self.routes.lobby.get("tp_spawn") else {
            return Ok(());
        };
        let b = r
            .to_batch(h, scr, &vars(&self.settings))
            .map_err(|e| e.to_string())?;
        if let Err(e) = self.arb.run(b) {
            self.log(format!("teleport to spawn: {e}"));
            // don't leave the settings open
            if let Some(p) = scr.find(&Self::names(&["settings_close"]), None) {
                let _ = self.click(h, p, "close settings");
            }
        }
        Ok(())
    }

    /// Place on every placing client at once. Returns placed (client, slot, spot).
    pub fn place_all(&self, map: &str) -> R<Vec<(Hwnd, usize, Point)>> {
        let s = &self.settings;
        let keys = Keys::from(&s.keybinds)?;
        let map = route_map(map);
        let mut work = vec![];
        for n in s.placers.iter().copied() {
            let Some((h, scr)) = self.clients.get(n.saturating_sub(1)).cloned() else {
                continue;
            };
            let key = spots_key(&map, n);
            // each placing client can use its own team
            let units = s.units_for(n);
            let mut jobs = vec![];
            for (i, u) in units
                .iter()
                .enumerate()
                .filter(|(_, u)| u.enabled && u.place > 0)
            {
                let slot = i + 1;
                let mut spots: Vec<Point> = self
                    .placements
                    .for_slot(&key, slot as u8)
                    .iter()
                    .map(|p| Point::new(p.x, p.y))
                    .collect();
                if s.spiral_fallback {
                    let nth = s.placers.iter().position(|x| *x == n).unwrap_or(0);
                    spots.extend(super::placing::shape_spots_for(
                        s.place_shape,
                        80,
                        n as u64 * 31 + slot as u64,
                        s.shape_scale,
                        nth,
                        s.placers.len(),
                    ));
                }
                // skip spots that failed before on this map (spot memory)
                if s.spot_memory {
                    let mem = self.memory.lock().unwrap();
                    let before = spots.len();
                    spots.retain(|p| !mem.is_bad(&key, *p));
                    if spots.len() < before && slot == 1 {
                        self.log(format!(
                            "{key}: skipping {} spot(s) that failed before",
                            mem.bad_count(&key)
                        ));
                    }
                }
                jobs.push((slot, u.place as u32, spots));
            }
            // farm units first
            jobs.sort_by_key(|(slot, _, _)| !units[slot - 1].farm);
            work.push((n, h, scr, jobs, units));
        }
        let delay = (s.place_delay_ms as f32 * self.speed) as u64;
        let placing: Vec<Hwnd> = work.iter().map(|w| w.1).collect();
        // clients that don't place: the first placer watches their card pop-ups
        let watchers: Vec<Hwnd> = self
            .clients
            .iter()
            .map(|c| c.0)
            .filter(|h| !placing.contains(h))
            .collect();
        // set when VICTORY / DEFEAT shows up while placing
        let over = std::sync::atomic::AtomicBool::new(false);
        let over = &over;
        let out = std::thread::scope(|sc| {
            let mut handles = vec![];
            for (wi, (n, h, scr, jobs, units)) in work.into_iter().enumerate() {
                let keys = keys.clone();
                let map = map.clone();
                let watchers = if wi == 0 { watchers.clone() } else { vec![] };
                handles.push(sc.spawn(move || {
                    // "UPG [0]" badge = the unit really went down
                    let badge = {
                        use crate::movement::RouteEnv;
                        scr.has("unit_placed")
                            .then(|| scr.probe(&["unit_placed".to_string()], None))
                    };
                    // before every placement: stage over? card pop-up?
                    let cards = || {
                        use std::sync::atomic::Ordering;
                        if over.load(Ordering::Relaxed) {
                            return false;
                        }
                        if matches!(
                            self.end_screen(&Self::names(&["victory", "defeat"])),
                            Ok(Some(_))
                        ) {
                            over.store(true, Ordering::Relaxed);
                            self.log("stage ended while placing");
                            return false;
                        }
                        let _ = self.handle_cards(h);
                        for w in &watchers {
                            let _ = self.handle_cards(*w);
                        }
                        true
                    };
                    let mut used: Vec<Point> = vec![];
                    let mut out = vec![];
                    for (slot, count, spots) in jobs {
                        if over.load(std::sync::atomic::Ordering::Relaxed) {
                            break; // VICTORY / DEFEAT: no more placing
                        }
                        let free: Vec<Point> =
                            spots.into_iter().filter(|p| !used.contains(p)).collect();
                        // BUFF units: turn Auto ability on while the panel is open
                        let buff = units[slot - 1].buff;
                        let auto = |_at: Point| {
                            if !buff || !scr.has("auto_ability") {
                                return;
                            }
                            match self.wait_any(h, &["auto_ability"], 1500) {
                                Ok(Some((_, p))) => {
                                    let _ = self.click(h, p, "auto ability");
                                    self.log(format!("client {n}: slot {slot} auto ability on"));
                                }
                                _ => self
                                    .log(format!("client {n}: slot {slot} auto ability not seen")),
                            }
                        };
                        let set = spots_key(&map, n);
                        let remember = s.spot_memory && scr.has("unit_placed");
                        let good = |p: Point| {
                            auto(p);
                            if remember {
                                self.memory.lock().unwrap().placed(&set, p);
                            }
                        };
                        let bad = |p: Point| {
                            if remember {
                                self.memory.lock().unwrap().failed(&set, p);
                            }
                        };
                        match place_slot(
                            &self.arb,
                            h,
                            &keys,
                            slot,
                            count,
                            &free,
                            badge.clone(),
                            buff,
                            delay,
                            &cards,
                            &good,
                            &bad,
                        ) {
                            Ok(done) => {
                                self.log(format!(
                                    "client {n}: slot {slot} placed {}/{count}",
                                    done.len()
                                ));
                                used.extend(done.iter().copied());
                                out.extend(done.into_iter().map(|p| (h, slot, p)));
                            }
                            Err(e) => {
                                self.log(format!("client {n}: placing stopped - {e}"));
                                break;
                            }
                        }
                    }
                    out
                }));
            }
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect::<Vec<_>>()
        });
        if !watchers.is_empty() {
            self.to_main()?;
        }
        Ok(out)
    }

    /// Upgrade by priority until VICTORY / DEFEAT shows. Returns true on a win.
    pub fn upgrade_until_end(&self, placed: &[(Hwnd, usize, Point)]) -> R<bool> {
        let s = &self.settings;
        let keys = Keys::from(&s.keybinds)?;
        // unit settings per client (each placing client may have its own team)
        let per: std::collections::HashMap<Hwnd, Vec<crate::config::UnitSlot>> = placed
            .iter()
            .map(|(h, _, _)| {
                let n = self.clients.iter().position(|c| c.0 == *h).unwrap_or(0) + 1;
                (*h, s.units_for(n))
            })
            .collect();
        let unit = |h: &Hwnd, slot: usize| per[h][slot - 1].clone();
        let mut order: Vec<&(Hwnd, usize, Point)> = placed
            .iter()
            .filter(|(h, slot, _)| unit(h, *slot).upgrade != Upgrade::None)
            .collect();
        order.sort_by_key(|(h, slot, _)| (!unit(h, *slot).farm, unit(h, *slot).priority));
        let mut done: Vec<u32> = vec![0; order.len()];
        let mut maxed: Vec<bool> = vec![false; order.len()];
        let mut last_idle = Instant::now();
        let end = Self::names(&["victory", "defeat"]);
        let mut dl = self.control.deadline(self.secs(60.0 * 40.0));
        let mut i = 0usize;
        loop {
            self.check()?;
            if let Some(won) = self.end_screen(&end)? {
                return Ok(won);
            }
            self.cards_all()?;
            if dl.expired() {
                return Err("no end screen after 40 min".into());
            }
            if order.is_empty() {
                self.idle_click(&mut last_idle, &keys)?;
                self.sleep(1000)?;
                continue;
            }
            // skip the ones already finished; all finished = just watch the end
            let finished = |k: usize, done: &[u32], maxed: &[bool]| {
                let (h, slot, _) = order[k];
                maxed[k]
                    || match unit(h, *slot).upgrade {
                        Upgrade::Level(n) => done[k] >= n as u32,
                        _ => false,
                    }
            };
            if (0..order.len()).all(|k| finished(k, &done, &maxed)) {
                // everything upgraded: keep the PC and Roblox busy until the end
                self.idle_click(&mut last_idle, &keys)?;
                self.sleep(1000)?;
                continue;
            }
            while finished(i % order.len(), &done, &maxed) {
                i += 1;
            }
            let k = i % order.len();
            let (h, slot, at) = order[k];
            let scr = self.screen(*h);
            // open the unit
            self.arb
                .run(Batch::new(*h, "open unit").click(at.x, at.y).wait(300))
                .map_err(|e| format!("upgrade: {e}"))?;
            let is_max =
                scr.has("upgrade_max") && scr.find(&Self::names(&["upgrade_max"]), None).is_some();
            let before = Self::read_level(scr);
            let reached = match (unit(h, *slot).upgrade, before) {
                (Upgrade::Level(n), Some(l)) => l >= n as u32,
                _ => false,
            };
            if is_max {
                maxed[k] = true;
                self.log(format!("slot {slot} at {},{} is MAX", at.x, at.y));
            } else if reached {
                done[k] = before.unwrap_or(0);
                self.log(format!(
                    "slot {slot} at {},{} reached its level",
                    at.x, at.y
                ));
            } else {
                self.arb
                    .run(
                        Batch::new(*h, "upgrade")
                            .then(crate::input::Action::KeyTap {
                                key: keys.upgrade,
                                hold_ms: 40,
                            })
                            .wait(150),
                    )
                    .map_err(|e| format!("upgrade: {e}"))?;
                self.sleep(500)?;
                let after = Self::read_level(scr);
                match (before, after) {
                    // verified: the level text went up
                    (_, Some(u32::MAX)) => {
                        maxed[k] = true;
                        self.log(format!("slot {slot} at {},{} is MAX", at.x, at.y));
                    }
                    (Some(b), Some(a)) if a > b => {
                        done[k] = a;
                        self.log(format!("slot {slot} at {},{} upgraded to {a}", at.x, at.y));
                    }
                    // same level = not enough money yet; doesn't count
                    (Some(_), Some(_)) => {}
                    (None, Some(a)) => done[k] = a,
                    // level text not captured / not readable: count the press
                    _ => done[k] += 1,
                }
            }
            self.arb
                .run(super::placing::cancel_batch(*h, &keys))
                .map_err(|e| format!("upgrade: {e}"))?;
            if !(s.run.priority_upgrade && !finished(k, &done, &maxed)) {
                i += 1; // priority upgrade keeps on this one until it's done
            }
            self.sleep(400)?;
        }
    }

    /// Every 5 s once there's nothing left to upgrade: a click on an empty
    /// spot (then Cancel, in case it opened a unit) on every client, so the PC
    /// doesn't idle into sleep and Roblox doesn't kick for AFK.
    fn idle_click(&self, last: &mut Instant, keys: &Keys) -> R<()> {
        if !self.settings.idle_click || last.elapsed() < Duration::from_secs_f64(self.secs(5.0)) {
            return Ok(());
        }
        *last = Instant::now();
        let (x, y) = self.settings.idle_click_at;
        let all: Vec<Hwnd> = self.clients.iter().map(|c| c.0).collect();
        for h in self.main_last(all) {
            self.arb
                .run(
                    Batch::new(h, "idle click")
                        .click(x, y)
                        .wait(150)
                        .then(Action::KeyTap {
                            key: keys.cancel,
                            hold_ms: 40,
                        }),
                )
                .map_err(|e| format!("idle click: {e}"))?;
        }
        Ok(())
    }

    /// The unit panel's level ("UPG [n]"): Windows OCR first (nothing to
    /// capture), else your upg_1 .. upg_10 patterns (unit_placed = 0).
    /// Some(u32::MAX) = MAX.
    fn read_level(scr: &super::LiveScreen) -> Option<u32> {
        let f = scr.frame()?;
        match scr.ocr_level(&f) {
            Some(Ok(n)) => return Some(n),
            Some(Err(())) => return Some(u32::MAX),
            None => {}
        }
        let mut names: Vec<(u32, String)> = (1..=10).map(|n| (n, format!("upg_{n}"))).collect();
        names.push((0, "unit_placed".into()));
        // highest first so "UPG [10]" isn't read as "UPG [1]"
        names.sort_by(|a, b| b.0.cmp(&a.0));
        names
            .into_iter()
            .filter(|(_, n)| scr.has(n))
            .find(|(_, n)| scr.find_in_frame(&f, n))
            .map(|(l, _)| l)
    }

    /// VICTORY / DEFEAT on any client (main first) - an alt can see the
    /// end first, then everyone goes to the end screen handling.
    fn end_screen(&self, end: &[String]) -> R<Option<bool>> {
        let mut order: Vec<Hwnd> = vec![self.main];
        order.extend(self.clients.iter().map(|c| c.0).filter(|h| *h != self.main));
        for h in order {
            let scr = self.screen(h);
            for n in end {
                if scr.find(std::slice::from_ref(n), None).is_some() {
                    return Ok(Some(n == "victory"));
                }
            }
        }
        Ok(None)
    }

    /// Next -> Replay or Return to lobby. Returns true when back in the lobby.
    pub fn after_end(&self) -> R<bool> {
        // straight to the main: spam Next until Replay shows, then Replay
        self.to_main()?;
        let main = self.screen(self.main);
        let lobby = self.settings.run.return_to_lobby;
        let want = if lobby { "return_to_lobby" } else { "replay" };
        let title = Self::names(&["victory", "defeat"]);
        let next = Self::names(&["next_button"]);
        let end = Self::names(&[
            "victory",
            "defeat",
            "next_button",
            "replay",
            "return_to_lobby",
        ]);
        let mut dl = self.control.deadline(self.secs(90.0));
        let waited = Instant::now();
        let mut clicked = false;
        let mut blind = 0u32;
        loop {
            self.check()?;
            // the button we want: click it, then make sure the end screen goes
            if let Some(p) = main.find_wide(&Self::names(&[want])) {
                self.click(self.main, p, want)?;
                clicked = true;
                let mut gone = self.control.deadline(self.secs(8.0));
                while main.find_wide(&end).is_some() && !gone.expired() {
                    self.sleep(400)?;
                }
                if main.find_wide(&end).is_none() {
                    break;
                }
                self.log(format!("{want} didn't take - clicking again"));
                continue;
            }
            // Next, wherever it is on this end screen
            if let Some(p) = main.find_wide(&next) {
                self.arb
                    .run(Batch::new(self.main, "next").click(p.x, p.y))
                    .map_err(|e| format!("next: {e}"))?;
                self.sleep(600)?;
                continue;
            }
            // VICTORY / DEFEAT but no Next seen: click where Next sits under
            // the title (spam it - one of them lands)
            if let Some(t) = main.find_wide(&title) {
                let p = Point::new(t.x, (t.y + self.settings.next_below_title).min(590));
                if blind % 5 == 0 {
                    self.log(format!(
                        "Next not recognised - clicking under the title at {},{}",
                        p.x, p.y
                    ));
                }
                blind += 1;
                self.arb
                    .run(Batch::new(self.main, "next (blind)").click(p.x, p.y))
                    .map_err(|e| format!("next: {e}"))?;
                self.sleep(800)?;
                continue;
            }
            // already left the end screen (e.g. the click took on the first try)
            if clicked && main.find_wide(&end).is_none() {
                break;
            }
            // no Replay on this end screen (some modes only offer the lobby)
            if !lobby && waited.elapsed() > Duration::from_secs_f64(self.secs(15.0)) {
                if let Some(p) = main.find_wide(&Self::names(&["return_to_lobby"])) {
                    self.log("no Replay here - going back to the lobby instead");
                    self.click(self.main, p, "return to lobby")?;
                    self.sleep(8000)?;
                    return Ok(true);
                }
            }
            if dl.expired() {
                return Err(format!("end screen: '{want}' never showed up"));
            }
            self.sleep(400)?;
        }
        self.log(if lobby {
            "back to the lobby"
        } else {
            "replaying"
        });
        if lobby {
            self.sleep(8000)?;
        }
        Ok(lobby)
    }

    /// What's on the main's screen right now, to get back on track after a
    /// hiccup: "end", "lobby", "stage" or "".
    fn where_am_i(&self) -> &'static str {
        let main = self.screen(self.main);
        if main
            .find(
                &Self::names(&["victory", "defeat", "next_button", "replay"]),
                None,
            )
            .is_some()
        {
            "end"
        } else if main
            .find(&Self::names(&["play_button", "areas_button"]), None)
            .is_some()
        {
            "lobby"
        } else if main.find(&Self::names(&["vote_start"]), None).is_some() {
            "stage"
        } else {
            ""
        }
    }

    /// Every few minutes: a few clicks (+ Cancel) on each alt, so Roblox's
    /// 20-minute idle kick never hits an alt that's just standing there.
    fn anti_afk(&self, finished: &std::sync::atomic::AtomicBool) {
        use std::sync::atomic::Ordering;
        let every = Duration::from_secs_f64(self.secs(self.settings.afk_every_min as f64 * 60.0));
        if self.settings.afk_every_min == 0 {
            return;
        }
        let Ok(keys) = Keys::from(&self.settings.keybinds) else {
            return;
        };
        let mut last = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(200));
            if finished.load(Ordering::Relaxed) || self.control.is_stopped() {
                return;
            }
            if self.control.is_paused() || last.elapsed() < every {
                continue;
            }
            last = Instant::now();
            let (x, y) = self.settings.idle_click_at;
            let alts: Vec<Hwnd> = self
                .clients
                .iter()
                .map(|c| c.0)
                .filter(|h| *h != self.main)
                .collect();
            for h in alts {
                let mut b = Batch::new(h, "anti-AFK");
                for _ in 0..5 {
                    b = b.click(x, y).wait(150);
                }
                b = b.then(Action::KeyTap {
                    key: keys.cancel,
                    hold_ms: 40,
                });
                match self.arb.run(b) {
                    Ok(_) => self.log("anti-AFK: clicked on an alt"),
                    Err(e) => self.log(format!("anti-AFK: {e}")),
                }
            }
        }
    }

    /// One stage: (lobby ->) setup -> place -> upgrade -> end -> Replay / lobby.
    fn one_round(&self, in_lobby: &mut bool, map: &mut String) -> R<()> {
        if *in_lobby {
            self.enter_stage()?;
            map.clear();
        }
        *map = self.stage_setup(map)?;
        self.cards_all()?;
        let started = Instant::now();
        let placed = self.place_all(map)?;
        self.log(format!(
            "placed {} unit(s) in {:.0}s",
            placed.len(),
            started.elapsed().as_secs_f32()
        ));
        let won = self.upgrade_until_end(&placed)?;
        let _ = self.tx.send(if won { Event::Win } else { Event::Loss });
        if !won && self.settings.run.stop_after_defeat {
            return Err("stopped after a defeat".into());
        }
        *in_lobby = self.after_end()?;
        Ok(())
    }

    /// The whole loop until Stop / an error.
    pub fn run(&self) {
        let finished = std::sync::atomic::AtomicBool::new(false);
        let res = std::thread::scope(|sc| {
            // alts that sit idle get kicked after 20 min: poke them every few minutes
            sc.spawn(|| self.anti_afk(&finished));
            let r = (|| -> R<()> {
                let mut in_lobby = true;
                let mut map = String::new();
                if self.settings.run.own_team
                    && self.settings.run.party_invite
                    && self.clients.len() > 1
                {
                    self.party_up()?;
                }
                let mut fails = 0;
                let mut blocked_told = false;
                loop {
                    match self.one_round(&mut in_lobby, &mut map) {
                        Ok(()) => {
                            fails = 0;
                            blocked_told = false;
                        }
                        Err(e) if e == "stopped" || e.starts_with("stopped after") => {
                            return Err(e)
                        }
                        Err(e) if e.contains("input blocked") => {
                            // locked PC / screen off: not the game's fault - wait, don't give up
                            if !blocked_told {
                                let _ = self.tx.send(Event::Attention(
                                "Input is blocked (PC locked, screen off or a UAC prompt) - the macro waits and carries on once it's unlocked".into(),
                            ));
                                blocked_told = true;
                            }
                            self.sleep(10_000)?;
                            let at = self.where_am_i();
                            match at {
                                "end" => in_lobby = self.after_end().unwrap_or(in_lobby),
                                "lobby" => in_lobby = true,
                                _ => in_lobby = false,
                            }
                        }
                        Err(e) => {
                            fails += 1;
                            if fails > 3 {
                                return Err(format!("{e} (3 tries in a row)"));
                            }
                            // find out where we are and carry on from there
                            let at = self.where_am_i();
                            self.log(format!(
                                "hiccup: {e} - on screen: {} - carrying on",
                                if at.is_empty() { "unknown" } else { at }
                            ));
                            match at {
                                "end" => in_lobby = self.after_end().unwrap_or(in_lobby),
                                "lobby" => in_lobby = true,
                                _ => {
                                    in_lobby = false;
                                    self.sleep(3000)?;
                                }
                            }
                        }
                    }
                    self.sleep(500)?;
                }
            })();
            finished.store(true, std::sync::atomic::Ordering::Relaxed);
            r
        });
        let _ = self.tx.send(Event::Done(match res {
            Ok(()) => "finished".into(),
            Err(e) => e,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn plan_per_mode() {
        let routes = Routes::load(Path::new("/nonexistent"));
        let mut s = Settings::default();
        s.units[0].enabled = true;
        s.run.mode = Mode::SummerHunt;
        let p = plan(&s, &routes);
        let all: Vec<String> = p.iter().flat_map(|x| x.needs.clone()).collect();
        for n in [
            "play_button",
            "join_matchmaking",
            "play_here",
            "open_portal",
            "open_button",
            "start_button",
            "victory",
            "next_button",
            "replay",
        ] {
            assert!(all.iter().any(|a| a == n), "summer needs {n}");
        }
        s.run.own_team = true;
        s.run.party_invite = false; // no party: alts walk into the portal
        let all: Vec<String> = plan(&s, &routes)
            .iter()
            .flat_map(|x| x.needs.clone())
            .collect();
        assert!(
            all.iter().any(|a| a == "open_friends_only")
                && all.iter().any(|a| a == "portal_join_e")
        );

        s.run.mode = Mode::Legend;
        s.run.map = "Ruined City (Midnight)".into();
        s.run.act = "Legend Stage 3".into();
        s.run.return_to_lobby = true;
        let p = plan(&s, &routes);
        let all: Vec<String> = p.iter().flat_map(|x| x.needs.clone()).collect();
        for n in [
            "areas_button",
            "play_area_button",
            "select_world",
            "world_ruined_city_midnight",
            "act_legend_stage_3",
            "select_button",
            "return_to_lobby",
        ] {
            assert!(all.iter().any(|a| a == n), "legend needs {n}");
        }
        assert!(p.iter().any(|x| x.label == "Map walk: Ruined City"));
        assert!(!all.iter().any(|a| a == "open_portal"));
    }

    #[test]
    fn route_map_strips_variant() {
        assert_eq!(route_map("Ruined City (Midnight)"), "Ruined City");
        assert_eq!(route_map("Space Center"), "Space Center");
    }
}
