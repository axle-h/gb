use std::sync::{Arc, Mutex};
#[cfg(feature = "slow-tests")]
use std::time::{Duration, Instant};

#[cfg(feature = "slow-tests")]
use crate::pokemon::integration_tests::cheats::Cheats;
#[cfg(feature = "slow-tests")]
use crate::pokemon::integration_tests::llm_harness::LlmRun;
use crate::pokemon::integration_tests::llm_harness::{Brain, Call, Reply, TurnRequest};

/// One thing the run means to do next, resolved against the rendered action menu alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Take transitions toward `map` until the `Location:` line says the player is there.
    Enter(&'static str),
    /// Choose the row whose id ends in `:{0}` — a person's name, `Pc`, `CutTree`, `Grass`.
    Row(&'static str),
    /// Choose the first row whose *description* contains `{0}`.
    Says(&'static str),
    /// Choose the row whose description contains `{0}` until the menu stops offering it.
    Repeat(&'static str),
    /// Nothing to do: end the turn without moving.
    Wait,
}

impl Intent {
    /// Whether the situation says this intent is already satisfied, so no turn is spent.
    fn satisfied_by(&self, request: &TurnRequest) -> bool {
        match self {
            Self::Enter(map) => request.location().as_deref() == Some(map),
            Self::Repeat(fragment) => !request.menu_rows().iter()
                .any(|(_, description)| description.contains(fragment)),
            // A situation cannot show that a person was talked to, so these are one intent per
            // turn.
            Self::Row(_) | Self::Says(_) | Self::Wait => false,
        }
    }

    /// The id that carries this intent out, or `None` if the menu does not offer one.
    fn resolve(&self, request: &TurnRequest) -> Option<String> {
        let rows = request.menu_rows();
        match self {
            Self::Wait => None,
            Self::Enter(map) => rows
                .iter()
                // A connection row says "go to ViridianCity"; a warp row "take the warp to OaksLab,
                // arriving at (12, 12)".
                .find(|(_, description)| names_map(description, map))
                .map(|(id, _)| id.clone()),
            Self::Row(kind) => rows
                .iter()
                .find(|(id, _)| id.rsplit(':').next() == Some(*kind))
                .map(|(id, _)| id.clone()),
            Self::Says(fragment) | Self::Repeat(fragment) => rows
                .iter()
                .find(|(_, description)| description.contains(fragment))
                .map(|(id, _)| id.clone()),
        }
    }
}

/// Whether `description` names exactly this map, rather than one whose name starts the same way.
fn names_map(description: &str, map: &str) -> bool {
    description
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word == map)
}

/// A brain that carries out an intent list, and nothing else.
pub struct ScriptedBrain {
    intents: Vec<Intent>,
    at: usize,
    /// The battle script's source, installed on the first overworld turn and then left alone.
    script: Option<&'static str>,
    armed: bool,
    pub stuck: Arc<Mutex<Option<String>>>,
    /// Consecutive turns the current intent has had no row to resolve against.
    unresolved: usize,
    /// How many times the current [`Intent::Repeat`] has been re-issued.
    reissued: usize,
    /// Requests answered, and how many were battle turns, each of which the script failed to
    /// decide.
    pub turns: Arc<Mutex<(usize, usize)>>,
}

impl ScriptedBrain {
    /// Turns an intent may find nothing before the run is called stuck.
    const PATIENCE: usize = 20;

    /// How many times an [`Intent::Repeat`] may be re-issued before the run is called stuck.
    const MAX_REISSUES: usize = 12;

    pub fn new(intents: Vec<Intent>) -> Self {
        Self {
            intents,
            at: 0,
            script: Some(crate::llm::battle_script::DETERMINISTIC),
            armed: false,
            unresolved: 0,
            reissued: 0,
            stuck: Arc::new(Mutex::new(None)),
            turns: Arc::new(Mutex::new((0, 0))),
        }
    }

}

impl Brain for ScriptedBrain {
    fn respond(&mut self, request: &TurnRequest) -> Reply {
        if request.is_summary() {
            return Reply::Content("Playing a scripted route with a party that cannot lose.".into());
        }
        {
            let mut turns = self.turns.lock().expect("not poisoned");
            turns.0 += 1;
            if request.is_battle() {
                turns.1 += 1;
            }
        }

        if request.is_battle() {
            // The script should have decided this; take the first attack.
            let id = request
                .menu_ids()
                .into_iter()
                .find(|id| id.starts_with("fight:"))
                .unwrap_or_else(|| "run".to_string());
            return Reply::call("choose_battle_action", serde_json::json!({ "id": id }));
        }
        if request.is_stuck() {
            return Reply::call(
                "press_buttons",
                serde_json::json!({ "buttons": ["a"], "why": "the agent reached no decision point" }),
            );
        }
        // Every other kind is answered with the game's own default.
        if !request.has_tool("choose_action") {
            return default_for(request);
        }

        // Armed on the first overworld turn as a read tool, so the turn still ends in a decision.
        let mut calls = Vec::new();
        if let Some(script) = self.script.filter(|_| !self.armed) {
            self.armed = true;
            calls.push(Call::new(
                "set_battle_script",
                serde_json::json!({
                    "script": script,
                    "purpose": "win every battle with the strongest move the lead knows",
                }),
            ));
        }

        while self.at < self.intents.len() && self.intents[self.at].satisfied_by(request) {
            self.at += 1;
        }
        let Some(intent) = self.intents.get(self.at).cloned() else {
            calls.push(Call::wait(50));
            return Reply::Calls(calls);
        };
        if intent == Intent::Wait {
            self.at += 1;
            calls.push(Call::wait(1));
            return Reply::Calls(calls);
        }

        match intent.resolve(request) {
            Some(id) => {
                self.unresolved = 0;
                // `resume_after_battle`, because a wild encounter says nothing about the walk.
                calls.push(Call::new(
                    "choose_action",
                    serde_json::json!({ "id": id, "resume_after_battle": true }),
                ));
                // A `Row` or `Says` is done once chosen; `Enter` and `Repeat` are checked by the
                // loop above next turn.
                if !matches!(intent, Intent::Enter(_) | Intent::Repeat(_)) {
                    self.at += 1;
                }
                if matches!(intent, Intent::Repeat(_)) {
                    self.reissued += 1;
                    if self.reissued > Self::MAX_REISSUES {
                        let mut stuck = self.stuck.lock().expect("not poisoned");
                        if stuck.is_none() {
                            *stuck = Some(format!(
                                "intent {} of {} ({:?}) was re-issued {} times and its row is still \
                                 on the menu, so the agent is not finishing it",
                                self.at + 1, self.intents.len(), intent, self.reissued));
                        }
                    }
                } else {
                    self.reissued = 0;
                }
                Reply::Calls(calls)
            }
            None => {
                // Give it [`Self::PATIENCE`] turns before believing the row is absent.
                self.unresolved += 1;
                if self.unresolved < Self::PATIENCE {
                    calls.push(Call::wait(10));
                    return Reply::Calls(calls);
                }
                // Recorded rather than panicked, with the menu.
                let mut stuck = self.stuck.lock().expect("not poisoned");
                if stuck.is_none() {
                    *stuck = Some(format!(
                        "intent {} of {} ({:?}) had no row in the menu on {} for {} turns:\n{}",
                        self.at + 1,
                        self.intents.len(),
                        intent,
                        request.location().unwrap_or_else(|| "(nowhere)".into()),
                        Self::PATIENCE,
                        request
                            .menu_rows()
                            .iter()
                            .map(|(id, description)| format!("  - `{id}` — {description}"))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ));
                }
                calls.push(Call::wait(50));
                Reply::Calls(calls)
            }
        }
    }
}

/// The answer that keeps the game moving on a kind this brain has no opinion about.
fn default_for(request: &TurnRequest) -> Reply {
    for (name, arguments) in [
        ("set_nickname", serde_json::json!({})),
        ("forget_move", serde_json::json!({})),
        ("buy_item", serde_json::json!({})),
    ] {
        if request.has_tool(name) {
            return Reply::call(name, arguments);
        }
    }
    Reply::Calls(vec![Call::wait(1)])
}

#[cfg(feature = "slow-tests")]
fn pallet_to_the_hall_of_fame() -> Vec<Intent> {
    vec![
        // ── Out of the bedroom and into the lab ──
        Intent::Enter("RedsHouse1F"),
        Intent::Enter("PalletTown"),
        // `Says`, not `Enter`: the one hop where the difference matters.
        Intent::Says("Route1"),
        Intent::Row("SquirtlePokeBall"),
        // Oak's parcel errand is not on this route; the Pokédex gates nothing west.
        Intent::Enter("PalletTown"),
        Intent::Enter("Route1"),
        Intent::Enter("ViridianCity"),
        // ── West out of Viridian: the badge gates ──
        Intent::Enter("Route22"),
        Intent::Enter("Route22Gate"),
        // The guard is a conversation, and it flips the gate's dynamic warp north.
        Intent::Row("Guard"),
        Intent::Enter("Route23"),
        Intent::Enter("VictoryRoad1F"),
        // ── Victory Road: four Strength goals and a hole, across three floors ──
        Intent::Repeat("switch at (17, 13)"),
        Intent::Enter("VictoryRoad2F"),
        Intent::Repeat("switch at (1, 16)"),
        Intent::Enter("VictoryRoad3F"),
        Intent::Repeat("switch at (3, 5)"),
        Intent::Repeat("hole at (23, 15)"),
        // Down the east ladder, onto the side the revealed boulder is on.
        Intent::Says("VictoryRoad2F, arriving at (22, 16)"),
        Intent::Repeat("switch at (9, 16)"),
        // Both of these name their landing, and must.
        Intent::Says("VictoryRoad3F, arriving at (27, 15)"),
        Intent::Says("VictoryRoad2F, arriving at (27, 7)"),
        Intent::Enter("Route23"),
        Intent::Enter("IndigoPlateau"),
        Intent::Enter("IndigoPlateauLobby"),
        // ── The gauntlet ──
        Intent::Row("Nurse"),
        Intent::Enter("LoreleisRoom"),
        Intent::Row("Lorelei"),
        Intent::Enter("BrunosRoom"),
        Intent::Row("Bruno"),
        Intent::Enter("AgathasRoom"),
        Intent::Row("Agatha"),
        Intent::Enter("LancesRoom"),
        Intent::Row("Lance"),
        // Nothing after this door.
        Intent::Enter("ChampionsRoom"),
    ]
}

#[test]
#[cfg(feature = "slow-tests")]
fn godmode_run() {
    play_the_god_run(crate::pokemon::options::HEADLESS_OPTIONS);
}

/// [`godmode_run`] with battle animations on: the `LlmPolicy` configuration that is deployed.
#[test]
#[cfg(feature = "slow-tests")]
fn godmode_run_animated() {
    play_the_god_run(crate::pokemon::options::SERVED_OPTIONS);
}

#[cfg(feature = "slow-tests")]
fn play_the_god_run(options: crate::pokemon::options::GameOptions) {
    let brain = ScriptedBrain::new(pallet_to_the_hall_of_fame());
    let (stuck, turns) = (Arc::clone(&brain.stuck), Arc::clone(&brain.turns));

    let mut run = LlmRun::builder(include_bytes!("../data/start-of-game-state.bin"))
        .named("godmode-run")
        .game_time(Duration::from_mins(400))
        .options(options)
        .with_coverage()
        .start(Box::new(brain));
    // Badges and a party that cannot lose, applied between ticks.
    run.with_cheats(Cheats::default());

    let started = Instant::now();
    let reached = run.tick_until(Duration::from_secs(1800), |run| {
        run.map_if_readable() == Some(crate::pokemon::map::Map::HallOfFame)
            || stuck.lock().expect("not poisoned").is_some()
    });
    let elapsed = started.elapsed();

    // The stuck report first, the useful failure.
    if let Some(why) = stuck.lock().expect("not poisoned").clone() {
        let live = match run.fixture().try_game_state() {
            Ok(state) => format!(
                "the agent's own view: {} @ {}, strength={} boulders {:?}\n  {}",
                state.map.map, state.map.player_position, state.map.can_strength,
                state.map.boulders(),
                state.map.actions().iter()
                    .map(|a| format!("{} — {}", a.id(), a.tile))
                    .collect::<Vec<_>>().join("\n  ")),
            Err(why) => format!("the agent has no readable state: {why}"),
        };
        panic!("the god run could not resolve an intent from the menu it was sent.\n{why}\n\n{live}");
    }

    let (all, battles) = *turns.lock().expect("not poisoned");
    let game_time = run.fixture().total_cycles.to_duration();
    let latencies = run.turn_latencies_ms();
    let mean = latencies.iter().sum::<u64>() as f64 / latencies.len().max(1) as f64;
    println!(
        "\n════ the god run: Pallet Town to the Hall of Fame ════\n\
         played {game_time:?} of game time in {elapsed:?} of wall clock ({:.0}x realtime)\n\
         requests           {all} ({battles} of them battle turns)\n\
         turn latency       {mean:.0} ms mean over {} completed turns\n\
         coverage           {}\n",
        game_time.as_secs_f64() / elapsed.as_secs_f64().max(0.001),
        latencies.len(),
        run.coverage().map(|log| log.summary()).unwrap_or_default(),
    );

    assert!(reached, "the run never reached the Hall of Fame in {elapsed:?}");
    assert_eq!(battles, 0,
        "{battles} battle turns reached the model; the battle script did not decide them");
    let log = run.coverage().expect("coverage was asked for");
    let hard: Vec<String> = log.entries()
        .filter(|entry| matches!(entry.verdict,
            crate::pokemon::integration_tests::coverage::Verdict::Defect { .. }))
        .map(|entry| entry.id.clone())
        .collect();
    assert!(hard.is_empty(), "the agent could not execute what the model chose: {hard:?}");
    assert!(log.watchdog.is_empty(), "the watchdog fired: {:?}", log.watchdog);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The resolver reads only the menu, and does not confuse `Route1` with `Route11`.
    #[test]
    fn an_intent_is_resolved_against_the_rendered_menu() {
        let request = request_with(
            "Location: ViridianCity at (23, 25), facing Down\n\
             \n\
             - `ViridianCity:33,20:Connection` — go to Route2, arriving at (10, 71)\n\
             - `ViridianCity:23,26:Warp` — take the warp to ViridianMart, arriving at (3, 7)\n\
             - `ViridianCity:OldMan` — talk to Old Man\n",
        );

        assert_eq!(request.location().as_deref(), Some("ViridianCity"));
        assert_eq!(
            Intent::Enter("Route2").resolve(&request).as_deref(),
            Some("ViridianCity:33,20:Connection"),
        );
        assert_eq!(
            Intent::Row("OldMan").resolve(&request).as_deref(),
            Some("ViridianCity:OldMan"),
        );
        assert_eq!(
            Intent::Says("ViridianMart").resolve(&request).as_deref(),
            Some("ViridianCity:23,26:Warp"),
        );
        // Not offered: a finding, not something near enough.
        assert_eq!(Intent::Enter("PewterCity").resolve(&request), None);
        // A prefix is not a match: `Route1` takes neither `Route2`'s connection nor `Route11`.
        assert_eq!(Intent::Enter("Route1").resolve(&request), None);

        assert!(Intent::Enter("ViridianCity").satisfied_by(&request), "we are already there");
        assert!(!Intent::Enter("Route2").satisfied_by(&request));

        // A `Repeat` resolves to its row and is done only once that row stops being offered.
        assert_eq!(
            Intent::Repeat("Old Man").resolve(&request).as_deref(),
            Some("ViridianCity:OldMan"),
        );
        assert!(!Intent::Repeat("Old Man").satisfied_by(&request));
        assert!(Intent::Repeat("Nurse").satisfied_by(&request), "no such row: nothing left to repeat");
    }

    /// A situation with no `Location:` line satisfies no `Enter` intent.
    #[test]
    fn a_turn_with_no_location_line_satisfies_nothing() {
        let request = request_with("### Battle\nA wild PIDGEY appeared!\n");
        assert_eq!(request.location(), None);
        assert!(!Intent::Enter("PewterCity").satisfied_by(&request));
        assert_eq!(Intent::Enter("PewterCity").resolve(&request), None);
    }

    fn request_with(situation: &str) -> TurnRequest {
        use crate::pokemon::integration_tests::llm_harness::SeenMessage;
        TurnRequest {
            messages: vec![SeenMessage {
                role: "user".to_string(),
                text: situation.to_string(),
                images: Vec::new(),
            }],
            tools: Vec::new(),
            seen: 0,
        }
    }
}
