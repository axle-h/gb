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
#[cfg(feature = "slow-tests")]
use std::time::{Duration, Instant};

#[cfg(feature = "slow-tests")]
use crate::pokemon::integration_tests::cheats::Cheats;
#[cfg(feature = "slow-tests")]
use crate::pokemon::integration_tests::llm_harness::LlmRun;
use crate::pokemon::integration_tests::llm_harness::{Brain, Call, Reply, TurnRequest};

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
    /// ⭐ **Choose the row whose description contains `{0}`, and keep choosing it until the menu
    /// stops offering it.**
    ///
    /// ⚠️ **[`Self::Says`] is one-shot, and a chosen action is not a finished one.** The god run
    /// failed on its second pass for exactly that: Victory Road 3F's hole goal was interrupted by a
    /// wild battle **six** times, `resume_after_battle` gives up after five, and the intent had
    /// already advanced on the turn it was chosen — so the list moved on to the next switch with the
    /// boulder still sitting on the floor, and the run then waited for a row the cartridge had no
    /// reason to mint. Nothing was wrong with the agent; the brain had confused *asking* with
    /// *arriving*.
    ///
    /// A boulder goal is the row this is for, because it is the one kind that both takes minutes and
    /// disappears when it is done: `actions()` withholds a `PushBoulderOntoSwitch` once a boulder is
    /// standing on the switch, so "gone from the menu" is exactly "finished". A person is the
    /// counter-example — talking to one leaves the row where it was — which is why this is a variant
    /// rather than the behaviour of `Says`.
    Repeat(&'static str),
    /// Nothing to do: end the turn without moving. Used to pad a measurement.
    Wait,
}

