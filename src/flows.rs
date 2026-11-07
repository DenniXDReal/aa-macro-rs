//! What each game flow needs to see. The Patterns page shows these as a
//! checklist so you capture exactly the names the game loop looks for.

pub struct Need {
    pub name: &'static str,
    pub kind: &'static str, // "text" or "icon"
    pub hint: &'static str,
}

pub struct Flow {
    pub name: &'static str,
    pub steps: &'static str,
    pub needs: &'static [Need],
}

const fn n(name: &'static str, kind: &'static str, hint: &'static str) -> Need {
    Need { name, kind, hint }
}

pub const FLOWS: &[Flow] = &[
    Flow {
        name: "Lobby",
        steps: "Lobby buttons used by every mode.",
        needs: &[
            n("play_button", "icon", "Left menu: the Play button (controller icon + 'Play')"),
            n("areas_button", "icon", "Right menu: the Areas button"),
        ],
    },
    Flow {
        name: "Stages (Story / Legend / Infinite / Raid)",
        steps: "Areas > Play Area (or Raids) > walk into the room until 'Select World' > Main/Legend toggle > scroll to your world > act > Select > Join Matchmaking > Play Here > Start. Your own world + act names are under 'Your stage'.",
        needs: &[
            n("play_area_button", "text", "Areas pop-up: the 'Play Area' tile title"),
            n("raids_button", "text", "Areas pop-up: the 'Raids' tile title"),
            n("select_world", "text", "World list title 'Select World'"),
            n("legend_stages_toggle", "text", "Orange 'Legend Stages' button above the list (shows on the Main list)"),
            n("main_stages_toggle", "text", "Blue 'Main Stages' button above the list (shows on the Legend list)"),
            n("select_button", "text", "Green 'Select' after picking an act"),
            n("find_match", "text", "'Find Match' under the magnifier (only if you use Find Match)"),
        ],
    },
    Flow {
        name: "Party (alts)",
        steps: "All clients in your private server > main clicks the party + once > waits for Invite buttons > clicks every Invite until none are left (they turn Invited) > swaps to each alt and clicks Accept > goes to the game mode.",
        needs: &[
            n("party_plus", "icon", "Main: the green + under '@you's Party' (top right)"),
            n("party_invite", "text", "Main: yellow 'Invite' next to an alt's name"),
            n("party_invited", "text", "Main: grey 'Invited' (invite already sent)"),
            n("party_invite_popup", "text", "Alt: 'Party Invite' title of the pop-up"),
            n("party_accept", "text", "Alt: green 'Accept'"),
            n("party_close", "icon", "Main: the button that closes the party menu once everyone joined"),
        ],
    },
    Flow {
        name: "Summer Hunt",
        steps: "Play > hold Shift+W into the Summer Hunt entrance > Play Here > Open (or Open (Friends Only) for a party host) > Start. Find Match = public matchmaking.",
        needs: &[
            n("join_matchmaking", "text", "Dialog title 'Join Matchmaking?'"),
            n("play_here", "text", "'Play Here' under the green controller"),
            n("open_portal", "text", "Dialog title 'Open Portal?'"),
            n("open_button", "text", "Green 'Open' button"),
            n("open_friends_only", "text", "Yellow 'Open (Friends Only)' button (party host)"),
            n("waiting_for_players", "text", "Portal lobby: 'Waiting for players...'"),
            n("start_button", "text", "Portal lobby: green 'Start'"),
            n("portal_join_e", "text", "Follower: the 'Join' prompt with the E key at the portal"),
            n("choose_card", "text", "Card pick pop-up title ('Choose a card...') - the macro takes the green card, else the middle one"),
            n("unit_close", "icon", "The X that closes a unit's panel - clicked before the card screenshot if a panel is open"),
            n("card_confirm", "text", "'Confirm' under the cards after picking one"),
        ],
    },
    Flow {
        name: "Infinity Castle / Challenge",
        steps: "Walk into the pod (from the AHK macro) until its Cancel button shows > Castle: Normal or Hard > Start (your start_button, or the spot just above Cancel).",
        needs: &[n("mode_cancel", "text", "Red 'Cancel' at the bottom once you're in the castle / challenge pod")],
    },
    Flow {
        name: "In stage",
        steps: "Vote Start shows > camera top-down > Settings > Teleport to Spawn > close > map walk > Vote Start > place units (1-6) > upgrade with R by priority > end screen. After Replay it waits for Vote Start and does it all again.",
        needs: &[
            n("vote_start", "text", "Vote Start button at the start of a stage"),
            n("settings_button", "icon", "Settings cog (bottom left) - optional, it clicks (26, 570) without it"),
            n("tp_spawn", "text", "The words 'Teleport back to Spawn' in settings (F6 on the text) - used to find the row"),
            n("tp_spawn_button", "icon", "The arrow button you click for Teleport back to Spawn (where to click; without it the macro clicks 178 px right of the label)"),
            n("settings_close", "icon", "X that closes settings - optional, it clicks (583, 147) without it"),
            n("unit_placed", "text", "'UPG [0]' badge that shows on a unit you just placed"),
            n("upgrade_max", "text", "'MAX' on a unit's panel when it can't upgrade any more (for MAX units)"),
            n("auto_ability", "text", "Auto ability button on a unit's panel while it's OFF - clicked whenever it shows (placing and upgrading)"),
        ],
    },
    Flow {
        name: "Upgrade levels (optional)",
        steps: "Not needed: the macro reads 'UPG [n]' and MAX with Windows' built-in OCR (test it with Placement > Read unit level). Only capture these if OCR misreads your level text: 'UPG [1]' as upg_1, 'UPG [2]' as upg_2 ...",
        needs: &[
            n("upg_1", "text", "'UPG [1]' on the unit panel"),
            n("upg_2", "text", "'UPG [2]'"),
            n("upg_3", "text", "'UPG [3]'"),
            n("upg_4", "text", "'UPG [4]'"),
            n("upg_5", "text", "'UPG [5]'"),
            n("upg_6", "text", "'UPG [6]'"),
            n("upg_7", "text", "'UPG [7]'"),
            n("upg_8", "text", "'UPG [8]'"),
        ],
    },
    Flow {
        name: "End screen",
        steps: "VICTORY / DEFEAT > Next > Replay (or Return to lobby).",
        needs: &[
            n("victory", "text", "Big 'VICTORY' title"),
            n("defeat", "text", "Big red 'DEFEAT' title"),
            n("next_button", "text", "Green 'Next' under the rewards"),
            n("replay", "text", "Yellow 'Replay'"),
            n("return_to_lobby", "text", "Green 'Return to lobby'"),
            n("disconnected", "text", "Roblox 'Disconnected' / 'Reconnect' dialog"),
        ],
    },
];

