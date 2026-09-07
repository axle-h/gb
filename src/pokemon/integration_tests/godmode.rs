//! **C2** — a run through `LlmPolicy` that cannot lose, and what one of its turns costs.
//!
//! `docs/coverage-plan.md` §4. The stack is [`LlmRun`]'s — the real worker, the real policy, the
//! real agent, the real wire — and the two things that make a whole game affordable are:
//!
//! - a **god party** installed by the [`Cheats`] sidecar between ticks, so no battle is ever lost
//!   and no black-out ever costs a re-walk;
//! - a **battle script** installed through the real `set_battle_script` tool, so a battle turn is
//!   decided on the emulator thread and costs no request at all.
//!
//! ⚠️ **The story is played rather than skipped.** Nothing here writes an event flag, and the
//! starter is chosen by walking into Oak's lab and answering the game. See `cheats`' module note for
//! why a save produced any other way makes every finding from it a false positive.
//!
//! ## What is here, and what is not
//!
//! ⚠️ **The god run to the Hall of Fame is not built.** What is here is the machinery under it —
//! [`Intent`], [`ScriptedBrain`], the cheat sidecar and the driver — and the **measurement** §4.2
//! says has to come first: *"The turn cost is the unknown and must be measured before this is
//! committed to… The first thing C2 produces is a number: milliseconds per overworld turn, and turns
//! to the Hall of Fame. If it lands above about ten minutes it goes behind its own feature and
//! `full_playthrough` stays as the gate."* [`godmode_turn_cost`] is that number.
//!
//! The step that remains is the intent list: `PolicyStep::complete_game_steps()` has variants with
//! no menu row behind them at all — `UseBagItem`, `Fish`, `UsePcBox`, `UseItemsInBattle` — and §4.1
//! says that where a step does not map, *that is the finding*. Working through them one at a time is
//! the rest of C2, and each one is either a gap in `llm::prompt` or an intent this file has to grow.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::pokemon::integration_tests::cheats::Cheats;
use crate::pokemon::integration_tests::llm_harness::{
    Brain, Call, LlmRun, Reply, TurnRequest,
};