impl Intent {
    /// Whether the situation says this intent is already satisfied, so the list can move on without
    /// spending a turn.
    fn satisfied_by(&self, request: &TurnRequest) -> bool {
        match self {
            Self::Enter(map) => request.location().as_deref() == Some(map),
            // Done when the row is no longer on offer. See the variant.
            Self::Repeat(fragment) => !request.menu_rows().iter()
                .any(|(_, description)| description.contains(fragment)),
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
            Self::Says(fragment) | Self::Repeat(fragment) => rows
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
    /// Consecutive turns the current intent has had no row to resolve against. See
    /// [`Self::PATIENCE`].
    unresolved: usize,
    /// How many times the current [`Intent::Repeat`] has been re-issued. See [`Self::MAX_REISSUES`].
    reissued: usize,
    /// How many requests this brain has answered, and how many of them were battle turns. A battle
    /// turn reaching here at all means the script did not decide it.
    pub turns: Arc<Mutex<(usize, usize)>>,
}

impl ScriptedBrain {
    /// ⭐ **Turns an intent may find nothing before the run is called stuck.**
    ///
    /// ⚠️ **The first draft gave up on the first turn, and a row that is not there *this tick* is
    /// not a row that is missing.** The menu is minted from live state, so it moves: a person
    /// standing on a doormat withholds the door until they step off (`a_door_with_somebody_standing
    /// _in_it_is_not_a_row_until_they_move`), a route lost to a wandering pet is waited out for
    /// `MAX_ROUTE_LOST_TICKS` before the agent will even say `NoRoute`, and a map's sprite table is
    /// incomplete for a couple of dozen ticks after a warp. The agent is patient about all three and
    /// a model would simply look again, so a brain that panics on the first miss is measuring its
    /// own impatience.
    ///
    /// Twenty turns is well past every one of those and still far inside the run's wall clock.
    const PATIENCE: usize = 20;

    /// How many times an [`Intent::Repeat`] may be re-issued before the run is called stuck.
    ///
    /// A boulder goal is re-issued once per interruption, and Victory Road's floors are thick with
    /// wild encounters: the god run's hole goal was interrupted six times in one pass. Twelve is
    /// twice that and still fails long before the wall clock does.
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
                self.unresolved = 0;
                // ⚠️ **`resume_after_battle`, because a wild encounter says nothing about the walk.**
                // Without it every patch of grass on the route costs an extra overworld turn to
                // re-issue a walk that was going to be re-issued word for word.
                calls.push(Call::new(
                    "choose_action",
                    serde_json::json!({ "id": id, "resume_after_battle": true }),
                ));
                // A `Row`/`Says` intent is done the moment it is chosen; an `Enter` is done when the
                // location says so and a `Repeat` when its row stops being offered, both of which
                // the loop above checks on the next turn.
                if !matches!(intent, Intent::Enter(_) | Intent::Repeat(_)) {
                    self.at += 1;
                }
                // ⚠️ **A `Repeat` that is still being re-issued after this many turns is a loop.**
                // The row it names is meant to disappear when the work is done, so one that keeps
                // coming back is either the wrong row or a goal the agent cannot finish — and a
                // brain that re-issues for ever turns that into a hung test rather than a failure.
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
                // Give it [`Self::PATIENCE`] turns before believing the row is really absent.
                self.unresolved += 1;
                if self.unresolved < Self::PATIENCE {
                    calls.push(Call::wait(10));
                    return Reply::Calls(calls);
                }
                // ⚠️ **Recorded rather than panicked, and the menu goes with it.** The panic would
                // happen on the mock's own thread, where it is a hung test rather than a failure.
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
/// ⭐ **Pallet Town to the Hall of Fame, as an intent list** — `docs/coverage-plan.md` step 2.
///
/// ⚠️ **It does not play the scripted route, and the reason is in the cartridge rather than in
/// taste.** `PolicyStep::complete_game_steps` walks the whole of Kanto because it has to *earn* what
/// it needs: eight badges, HM01 through HM04, the Silph Scope, the Poké Flute. A god run is handed
/// the badges and a party that already knows every field move, so the only question left is which
/// gates read the **badge byte** and which read an **event flag** — and the two gates between
/// Viridian and the Elite Four both read the byte:
///
/// - `Route22GateGuardText` is `ld a, [wObtainedBadges] / bit BIT_BOULDERBADGE`;
/// - `Route23CheckForBadgeScript` is `ld hl, wObtainedBadges` for all seven of its guards, and the
///   `EVENT_PASSED_*_CHECK` flags it also touches are only a memo that the guard has already asked.
///
/// So `Cheats`' badges open the whole west road, and the run is Pallet → Viridian → Route 22 →
/// Route 23 → Victory Road → the Elite Four. ⚠️ **Pewter is the counter-example and is why the
/// coverage walk starts from a finished game instead**: its east exit is held by a Youngster who
/// reads the *event flag* for having beaten Brock, which no cheat here sets.
///
/// ⚠️ **The story is still played, not skipped.** The run walks into Oak's lab and answers the game
/// for its starter, fights the rival, and solves both of Victory Road's Strength floors with the
/// same `PushBoulderOntoSwitch` rows a model is offered. Nothing writes an event flag.
fn pallet_to_the_hall_of_fame() -> Vec<Intent> {
    vec![
        // ── Out of the bedroom and into the lab ──
        Intent::Enter("RedsHouse1F"),
        Intent::Enter("PalletTown"),
        // ⚠️ **`Says`, not `Enter`, and this is the one hop where the difference matters.** Choosing
        // the way north is what makes Oak run out and drag the player to his lab, so the player
        // never arrives on Route 1 and an `Enter("Route1")` would wait for a location that is not
        // coming. A `Says` intent is done the moment its row is chosen, which is the honest shape
        // for "walk at the thing that interrupts you".
        Intent::Says("Route1"),
        Intent::Row("SquirtlePokeBall"),
        // Oak's parcel errand is not on this route: the Pokédex is not a gate on anything west.
        Intent::Enter("PalletTown"),
        Intent::Enter("Route1"),
        Intent::Enter("ViridianCity"),
        // ── West out of Viridian: the badge gates ──
        Intent::Enter("Route22"),
        Intent::Enter("Route22Gate"),
        // The guard is a *conversation*, and it is what flips the gate's dynamic warp north.
        Intent::Row("Guard"),
        Intent::Enter("Route23"),
        Intent::Enter("VictoryRoad1F"),
        // ── Victory Road: four Strength goals and a hole, across three floors ──
        //
        // ⚠️ **Every one of these is a `Says` rather than a `Row`, and the first draft's `Row`s are
        // why.** A boulder goal is one decision per *target*, so VictoryRoad2F offers **two**
        // `PushBoulderOntoSwitch` rows and VictoryRoad3F offers **two** ways down to
        // VictoryRoad2F — and `Row(kind)`/`Enter(map)` both take whichever sorts first. Measured:
        // the run solved (17, 13), (1, 16), (3, 5) and the hole correctly, then re-chose the (1, 16)
        // switch it had already pressed and spent the rest of its budget going up and down between
        // VR3F (2, 0) and VR2F (1, 1).
        //
        // ⭐ **The menu already says which is which**, which is the useful half of that finding: a
        // goal names its target square and a warp names its landing, so a model choosing off the
        // same rows can tell them apart. `enter_at` exists in `PolicyStep` for exactly this and has
        // no equivalent here because it does not need one — the prose is the discriminator.
        Intent::Repeat("switch at (17, 13)"),
        Intent::Enter("VictoryRoad2F"),
        Intent::Repeat("switch at (1, 16)"),
        Intent::Enter("VictoryRoad3F"),
        Intent::Repeat("switch at (3, 5)"),
        Intent::Repeat("hole at (23, 15)"),
        // Down the east ladder, onto the side the revealed boulder is on.
        Intent::Says("VictoryRoad2F, arriving at (22, 16)"),
        Intent::Repeat("switch at (9, 16)"),
        // ⚠️ **Both of these name their landing, and both had to.** Victory Road's top two floors are
        // joined by **four** ladder pairs, and which one you take decides which pocket you are in:
        //
        // | up from VR2F | lands on VR3F | down from VR3F | lands on VR2F |
        // |---|---|---|---|
        // | (1, 1) | (2, 0) west | (2, 0) | (1, 1) west |
        // | (23, 7) | (23, 7) east | (23, 7) | (23, 7) east |
        // | (27, 7) | (26, 8) | (26, 8) | **(27, 7) — the exit pocket** |
        // | (25, 14) | (27, 15) | (27, 15) | (25, 14) dead end |
        //
        // Only the (26, 8) ladder lands in the pocket the Route 23 exit at (29, 7) is in, and the way
        // to *reach* (26, 8) is the (25, 14) ladder — not the (23, 7) one the trip in used, which
        // lands in a pocket (26, 8) cannot be reached from at all.
        //
        // ⚠️ **`Enter(map)` cannot express any of this and `PolicyStep::enter` can, which is the one
        // place the two are not one for one.** `enter_map_action` picks the **nearest** matching row
        // by route length; the rendered menu carries no step counts, so `Intent::Enter` takes the
        // first match in menu order and lands wherever that happens to be. What the menu *does*
        // carry is the landing and a side ("this one is on the east side of the map"), so naming it
        // is both the fix here and the evidence that a model has enough to choose correctly.
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
        // ⚠️ **Nothing after this door.** The rival's script fires on entry and starts the battle
        // with no overworld tick in between, so an intent placed after it is never reached — the
        // same fact `elite_four_steps` records about where the Elixer has to go.
        Intent::Enter("ChampionsRoom"),
    ]
}

/// ⭐ **The god run: a fresh save at Pallet Town played to the Hall of Fame through `LlmPolicy`,
/// the worker and the wire** — `docs/coverage-plan.md` step 2, and the thing that whole step was
/// about.
///
/// Everything a deployed model does, this does: the same policy, the same turn loop, the same tool
/// catalogue, the same agent, over a real game from the title save to the credits. What it is
/// *not* is the scripted route — see [`pallet_to_the_hall_of_fame`] for which gates that lets it
/// skip and why the cartridge allows it.
///
/// ⚠️ **The battle script is what makes it affordable and the run asserts that it worked.** Every
/// battle on this route — the rival in Oak's lab, Victory Road's trainers, the wild encounters in
/// its grass, and all twenty-six Pokémon of the Elite Four — is decided on the emulator thread by
/// the program the brain installs on its first overworld turn. A battle turn reaching the brain at
/// all means the script did not decide it, and that is a failure rather than a slow run.
///
/// It prints its game time, wall clock and rate in the same shape `full_playthrough` does, because
/// the two are meant to be read side by side and the bar is "measured on the same machine on the
/// same day".
#[test]
#[cfg(feature = "slow-tests")]
fn godmode_run() {
    let brain = ScriptedBrain::new(pallet_to_the_hall_of_fame());
    let (stuck, turns) = (Arc::clone(&brain.stuck), Arc::clone(&brain.turns));

    let mut run = LlmRun::builder(include_bytes!("../data/start-of-game-state.bin"))
        .named("godmode-run")
        .game_time(Duration::from_mins(400))
        .with_coverage()
        .start(Box::new(brain));
    // Badges and a party that cannot lose, applied between ticks. Nothing here writes an event flag.
    run.with_cheats(Cheats::default());

    let started = Instant::now();
    let reached = run.tick_until(Duration::from_secs(1800), |run| {
        run.map_if_readable() == Some(crate::pokemon::map::Map::HallOfFame)
            || stuck.lock().expect("not poisoned").is_some()
    });
    let elapsed = started.elapsed();

    // ⚠️ **The stuck report first, because it is the useful failure.** §4.1: an intent the rendered
    // menu cannot answer is a gap in `llm::prompt`, and the menu it was looking at is the evidence.
    //
    // ⭐ **And `actions()` beside it, because the menu alone cannot tell the two failures apart.**
    // A row the agent never minted is an agent or a map-layer question; a row it minted and the
    // prompt did not show is a `llm::tools` question, and they are fixed in different files. The
    // first draft printed only the menu and cost three runs guessing which of the two it was
    // looking at.
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

    /// The resolver reads the menu and nothing else, and it does not confuse `Route1` with `Route11`.
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
        // Not offered: recorded as a finding rather than resolved to something near enough.
        assert_eq!(Intent::Enter("PewterCity").resolve(&request), None);
        // ⚠️ And a prefix is not a match. `Route1` must not take the connection to `Route2`, and on a
        // map that offers both, `Route1` must not take `Route11`.
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