/// Map detection: one `loading_<map>` pattern per map you play, taken from
/// its loading screen (the map name / a stand-out part). Needed for random
/// map modes like the Summer Event; optional for Story / Legend / Raid.
pub fn map_needs(maps: &[String]) -> Vec<(String, String)> {
    maps.iter()
        .map(|m| {
            (
                crate::game::run::loading_pattern(m),
                format!("'{m}' while it loads (map name on the loading screen)"),
            )
        })
        .collect()
}

pub fn all_names() -> impl Iterator<Item = &'static str> {
    FLOWS.iter().flat_map(|f| f.needs.iter().map(|n| n.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_already_normalised() {
        let mut seen = std::collections::HashSet::new();
        for n in all_names() {
            assert!(seen.insert(n), "duplicate {n}");
            assert_eq!(crate::vision::key(n), n, "{n} isn't a normalised key");
        }
    }
}

/// Patterns for the world + act chosen on Home (Story / Legend / Raid).
pub fn stage_needs(mode: crate::config::Mode, map: &str, act: &str) -> Vec<(String, String)> {
    use crate::config::Mode;
    if !matches!(mode, Mode::Story | Mode::Legend | Mode::Raid) {
        return vec![];
    }
    vec![
        (
            format!("world_{}", crate::vision::key(map)),
            format!("World title '{map}' in the Select World list"),
        ),
        (
            format!("act_{}", crate::vision::key(act)),
            format!("Act button '{act}'"),
        ),
    ]
}