/// One thing the run means to do next, resolved against the **rendered action menu** and nothing
/// else.
///
/// ⚠️ **Every variant has to be answerable from the strings in a [`TurnRequest`].** That is not a
/// limitation of this type, it is the point of it: a brain that reached around to `GameState` for a
/// destination would prove the agent works and say nothing about whether a model could have found
/// the same row. Where an intent cannot be resolved, [`ScriptedBrain`] records the situation and the
/// menu it was looking at, and the run fails naming both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Take the one transition on this map that leads to `map`, and keep taking transitions until
    /// the `Location:` line says we are there.
    ///
    /// ⚠️ **One hop, not a route.** A menu is one map's worth of rows, so an intent that named a map
    /// three maps away could not be resolved from it — which is exactly why
    /// `PolicyStep::complete_game_steps` is written as a chain of `enter(...)` hops rather than as
    /// `goto`. The correspondence is one for one, deliberately.
    Enter(&'static str),
    /// Choose the row whose id ends in `:{0}` — a person's name, `Pc`, `CutTree`, `Grass`.
    Row(&'static str),
    /// Choose the first row whose *description* contains `{0}`. For the rows an id does not
    /// distinguish, which is every warp and connection.
    Says(&'static str),
    /// Nothing to do: end the turn without moving. Used to pad a measurement.
    Wait,
}

impl Intent {
    /// Whether the situation says this intent is already satisfied, so the list can move on without
    /// spending a turn.
    fn satisfied_by(&self, request: &TurnRequest) -> bool {
        match self {
            Self::Enter(map) => request.location().as_deref() == Some(map),
            // Nothing else can be told from a situation alone: a person talked to leaves no mark on
            // the next turn's rendering, so those are one intent per turn by construction.
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
                // A connection row says "go to ViridianCity"; a warp row says "take the warp to
                // OaksLab, arriving at (12, 12)". Matching the map name in the prose covers both,
                // and matching on the *word* rather than a substring keeps `Route1` off `Route11`.
                .find(|(_, description)| names_map(description, map))
                .map(|(id, _)| id.clone()),
            Self::Row(kind) => rows
                .iter()
                .find(|(id, _)| id.rsplit(':').next() == Some(*kind))
                .map(|(id, _)| id.clone()),
            Self::Says(fragment) => rows
                .iter()
                .find(|(_, description)| description.contains(fragment))
                .map(|(id, _)| id.clone()),
        }
    }
}

/// Whether `description` names exactly this map, rather than one whose name starts the same way.
///
/// ⚠️ `Route1` is a prefix of `Route11`, `Route12` and eight more, and `CeruleanCity` is a prefix of
/// nothing but `PewterCity` is a suffix of nothing either — a bare `contains` gets one of these
/// wrong and the run then walks confidently to the wrong map.
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
    ///
    /// ⚠️ **Through the real `set_battle_script` tool**, so the seven made-up battles and the arming
    /// path are covered rather than bypassed — `docs/coverage-plan.md` §2.3's fourth seam.
    script: Option<&'static str>,
    armed: bool,
    /// What went wrong, if the list could not be carried out: the intent, and the menu it was
    /// looking at. ⚠️ **This is the deliverable of a failed god run**, not a panic — §4.1: a step
    /// that knows something the menu does not say is a gap in `llm::prompt`.
    pub stuck: Arc<Mutex<Option<String>>>,
    /// How many requests this brain has answered, and how many of them were battle turns. A battle
    /// turn reaching here at all means the script did not decide it.
    pub turns: Arc<Mutex<(usize, usize)>>,
}

impl ScriptedBrain {
    pub fn new(intents: Vec<Intent>) -> Self {
        Self {
            intents,
            at: 0,
            script: Some(crate::llm::battle_script::DETERMINISTIC),
            armed: false,
            stuck: Arc::new(Mutex::new(None)),
            turns: Arc::new(Mutex::new((0, 0))),
        }
    }

    /// Play without a battle script, so every battle turn is a paid request. The comparison that
    /// says what the script is worth.
    pub fn without_a_script(mut self) -> Self {
        self.script = None;
        self.armed = true;
        self
    }

    pub fn finished(&self) -> bool {
        self.at >= self.intents.len()
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
            // The script should have decided this; if the turn reached here, take the first attack.
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
        // Every other kind — a nickname, a mart, a move to forget — is answered with the game's own
        // default, because none of them is what this brain is for.
        if !request.has_tool("choose_action") {
            return default_for(request);
        }

        // ⚠️ **Armed on the first overworld turn, as a read tool, so the turn still ends in a
        // decision.** `set_battle_script` is non-terminal; pairing it with the action below is one
        // request rather than two, and it is also the shape a model would use.
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
                // ⚠️ **`resume_after_battle`, because a wild encounter says nothing about the walk.**
                // Without it every patch of grass on the route costs an extra overworld turn to
                // re-issue a walk that was going to be re-issued word for word.
                calls.push(Call::new(
                    "choose_action",
                    serde_json::json!({ "id": id, "resume_after_battle": true }),
                ));
                // A `Row`/`Says` intent is done the moment it is chosen; an `Enter` is done when the
                // location says so, which the loop above checks on the next turn.
                if !matches!(intent, Intent::Enter(_)) {
                    self.at += 1;
                }
                Reply::Calls(calls)
            }
            None => {
                // ⚠️ **Recorded rather than panicked, and the menu goes with it.** The panic would
                // happen on the mock's own thread, where it is a hung test rather than a failure.
                let mut stuck = self.stuck.lock().expect("not poisoned");
                if stuck.is_none() {
                    *stuck = Some(format!(
                        "intent {} of {} ({:?}) has no row in the menu on {}:\n{}",
                        self.at + 1,
                        self.intents.len(),
                        intent,
                        request.location().unwrap_or_else(|| "(nowhere)".into()),
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

/// Pallet Town to Pewter City, as an intent list — the same hops
/// `early_game::can_navigate_to_pewter_city` makes with `PolicyStep::enter`, one for one.
///
/// It starts inside the Viridian Mart, which is a warp out before anything else.
fn viridian_to_pewter() -> Vec<Intent> {
    vec![
        Intent::Enter("ViridianCity"),
        Intent::Enter("Route2"),
        Intent::Enter("ViridianForestSouthGate"),
        Intent::Enter("ViridianForest"),
        Intent::Enter("ViridianForestNorthGate"),
        Intent::Enter("Route2"),
        Intent::Enter("PewterCity"),
    ]
}

/// ⚠️ **§4.2's number, and it is the gate on the rest of C2.** How long one turn of the deployed
/// stack takes against a localhost mock, and how many turns a stretch of route costs.
///
/// A round trip to a mock plus a prompt build over a growing history is not free, and the history
/// grows until compaction bounds it — so `turns × ms/turn` is the whole of what a god run would
/// cost, and `full_playthrough` is retired only if that lands under about ten minutes.
///
/// It prints rather than asserting a threshold: the number is the deliverable, and a wall-clock
/// assertion in a test tier is a flake on a loaded machine. What it *does* assert is that the run
/// arrived, so a measurement of a run that did not happen cannot be quoted.
#[test]
#[cfg(feature = "godmode")]
fn godmode_turn_cost() {
    let brain = ScriptedBrain::new(viridian_to_pewter());
    let (stuck, turns) = (Arc::clone(&brain.stuck), Arc::clone(&brain.turns));

    let mut run = LlmRun::builder(include_bytes!("../data/viridian-city-pokemart-shopping.bin"))
        .named("godmode-cost")
        .game_time(Duration::from_mins(20))
        .with_coverage()
        .start(Box::new(brain));
    run.with_cheats(Cheats::default());

    let started = Instant::now();
    let arrived = run.tick_until(Duration::from_secs(300), |run| {
        run.map() == crate::pokemon::map::Map::PewterCity
    });
    let elapsed = started.elapsed();

    if let Some(why) = stuck.lock().expect("not poisoned").clone() {
        panic!("the scripted brain could not resolve an intent from the menu it was sent.\n{why}");
    }
    assert!(arrived, "the run never reached Pewter City in {elapsed:?}");

    let (all, battles) = *turns.lock().expect("not poisoned");
    let latencies = run.turn_latencies_ms();
    let mean = latencies.iter().sum::<u64>() as f64 / latencies.len().max(1) as f64;
    let worst = latencies.iter().copied().max().unwrap_or(0);
    let game_time = run.fixture().total_cycles.to_duration();
    let turns_per_game_minute = all as f64 / (game_time.as_secs_f64() / 60.0).max(0.001);
    println!(
        "\n════ godmode turn cost: {} ════\n\
         requests           {all} ({battles} of them battle turns)\n\
         game time          {game_time:?}\n\
         wall clock         {elapsed:?}  ({:.0}x realtime)\n\
         turn latency       {mean:.0} ms mean, {worst} ms worst, over {} completed turns\n\
         turn rate          {turns_per_game_minute:.1} per game-minute\n\
         thinking : running {:.2}  (>1 means the endpoint is the bound, not the emulator)\n\
         coverage           {}\n\
         history            {} messages at the end\n\
         \n\
         §4.2's projection: a god run of G game-minutes costs about\n\
         max(G x 60 / {:.0}, G x {turns_per_game_minute:.1} x {mean:.0} / 1000) seconds of wall clock.\n\
         WARNING: the latency above is a LOWER BOUND. It is measured over a history of a handful of\n\
         messages; a prompt is rebuilt from the whole conversation every turn, so this term grows\n\
         until compaction bounds it. Re-measure on a run long enough to compact before quoting it.\n",
        "Viridian Mart to Pewter City",
        game_time.as_secs_f64() / elapsed.as_secs_f64().max(0.001),
        latencies.len(),
        (all as f64 * mean / 1000.0) / elapsed.as_secs_f64().max(0.001),
        run.coverage().map(|log| log.summary()).unwrap_or_default(),
        run.messages_last_sent(),
        game_time.as_secs_f64() / elapsed.as_secs_f64().max(0.001),
    );

    // ⚠️ **The battle script is what makes the number affordable**, so a run in which it silently
    // failed to arm must not be quoted as one in which it worked. Every battle on this stretch is a
    // wild encounter in Viridian Forest and the script decides all of them.
    assert_eq!(battles, 0, "{battles} battle turns reached the model; the script did not decide them");

    // And the run is honest: the agent carried out everything it was asked to.
    let log = run.coverage().expect("coverage was asked for");
    let hard: Vec<String> = log
        .entries()
        .filter(|entry| {
            matches!(entry.verdict, crate::pokemon::integration_tests::coverage::Verdict::Defect { .. })
        })
        .map(|entry| entry.id.clone())
        .collect();
    assert!(hard.is_empty(), "the agent could not execute what the model chose: {hard:?}");
    assert!(log.watchdog.is_empty(), "the watchdog fired: {:?}", log.watchdog);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The resolver reads the menu and nothing else, and it does not confuse `Route1` with `Route11`.
    #[test]
    fn an_intent_is_resolved_against_the_rendered_menu() {
        let request = request_with(
            "Location: ViridianCity at (23, 25), facing Down\n\
             \n\
             - `ViridianCity:33,20:Connection` — go to Route2, arriving at (10, 71)\n\
             - `ViridianCity:23,26:Warp` — take the warp to ViridianMart, arriving at (3, 7)\n\
             - `ViridianCity:21,25:OldMan` — talk to Old Man\n",
        );

        assert_eq!(request.location().as_deref(), Some("ViridianCity"));
        assert_eq!(
            Intent::Enter("Route2").resolve(&request).as_deref(),
            Some("ViridianCity:33,20:Connection"),
        );
        assert_eq!(
            Intent::Row("OldMan").resolve(&request).as_deref(),
            Some("ViridianCity:21,25:OldMan"),
        );
        assert_eq!(
            Intent::Says("ViridianMart").resolve(&request).as_deref(),
            Some("ViridianCity:23,26:Warp"),
        );
        // Not offered: recorded as a finding rather than resolved to something near enough.
        assert_eq!(Intent::Enter("PewterCity").resolve(&request), None);
        // ⚠️ And a prefix is not a match. `Route1` must not take the connection to `Route2`, and on a
        // map that offers both, `Route1` must not take `Route11`.
        assert_eq!(Intent::Enter("Route1").resolve(&request), None);

        assert!(Intent::Enter("ViridianCity").satisfied_by(&request), "we are already there");
        assert!(!Intent::Enter("Route2").satisfied_by(&request));
    }

    /// A situation with no `Location:` line — a battle, a naming screen — has no location, and an
    /// `Enter` intent is then simply not satisfied rather than matching an empty string.
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
            system: String::new(),
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
