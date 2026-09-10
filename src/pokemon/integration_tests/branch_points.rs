//! **Step 8 of `docs/coverage-plan.md`** — the branch points, through `LlmPolicy` and the worker.
//!
//! Some of this game is **exclusive per save**. One starter of three, one fossil of two, one of the
//! Fighting Dojo's two Poké Balls, a Bicycle fetched with a voucher or bought without one: taking an
//! arm destroys the others for the life of that cartridge. The coverage walk reaches none of it, and
//! not because of a gate — nine of its ten starts are *finished games*, and a finished game has
//! already spent every one of these choices. So a sweep of Kanto can come back clean having never
//! once watched the Charmander branch run.
//!
//! What that costs is not a dex entry. **Each arm is a different script**, and a script the suite has
//! never run is one nobody has watched the agent drive: the rival's starter and therefore his whole
//! battle is decided here, the fossil branch decides which revival the Cinnabar lab is later asked
//! for, and the bike shop's second arm is a shop rather than a gift, at a price of ¥1,000,000.
//!
//! The shape is what step 8 asks for: **one save state cut before the branch, N tests after it.**
//! Exploration is destructive and one-shot, so an arm costs a resume rather than a replay. The four
//! snapshots are cut by the `regen_*` tests at the bottom of this file, under `--features
//! regen-fixtures`, each from a committed fixture the leg chain already produces.
//!
//! ⚠️ **Through the whole stack, like step 7's.** Every arm runs on [`LlmRun`] — a real socket, the
//! real worker, the real `LlmPolicy`, the real agent — because the question is not whether the
//! *cartridge* has two arms. It is whether a **model** is offered both, in one menu, as rows it can
//! tell apart and choose between. Where it is not, that is the finding — and §5 below is where this
//! step found one: an in-game trade handed over **whatever the party-menu cursor was left on**,
//! because `InGameTrade_DoTrade` opens `DisplayPartyMenu` without resetting `wCurrentMenuItem` and
//! nothing on the deployed path answered it. `PokemonAgent`'s `PartyMenuAnswer` is the fix, and §5's
//! three tests are what it is measured by.
//!
//! ⚠️ **A brain sees strings** (`docs/coverage-plan.md` §3), so every arm below is driven by
//! [`Intent`] against the rendered menu and never by reaching for a `GameState`. An arm that cannot
//! be expressed as an intent is an arm a model could not take either.

#[allow(unused_imports)]
use super::*;

use crate::pokemon::integration_tests::godmode::{Intent, ScriptedBrain};
use crate::pokemon::integration_tests::llm_harness::LlmRun;
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::party::PokemonParty;
use crate::pokemon::pokemon::Pokemon;
use std::sync::{Arc, Mutex};

// ── The snapshots ────────────────────────────────────────────────────────────────────────────────

/// Oak's lab with his speech over and the three Poké Balls still on the table: the starter branch,
/// cut one decision before it. Party empty, no Pokédex, the rival waiting.
const OAKS_LAB: &[u8] = include_bytes!("../data/branch-oaks-lab.bin");

/// Mt Moon B2F standing at the mouth of the fossil chamber, **both** fossils still on the floor and
/// the Super Nerd still guarding them. See [`regen_mt_moon_fossils_fixture`] for why he is left there
/// and [`fossil_arm`] for what each arm pays for him.
const MT_MOON: &[u8] = include_bytes!("../data/branch-mt-moon-fossils.bin");

/// The Fighting Dojo with the Karate Master beaten, both prize balls still there and a party slot
/// free so either lands in the party rather than the box.
const DOJO: &[u8] = include_bytes!("../data/branch-dojo-prize.bin");

/// Inside the Cerulean Bike Shop holding the Bike Voucher, one decision from the clerk.
const BIKE_SHOP: &[u8] = include_bytes!("../data/branch-bike-shop.bin");

/// How long an arm will wait on the **wall** clock before giving up and asserting.
///
/// ⚠️ **Six times `battle_refusals`' figure, and the margin is the point rather than the wait.** An
/// arm that passes never spends it — the whole file runs in under three seconds — but this is a
/// deadline on a real thread doing real emulation, and a loaded machine is not a failure. Measured:
/// the Bulbasaur arm failed once, in a default-tier run that happened to be sharing the box with
/// `hall_of_fame_playthrough`, and passed on the next three. The arms with two sequential waits (a
/// choice, then the script the choice starts) are the ones that get close.
const PATIENCE: Duration = Duration::from_secs(180);

// ── Driving an arm ───────────────────────────────────────────────────────────────────────────────

/// Start `fixture` with a brain that carries out `intents` and nothing else.
///
/// ⚠️ **No cheats.** Every one of these snapshots was played to by the leg chain, so the party that
/// arrives is the party the route earned, and a god party would change which battles the arms fight
/// — which for the starter branch is the entire point of the branch.
fn arm(fixture: &'static [u8], name: &'static str, intents: Vec<Intent>) -> (LlmRun, Stuck) {
    let brain = ScriptedBrain::new(intents);
    let stuck = Arc::clone(&brain.stuck);
    let run = LlmRun::builder(fixture)
        .named(name)
        // ⚠️ **Six game-hours for a decision that takes seconds, and the margin is not slack.** The
        // budget has to outlast [`PATIENCE`], so that an arm which does not happen fails on the
        // assertion naming the missing row rather than on the fixture's cycle budget, which says
        // only that time ran out. What sets the ratio is that **the emulator keeps running while the
        // worker thinks** — that is the property the whole policy is built on — so a mock round trip
        // slowed by a loaded machine is paid for in *emulated* seconds. Measured: at one game-hour
        // this arm passed six times out of six alone and failed roughly one run in three inside the
        // full default tier, where two dozen tests share the box.
        .game_time(Duration::from_secs(6 * 3600))
        // ⚠️ **The watchdog is armed, and every arm asserts it never fired.** A branch that wedges
        // the agent is the failure this file exists to find, and without a stuck timeout the run
        // simply goes quiet — which reads from outside exactly like an arm that finished. Sixty
        // seconds of emulated silence is ten times the longest gap ordinary play has.
        .stuck_timeout(Duration::from_secs(60))
        .start(Box::new(brain));
    (run, stuck)
}

/// What [`ScriptedBrain`] writes when an intent has no row in the menu, and the menu it was looking
/// at. ⚠️ **This is the deliverable of a failed arm rather than a panic** — an intent that cannot be
/// resolved is a branch a model could not have taken either, which is a finding about `llm::prompt`
/// and not about the test.
type Stuck = Arc<Mutex<Option<String>>>;

/// Whether any turn this run put to the model contained `needle`.
///
/// ⚠️ **Read out of the request rather than out of the agent's event stream**, for the reason §3 of
/// the plan gives: a sentence the agent emitted and the prompt did not carry is a sentence no model
/// ever saw.
fn told(run: &LlmRun, needle: &str) -> bool {
    run.endpoint.requests().iter().any(|request| request.situation().contains(needle))
}

/// Whether the watchdog ever had to nudge this run. `true` is always a failure here.
fn was_stuck(run: &LlmRun) -> bool {
    run.endpoint.requests().iter().any(|request| request.is_stuck())
}

/// `stuck`'s message, or a note that the arm simply ran out of game time with the list unfinished.
fn why(stuck: &Stuck) -> String {
    stuck.lock().expect("not poisoned").clone()
        .unwrap_or_else(|| "the intent list resolved but the game never got there".to_string())
}

/// Every row the model was offered on the first overworld turn of a run, as ids.
///
/// ⚠️ **The first one, not the union.** The claim each branch test makes is that all of its arms are
/// on the table *at the same time*, which is what makes the choice a choice; a union over the whole
/// run would also be satisfied by a menu that offered them one after another.
fn first_menu(run: &LlmRun) -> Vec<String> {
    run.endpoint
        .requests()
        .into_iter()
        .find(|request| request.has_tool("choose_action"))
        .map(|request| request.menu_ids())
        .unwrap_or_default()
}

/// Every row the model was offered on the **last** overworld turn of a run.
///
/// ⚠️ **What a branch does to the menu afterwards is not one rule, and finding that out is half of
/// what this file is for.** Mt Moon hides the fossil you leave, so the row goes; the Fighting Dojo
/// hides only the ball you take, so the other row stays and answers a refusal. This helper reads
/// what is on the table *after* the choice; each branch says which of the two it should be. The
/// brain answers `wait` once its intent list is done, so a run always ends on an ordinary overworld
/// turn with a full menu in it.
fn last_menu(run: &LlmRun) -> Vec<String> {
    run.endpoint
        .requests()
        .into_iter()
        .filter(|request| request.has_tool("choose_action"))
        .next_back()
        .map(|request| request.menu_ids())
        .unwrap_or_default()
}

/// Diagnostic: what is a branch snapshot's first turn actually offering?
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_branch_menus() {
    for (name, fixture) in [("oaks-lab", OAKS_LAB), ("mt-moon", MT_MOON), ("dojo", DOJO), ("bike-shop", BIKE_SHOP)] {
        let (mut run, _) = arm(fixture, "probe", vec![Intent::Wait]);
        run.tick_until(Duration::from_secs(20), |run| run.endpoint.requests_served() >= 2);
        println!("== {name}");
        for (n, request) in run.endpoint.requests().into_iter().enumerate() {
            println!("  request {n}: tools {:?} location {:?}", request.tool_names(), request.location());
            for (id, description) in request.menu_rows() {
                println!("     `{id}` — {description}");
            }
        }
    }
}

// ── 1. The starter ───────────────────────────────────────────────────────────────────────────────

/// The lab's table, as the branch: the row, what taking it puts in the party, and what the **rival**
/// then takes.
///
/// ⚠️ **The third column is why this branch is worth three tests rather than one.** The rival always
/// takes the starter that beats yours, so the choice made here decides his whole team for the run and
/// every one of the six battles he is met in. `full_playthrough` has only ever played the Squirtle
/// row; the other two rivals existed nowhere in this suite.
const STARTERS: [(&str, PokemonSpecies, PokemonSpecies); 3] = [
    ("BulbasaurPokeBall", PokemonSpecies::Bulbasaur, PokemonSpecies::Charmander),
    ("CharmanderPokeBall", PokemonSpecies::Charmander, PokemonSpecies::Squirtle),
    ("SquirtlePokeBall", PokemonSpecies::Squirtle, PokemonSpecies::Bulbasaur),
];

/// Wait for `species` to be registered in the Pokédex, and say so if it never is.
///
/// ⚠️ **A separate wait, and not fussiness.** `_AddPartyMon` **increments `wPartyCount` first**
/// (`engine/pokemon/add_mon.asm:15`) and sets the `wPokedexOwned` bit about eighty lines later, so a
/// poll that lands between the two reads a party of one and an empty dex — and every wait in this
/// file polls once per host tick over exactly that write. Asserting the dex on the same sample as
/// the party is a race, and it is one that fails about one default-tier run in five: measured, twice
/// on `the_starter_branch_can_be_taken_to_bulbasaur` and once on the trade.
fn assert_owned(run: &mut LlmRun, species: PokemonSpecies) {
    let owned = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state().is_ok_and(|state| state.pokedex_owned.contains(&species))
    });
    assert!(owned, "{species:?} never reached the Pokédex");
}

/// One arm of the starter branch: take `ball`, and check the game agreed about both halves.
fn starter_arm(name: &'static str, ball: &'static str) {
    let (_, mine, rivals) = STARTERS.iter().copied().find(|(row, _, _)| *row == ball)
        .expect("a row in the table above");
    // ⚠️ **The rival's battle is a *coordinate* trigger on the lab door, and neither waiting for it
    // nor talking to him reaches it.** `OaksLabScript` hands him the counter-starter the moment
    // yours is taken and then stops; talking to him after that answers "GARY: My POKéMON looks a lot
    // stronger." and nothing else — this is Red, not Yellow, and the fight is on the way out. So the
    // arm walks for the door, which is an ordinary menu row, and the script takes it from there.
    // Two earlier versions of this test spent an hour of game time each watching Gary hold a
    // Charmander.
    let (mut run, stuck) = arm(OAKS_LAB, name,
        vec![Intent::Row(ball), Intent::Enter("PalletTown")]);

    // ⚠️ **`try_game_state`, not `game_state`.** `AddPartyMon` writes the species byte before the
    // rest of the struct, so a poll landing inside that window reads "Invalid Pokemon species" — and
    // this is a poll on every host tick over exactly the write it is waiting for.
    let chosen = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state().is_ok_and(|state| state.pokemon.len() == 1)
    });

    // ⭐ **All three at once, before anything is asserted about the one that was taken.** A branch
    // the model cannot see is not a branch: if the table came back as one row, or as a row that did
    // not say which Pokémon was in which ball, every arm below would still pass and the choice would
    // be the agent's rather than the model's.
    let menu = first_menu(&run);
    for (row, _, _) in STARTERS {
        assert!(
            menu.iter().any(|id| id.ends_with(row)),
            "the lab offered no `{row}` row, so a model could not have chosen it.\n  menu: {menu:?}",
        );
    }
    assert!(chosen, "nothing reached the party; the `{ball}` row was never carried out.\n  {}", why(&stuck));

    let state = run.fixture().game_state();
    assert_eq!(state.pokemon[0].species, mine,
        "the `{ball}` row put a {:?} in the party", state.pokemon[0].species);
    assert_owned(&mut run, mine);

    // ⭐ **And the other half of the branch, which is the half that lasts.** `OaksLabScript` gives the
    // rival the starter that beats this one and then puts him in front of you, so this assertion is
    // the whole claim that the *right* arm of the script ran rather than merely an arm of it.
    let fought = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state().is_ok_and(|state| state.battle.is_some())
    });
    let state = run.fixture().game_state();
    let battle = state.battle.as_ref();
    assert!(fought, "the rival never challenged, so the branch's other half never ran");
    assert_eq!(battle.map(|b| b.enemy.species), Some(rivals),
        "the rival took the wrong starter for this arm");
    println!("[{name}] took {mine:?}, the rival took {rivals:?}");
}

/// **The Bulbasaur arm**, which no test in this suite had ever taken.
#[test]
fn the_starter_branch_can_be_taken_to_bulbasaur() {
    starter_arm("branch-starter-bulbasaur", "BulbasaurPokeBall");
}

/// **The Charmander arm.** ⚠️ It is the ball
/// `mechanics::a_pickup_the_game_refuses_says_the_item_is_still_there` presses A on and is *refused*
/// — that fixture is a Squirtle save, where Charmander's is the ball the rival has left behind. So
/// this is the only place in the suite the Charmander row is chosen rather than bounced off.
#[test]
fn the_starter_branch_can_be_taken_to_charmander() {
    starter_arm("branch-starter-charmander", "CharmanderPokeBall");
}

/// **The Squirtle arm** — the one the scripted route takes, and the reason the other two are worth
/// having: everything downstream of `at-cerulean.bin` is a Squirtle save, so this arm is the only one
/// the fixture chain can speak for.
#[test]
fn the_starter_branch_can_be_taken_to_squirtle() {
    starter_arm("branch-starter-squirtle", "SquirtlePokeBall");
}

// ── 2. The fossil ────────────────────────────────────────────────────────────────────────────────

/// Mt Moon B2F's two fossils: the row, and what taking it puts in the bag.
///
/// ⚠️ **The Super Nerd takes the other one**, which is what makes this a branch rather than two item
/// balls: `MtMoonB2FSuperNerdTakesOtherFossilScript` `HideObject`s the fossil the player leaves.
/// Every save in this repo is a Helix save, so `postgame::gifts::can_revive_the_helix_fossil` speaks
/// for one arm of it and nothing has ever spoken for the other.
const FOSSILS: [(&str, ItemId, &str); 2] = [
    ("DomeFossil", ItemId::DomeFossil, "HelixFossil"),
    ("HelixFossil", ItemId::HelixFossil, "DomeFossil"),
];

/// One arm of the fossil branch: take `row`, and check the other one has gone with it.
fn fossil_arm(name: &'static str, row: &'static str) {
    let (_, item, other) = FOSSILS.iter().copied().find(|(id, _, _)| *id == row)
        .expect("a row in the table above");
    let (_, other_item, _) = FOSSILS.iter().copied().find(|(id, _, _)| *id == other)
        .expect("the other row in the table above");
    // ⚠️ **The row is asked for twice, because the Super Nerd interrupts the first attempt.**
    // `MtMoonB2FFossilAreaCoords` fires his battle when the player steps into the chamber — "Hey,
    // stop! I found these fossils! They're both mine!" — and the walk is abandoned on the *text box*
    // rather than on the battle, so `resume_after_battle` does not carry it. The second ask is the
    // one that takes the fossil, and it costs nothing when the first already did: the row has gone
    // by then, so the brain records that it could not resolve and simply waits.
    let (mut run, stuck) = arm(MT_MOON, name, vec![Intent::Row(row), Intent::Row(row)]);

    let taken = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state().is_ok_and(|state| state.bag.iter().any(|i| i.id == item))
    });

    let menu = first_menu(&run);
    for (id, _, _) in FOSSILS {
        assert!(menu.iter().any(|row| row.ends_with(id)),
            "B2F offered no `{id}` row, so a model could not have chosen it.\n  menu: {menu:?}");
    }
    assert!(taken, "the `{row}` row never put a {item:?} in the bag.\n  {}", why(&stuck));

    // ⭐ **The other fossil has to be gone from the menu, and it is the whole branch.** The cartridge
    // hides it the moment this one is taken, so a menu that still offered it would be promising a
    // second fossil this save can never have.
    let cleared = run.tick_until(PATIENCE, |run| !last_menu(run).iter().any(|id| id.ends_with(other)));
    let menu = last_menu(&run);
    assert!(cleared, "`{other}` is still a row after `{row}` was taken.\n  menu: {menu:?}");
    assert!(!run.fixture().game_state().bag.iter().any(|entry| entry.id == other_item),
        "both fossils are in the bag, which no save out of Mt Moon can hold");
    println!("[{name}] took {item:?}; {other} is gone");
}

/// **The Dome Fossil arm** — Kabuto's half of the branch, and the half no save in this repo has.
#[test]
fn the_fossil_branch_can_be_taken_to_the_dome_fossil() {
    fossil_arm("branch-fossil-dome", "DomeFossil");
}

/// **The Helix Fossil arm**, which is the one the scripted route takes and every committed fixture
/// downstream of Mt Moon carries.
#[test]
fn the_fossil_branch_can_be_taken_to_the_helix_fossil() {
    fossil_arm("branch-fossil-helix", "HelixFossil");
}

// ── 3. Hitmonlee or Hitmonchan ───────────────────────────────────────────────────────────────────

/// The Fighting Dojo's two prize balls: the row, what taking it puts in the party, and the other
/// row — which, unlike Mt Moon's, is *still there* afterwards and answers a refusal.
const DOJO_PRIZE: [(&str, PokemonSpecies, &str); 2] = [
    ("HitmonleePokeBall", PokemonSpecies::Hitmonlee, "HitmonchanPokeBall"),
    ("HitmonchanPokeBall", PokemonSpecies::Hitmonchan, "HitmonleePokeBall"),
];

/// What the cartridge says to a player who comes back for the second ball.
///
/// ⚠️ **The opening of the sentence, not the whole of it.** The reader samples the screen once per
/// agent tick and the driver dismisses the box a tick or two later, so a quoted line arrives a
/// character or two short — the same rule `battle_refusals::BALL_FAILED` is written under.
const GREEDY: &str = "Better not get";

/// One arm of the dojo branch: take `row`, then try the other one and be told no.
fn dojo_arm(name: &'static str, row: &'static str) {
    let (_, species, other) = DOJO_PRIZE.iter().copied().find(|(id, _, _)| *id == row)
        .expect("a row in the table above");
    let (_, other_species, _) = DOJO_PRIZE.iter().copied().find(|(id, _, _)| *id == other)
        .expect("the other row in the table above");
    let (mut run, stuck) = arm(DOJO, name, vec![Intent::Row(row), Intent::Row(other)]);

    let got = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state()
            .is_ok_and(|state| state.pokemon.iter().any(|mon| mon.species == species))
    });

    let menu = first_menu(&run);
    for (id, _, _) in DOJO_PRIZE {
        assert!(menu.iter().any(|row| row.ends_with(id)),
            "the dojo offered no `{id}` row, so a model could not have chosen it.\n  menu: {menu:?}");
    }
    assert!(got, "the `{row}` row never put a {species:?} in the party.\n  {}", why(&stuck));

    // ⭐ **And now the other ball, which is where this branch is different from the other three.**
    // Mt Moon hides the fossil you leave and Oak's lab clears the table, so those branches close
    // themselves. `FightingDojoHitmonchanPokeBallText` hides only the ball that was *taken*: the
    // other object stays on the map, stays a row, and answers "Better not get greedy..." — a refusal
    // rather than a gift. So the menu is right to keep offering it, and what has to hold is that the
    // model is told what happened, which is the failure mode this repo keeps finding.
    let refused = run.tick_until(PATIENCE, |run| told(run, GREEDY));

    let state = run.fixture().game_state();
    assert!(!state.pokemon.iter().any(|mon| mon.species == other_species),
        "both dojo prizes are in the party, which the cartridge allows no save to hold");
    assert!(!state.pokedex_owned.contains(&other_species),
        "{other_species:?} is registered as owned; only one of the two can ever be");
    assert!(refused,
        "the model was never told why the second ball gave it nothing. `FightingDojoBetterNotGet\
         GreedyText` is the cartridge's own sentence for it and it never reached a turn.\n  {}",
        why(&stuck));

    assert_eq!(state.pokemon.len(), 6, "the snapshot banks a slot so the gift lands in the party");
    assert!(state.pokedex_owned.contains(&species));
    assert!(!was_stuck(&run), "the watchdog fired on the dojo branch");
    println!("[{name}] took {species:?}; the {other} answered \"Better not get greedy...\"");
}

/// **The Hitmonlee arm**, which `postgame::gifts::can_beat_the_karate_master_and_take_a_hitmonlee`
/// takes as part of the postgame chain — repeated here because a branch is only a branch with both
/// arms beside it, and because that test never went back for the second ball.
#[test]
fn the_dojo_branch_can_be_taken_to_hitmonlee() {
    dojo_arm("branch-dojo-hitmonlee", "HitmonleePokeBall");
}

/// **The Hitmonchan arm.** Nothing in this repo had ever taken it: the postgame chain spent the
/// choice on Hitmonlee and `postgame-hitmonlee.bin` is what every later leg reads, so Hitmonchan was
/// a species the suite could name and never obtain.
#[test]
fn the_dojo_branch_can_be_taken_to_hitmonchan() {
    dojo_arm("branch-dojo-hitmonchan", "HitmonchanPokeBall");
}

// ── 4. The Bike Voucher against buying ───────────────────────────────────────────────────────────

/// **The voucher arm** — the Cerulean clerk hands the Bicycle over as a gift.
///
/// `scripts/BikeShop.asm:10-31` is an `IsItemInBag` on the voucher: find it and the clerk runs
/// `GiveItem BICYCLE`, takes the voucher and never opens a menu at all. So this arm is one row, one
/// conversation and a bike, and it is the short half of the branch.
#[test]
fn the_bike_branch_can_be_taken_with_the_voucher() {
    let (mut run, stuck) = arm(BIKE_SHOP, "branch-bike-voucher", vec![Intent::Row("Clerk")]);

    let got = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state()
            .is_ok_and(|state| state.bag.iter().any(|entry| entry.id == ItemId::Bicycle))
    });
    assert!(got, "the clerk never handed the Bicycle over.\n  {}", why(&stuck));

    let state = run.fixture().game_state();
    assert!(!state.bag.iter().any(|entry| entry.id == ItemId::BikeVoucher),
        "the voucher is what pays for it and should have gone");
    // ⚠️ **And no menu at all.** `.dontHaveVoucher` is the branch this must not have taken, and it
    // opens a BICYCLE/CANCEL list; a bike that arrived through *that* would be a bike this save
    // could not afford, so the price never appearing is what says the voucher was found.
    assert!(!told(&run, "1000000"), "the voucher arm was quoted a price; it is a gift, not a sale");
    assert!(!was_stuck(&run), "the watchdog fired on the voucher arm");
    println!("[branch-bike-voucher] Bicycle received, voucher spent, no shop opened");
}

/// **The no-voucher arm** — the same clerk, the same row, and a shop at **¥1,000,000**.
///
/// ⚠️ **This is the arm no save can reach twice**, and nothing in this repo had ever played it. The
/// voucher is fetched once and spent immediately, so `postgame::fly_bike` walks the branch from the
/// side that works and the shop side existed only as a sentence in `PolicyStep::bicycle_steps`'
/// comment: *"Without the voucher the same NPC opens a BICYCLE/CANCEL menu at ¥1,000,000 instead,
/// which is the branch this step must not land in."*
///
/// The voucher is taken out of the bag by the **driver**, before the first tick — the same seam
/// `battle_refusals` uses for a catch rate. There is no other way to stand here: a save that has been
/// to the Pokémon Fan Club holds the voucher, and one that has not has no reason to be in Cerulean's
/// bike shop at all.
///
/// ⭐ **What it found is that the shop is not a shop.** `BikeShopClerkText`'s `.dontHaveVoucher`
/// branch is a hand-rolled `TextBoxBorder` + `HandleMenuInput` list rather than
/// `DisplayPokemartDialogue`, so the agent never enters `AgentState::PokemartShopping` and the model
/// is never handed a `buy_item` turn. It reads as one long text box, and the cartridge answers its
/// own menu: *"Sorry! You can't afford it!"*
///
/// ⚠️ **And that costs nothing, which is the part worth writing down rather than filing.** The A-mash
/// picks BICYCLE, which is the row a model would have picked too, and the price is **above Gen 1's
/// money cap of ¥999,999** — so this shop can never sell to anybody, on any save, however rich. The
/// only decision in it is one with a single outcome. What has to hold is therefore not that the model
/// chose, but that it is *told*: the price, the refusal, and a run still walking afterwards.
#[test]
fn the_bike_branch_can_be_taken_without_the_voucher() {
    let (mut run, stuck) = arm(BIKE_SHOP, "branch-bike-shop", vec![Intent::Row("Clerk")]);
    run.fixture().api().debug_take_item(ItemId::BikeVoucher).expect("the snapshot carries one");
    let money = run.fixture().game_state().money;

    // The cartridge's own sentence for a purchase it will not make, quoted into a turn.
    let refused = run.tick_until(PATIENCE, |run| told(run, "Sorry! You can"));
    assert!(refused,
        "the model was never told the shop refused. `BikeShopCantAffordText` is the cartridge's own \
         sentence for it and it never reached a turn.\n  {}", why(&stuck));
    assert!(told(&run, "1000000"),
        "the price never reached a turn, so a model reading this cannot tell why ¥{money} was not \
         enough");

    let state = run.fixture().game_state();
    assert!(!state.bag.iter().any(|entry| entry.id == ItemId::Bicycle),
        "¥{money} bought a ¥1,000,000 bicycle");
    assert_eq!(state.money, money, "nothing should have been spent");
    // ⚠️ **And the run is still playing.** A bespoke menu the agent cannot leave is the jam family
    // `stalls` is made of, and this one has three rows and a cursor the agent did not open.
    assert!(!was_stuck(&run), "the watchdog fired: the no-voucher shop wedged the agent");
    assert!(!run.endpoint.requests().iter().any(|request| request.has_tool("buy_item")),
        "this arm is a text box rather than a mart; a `buy_item` turn here means the agent's mart \
         driver has started claiming it, and the assertions above are then about the wrong thing");
    println!("[branch-bike-shop] the shop asked ¥1,000,000 of ¥{money} and refused, in one text box");
}

// ── 5. The in-game trades ────────────────────────────────────────────────────────────────────────

/// Put `species` in party slot `at`, keeping whatever is already there around it.
///
/// ⚠️ **A driver write, applied before the first tick**, which is the same seam `battle_refusals`
/// uses for a catch rate and `cheats` for the god party: the policy sees it only as an ordinary
/// `GameState`. There is no honest alternative for a trade — the four in-game trades this suite had
/// never made want a **Nidorino, a Slowbro, a Poliwhirl and a Raichu**, and not one of those is
/// catchable as itself anywhere these fixtures have been, so covering them by *playing* costs an
/// evolution grind and a Thunder Stone apiece and proves nothing about the trade.
///
/// ⚠️ **`at` is the whole of the second test below**, so it is a parameter rather than a constant:
/// the two tests differ by where the give-species is standing and by nothing else.
fn party_with(run: &mut LlmRun, species: PokemonSpecies, moves: [PokemonMoveName; 4], at: usize) {
    let state = run.fixture().game_state();
    let mut members: Vec<Pokemon> = state.pokemon.iter().cloned().collect();
    let seeded = Pokemon::maxed(species, "SWAPME", moves, state.name.clone(), state.player_id);
    members.insert(at.min(members.len()), seeded);
    members.truncate(6);
    let mut party = PokemonParty::default();
    for member in members {
        let _ = party.push(member);
    }
    run.fixture().api().debug_set_party(&party).expect("a party can be installed");
}

/// A Poliwhirl good enough to be a party member and irrelevant in every other way.
const SWAP_MOVES: [PokemonMoveName; 4] = [
    PokemonMoveName::WaterGun, PokemonMoveName::BodySlam,
    PokemonMoveName::Psychic, PokemonMoveName::Blizzard,
];

/// The three sentences a trader has for a Pokémon that is not the one it asked for —
/// `_WrongMon1Text`, `_WrongMon2Text`, `_WrongMon3Text`, one per text-pointer set. Asserted as a set
/// because which set a trade uses is a property of its index, and the claim is that *one* of them
/// reached the model. ⚠️ Matched from the front, for `battle_refusals::BALL_FAILED`'s reason: the
/// reader samples once a tick and the driver dismisses the box a tick or two later.
const WRONG_MON: [&str; 3] = ["What? That's not", "Hmmm? This isn", "...This is no"];

/// **An in-game trade through the deployed stack** — Poliwhirl → Jynx at `CeruleanTradeHouse`.
///
/// The trades are on step 8's list because each is one-shot per save: `postgame::trades` walks five
/// of the nine as a linear chain and each of them spends the NPC for good. What this asks is the
/// question the chain cannot, because every one of those legs is a `DeterministicPolicy` driving
/// [`PolicyStep::PartyScript`]: **what is a model offered here?**
///
/// The house is one warp off the Cerulean terrace the bike-shop snapshot already stands on, and the
/// gambler is an ordinary `talk to` row — so the walk and the conversation are both things a model
/// can ask for. The give-species is seeded because it is not obtainable on this save; see
/// [`party_with`].
///
/// ⚠️ **This one is the easy case**, and it is here to be compared with the next: the Poliwhirl is at
/// slot 0, which is where an A-mash would have found it anyway. The test below is the same trade with
/// the same Pokémon three slots back, and until `PokemonAgent::party_menu` existed that one traded
/// nothing at all. `postgame::trades::every_in_game_trade_can_be_made_by_talking_to_the_trader`
/// carries the breadth: all nine, each with the give-species off the front.
#[test]
fn an_in_game_trade_can_be_reached_through_the_llm_path() {
    let trade = crate::pokemon::postgame::trades::trade_for(PokemonSpecies::Poliwhirl);
    let (mut run, stuck) = arm(BIKE_SHOP, "branch-trade", vec![
        Intent::Enter("CeruleanCity"),
        Intent::Enter("CeruleanTradeHouse"),
        Intent::Row("Gambler"),
        Intent::Row("Gambler"),
    ]);
    party_with(&mut run, trade.give, SWAP_MOVES, 0);

    let traded = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state()
            .is_ok_and(|state| state.pokemon.iter().any(|mon| mon.species == trade.get))
    });

    let state = run.fixture().game_state();
    println!("[branch-trade] party {:?}", state.pokemon.iter().map(|p| p.species).collect::<Vec<_>>());
    assert!(traded, "the gambler never handed a {:?} over.\n  {}", trade.get, why(&stuck));
    assert!(!state.pokemon.iter().any(|mon| mon.species == trade.give),
        "the {:?} was not handed over; a trade swaps rather than adds", trade.give);
    assert_owned(&mut run, trade.get);
    assert!(!was_stuck(&run), "the watchdog fired on the trade");
    println!("[branch-trade] traded {:?} -> {:?}", trade.give, trade.get);
}

/// ⭐ **The trade finds its own Pokémon, wherever in the party it is.**
///
/// This is the test that was the finding. `InGameTrade_DoTrade` calls `DisplayPartyMenu` **without
/// resetting `wCurrentMenuItem`** — the same missing instruction `postgame::gifts::tick` exists for —
/// so before `PokemonAgent`'s `PartyMenuAnswer` the agent's A-mash offered the trader whatever the cursor
/// had been left on. Move the Poliwhirl one slot back and the same walk, the same row and the same
/// conversation traded nothing at all: *"Hmmm? This isn't POLIWHIRL."*
///
/// ⚠️ **It is the agent that answers this menu and not the policy, and that is the argument rather
/// than an omission.** The menu has exactly one acceptable row and the cartridge is the thing that
/// says which — `InGameTrade_DoTrade` compares the selected species against
/// `wInGameTradeGiveMonSpecies` and refuses anything else — so there is nothing for a model to
/// decide and a `use_field_move` variant for it would be a round trip spent on a list with one legal
/// entry. What a model *does* decide is whether to talk to the trader at all, and that has always
/// been an ordinary menu row. `PokemonAgent`'s `PartyMenuAnswer` carries the whole of it, including
/// why the Day Care and the Name Rater are **declined** instead.
#[test]
fn a_trade_finds_the_give_species_wherever_it_is_in_the_party() {
    let trade = crate::pokemon::postgame::trades::trade_for(PokemonSpecies::Poliwhirl);
    let (mut run, stuck) = arm(BIKE_SHOP, "branch-trade-benched", vec![
        Intent::Enter("CeruleanCity"),
        Intent::Enter("CeruleanTradeHouse"),
        Intent::Row("Gambler"),
    ]);
    // Three slots back, and that is the entire difference from the test above.
    party_with(&mut run, trade.give, SWAP_MOVES, 3);

    let traded = run.tick_until(PATIENCE, |run| {
        run.fixture().try_game_state()
            .is_ok_and(|state| state.pokemon.iter().any(|mon| mon.species == trade.get))
    });

    let state = run.fixture().game_state();
    assert!(traded,
        "a benched {:?} was not traded. The cursor opens where the last party menu left it, so \
         without a driver this menu hands over slot {} instead.\n  party: {:?}\n  {}",
        trade.give,
        run.endpoint.requests().len().min(1),
        state.pokemon.iter().map(|mon| mon.species).collect::<Vec<_>>(),
        why(&stuck));
    assert!(!state.pokemon.iter().any(|mon| mon.species == trade.give),
        "the {:?} is still in the party; something else was handed over", trade.give);
    assert_owned(&mut run, trade.get);
    assert!(!WRONG_MON.iter().any(|words| told(&run, words)),
        "the trader was handed the wrong Pokémon at some point in this run");
    assert!(!was_stuck(&run), "the watchdog fired on the trade");
    println!("[branch-trade-benched] a {:?} in slot 4 was found and traded for a {:?}",
        trade.give, trade.get);
}

/// **A trade the party cannot make is backed out of, and the model is told why.**
///
/// The other half of [`a_trade_finds_the_give_species_wherever_it_is_in_the_party`]: with none of the
/// give-species anywhere, there is no row to move the cursor to. Offering the trader something it
/// will refuse costs a whole conversation to be told so, so the driver presses B instead and says in
/// one line what the trade wanted — and the trader's own opening question, which names the species,
/// is quoted into the turn either way.
///
/// ⚠️ **The watchdog assertion is the load-bearing one.** A party menu nothing answers is a closed
/// loop under A, which is the jam family `stalls` is made of.
#[test]
fn a_trade_with_nothing_to_give_backs_out_and_says_so() {
    let trade = crate::pokemon::postgame::trades::trade_for(PokemonSpecies::Poliwhirl);
    let (mut run, _) = arm(BIKE_SHOP, "branch-trade-empty", vec![
        Intent::Enter("CeruleanCity"),
        Intent::Enter("CeruleanTradeHouse"),
        Intent::Row("Gambler"),
        Intent::Row("Gambler"),
    ]);
    let before = run.fixture().game_state();
    assert!(!before.pokemon.iter().any(|mon| mon.species == trade.give),
        "this test is about a party with no {:?} in it", trade.give);

    // ⚠️ **Read out of the *turn*, not out of the event stream.** An agent event nobody carried into
    // the prompt is a sentence no model saw, which is this file's whole standard of proof.
    let spoke = run.tick_until(PATIENCE, |run| told(run, "there is none in the party"));

    let state = run.fixture().game_state();
    assert!(spoke, "the model was never told why the trade did not happen");
    assert!(!WRONG_MON.iter().any(|words| told(&run, words)),
        "the trader was offered a Pokémon it had not asked for; the menu should have been backed \
         out of instead");
    assert!(!state.pokemon.iter().any(|mon| mon.species == trade.get),
        "a {:?} arrived from a trade the party could not make", trade.get);
    assert!(!was_stuck(&run), "the watchdog fired: an unmakeable trade wedged the party menu");
    println!("[branch-trade-empty] the trade wanted a {:?}, the party had none, and it said so",
        trade.give);
}

/// Diagnostic: what does the agent see while an in-game trade has the party menu open?
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "diagnostic — run with --ignored --nocapture"]
fn probe_trade_party_menu() {
    use crate::pokemon::symbols::pokered_symbols;
    let trade = crate::pokemon::postgame::trades::trade_for(PokemonSpecies::Poliwhirl);
    let (mut run, _) = arm(BIKE_SHOP, "probe-trade", vec![
        Intent::Enter("CeruleanCity"),
        Intent::Enter("CeruleanTradeHouse"),
        Intent::Row("Gambler"),
        Intent::Row("Gambler"),
    ]);
    party_with(&mut run, trade.give, SWAP_MOVES, 1);
    let mut last = String::new();
    for _ in 0..4000 {
        run.tick();
        let fixture = run.fixture();
        let mode = fixture.api().game_mode();
        let menu = fixture.api().mmu().read_menu_state();
        let give = fixture.api().mmu().read_pointer(&pokered_symbols::wInGameTradeGiveMonSpecies);
        let which = fixture.api().mmu().read_pointer(&pokered_symbols::wWhichTrade);
        let ptype = fixture.api().mmu().read_pointer(&pokered_symbols::wPartyMenuTypeOrMessageID);
        let state = fixture.agent.state_debug();
        let line = format!("{mode:?} {state} menu={menu:?} give={give:#04x} which={which} ptype={ptype}");
        if line != last {
            println!("{line}");
            last = line;
        }
    }
}

// ── The snapshots, cut ───────────────────────────────────────────────────────────────────────────

/// Cut Oak's lab one A press before the starter branch.
#[test]
#[cfg(feature = "regen-fixtures")]
fn regen_oaks_lab_fixture() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/start-of-game-state.bin"),
        Duration::from_mins(10),
        vec![
            PolicyStep::enter(Map::RedsHouse1F),
            PolicyStep::enter(Map::PalletTown),
            // Oak stops you on the way to Route 1 and marches you into the lab.
            PolicyStep::soft_goto(Map::Route1),
        ],
    );
    fixture.step_until_exhausted();
    // Oak stops the player in the grass and marches them into the lab; the walk that triggers it is
    // still in flight when the queue empties, so the arrival is waited for rather than assumed.
    fixture.run_until(|s| s.map.map == Map::OaksLab);
    // And then his speech, which the agent advances by itself. Cut once the table is offering rows.
    for _ in 0..3000 { fixture.step(); }
    let s = fixture.game_state();
    println!("ended {} @ {} — party {:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
    for action in s.map.actions() {
        println!("   {:?} @ {}", action.tile, action.destination);
    }
    assert_eq!(s.map.map, Map::OaksLab, "the branch is in the lab");
    assert_eq!(s.pokemon.len(), 0, "the starter has not been chosen yet");
    fixture.save_state_named("src/pokemon/data/branch-oaks-lab.bin").unwrap();
}

/// Cut Mt Moon B2F with **both** fossils still on the floor.
///
/// ⚠️ **The Super Nerd cannot be beaten in advance, and both arms therefore fight him.**
/// `MtMoonB2FFossilAreaCoords` is a *coordinate* trigger inside the chamber, so his battle is
/// reachable only by walking at the fossils — which is the branch action itself. He is left where he
/// is, and each arm pays for him: see [`fossil_arm`] for what that costs the intent list.
#[test]
#[cfg(feature = "regen-fixtures")]
fn regen_mt_moon_fossils_fixture() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/mt-moon.bin"),
        Duration::from_mins(40),
        vec![
            PolicyStep::enter_at(Map::MtMoonB1F, 5, 5),
            PolicyStep::enter_at(Map::MtMoonB2F, 21, 17),
        ]
        .into_iter()
        // ⚠️ **These two walks are what make the snapshot restorable, and neither is the branch.**
        // The Super Nerd is a *coordinate* trigger, so walking at him starts no battle — but it
        // carries the player off the B2F landing and up to the chamber, and that route crosses
        // `ROCKET1`'s line of sight. A trainer walks up to the player on the map's own tick rather
        // than on a step, so a state cut anywhere in that sight line **restores into a battle** and
        // every arm's first turn is `choose_battle_action` instead of the menu the branch lives in.
        // Measured twice: the first version of this fixture did exactly that, and a version with the
        // Super Nerd walk removed wandered off and was cut inside a different trainer's.
        .chain(std::iter::repeat_n(
            PolicyStep::InteractIfReachable(MapSprite::MTMOONB2F_SUPER_NERD), 4))
        .chain(std::iter::repeat_n(
            PolicyStep::InteractIfReachable(MapSprite::MTMOONB2F_ROCKET1), 3))
        .collect(),
    );
    fixture.pimp_pokemon();
    fixture.step_until_exhausted();
    // Out of whatever fight the last interact left running, and then a second of quiet.
    fixture.run_until(|s| s.battle.is_none());
    for _ in 0..500 { fixture.step(); }
    let s = fixture.game_state();
    println!("ended {} @ {} — party {:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
    for action in s.map.actions() {
        println!("   {:?} @ {}", action.tile, action.destination);
    }
    assert_eq!(s.map.map, Map::MtMoonB2F, "the branch is on B2F");
    assert!(s.battle.is_none(), "a snapshot cut mid-battle restores into one, and the branch is a menu");
    assert!(s.map.actions().iter().any(|a| format!("{:?}", a.tile).contains("Dome Fossil")),
        "the Dome Fossil has to still be a row");
    assert!(s.map.actions().iter().any(|a| format!("{:?}", a.tile).contains("Helix Fossil")),
        "the Helix Fossil has to still be a row");
    assert!(!s.bag.iter().any(|i| matches!(i.id, ItemId::HelixFossil | ItemId::DomeFossil)),
        "neither fossil has been taken yet");
    fixture.save_state_named("src/pokemon/data/branch-mt-moon-fossils.bin").unwrap();
}

/// Cut the Fighting Dojo with the Karate Master beaten and both prize balls still on the floor.
///
/// The same route `PolicyStep::hitmonlee_steps` takes, minus its last two steps — the `CollectItem`
/// that spends the choice and the walk back out. A party slot is banked first for the same reason it
/// is there: with six members the gift takes `SendNewMonToBox` and the arms stop being about the
/// choice.
#[test]
#[cfg(feature = "regen-fixtures")]
fn regen_dojo_prize_fixture() {
    /// Omanyte, which `postgame-lapras.bin` carries in slot 4 — dex-registered, so banking it is free.
    const BANK_SLOT: u8 = 4;
    let mut steps = PolicyStep::hitmonlee_steps(BANK_SLOT);
    steps.truncate(steps.len() - 2);
    let mut fixture = TestFixture::new(
        include_bytes!("../data/postgame-lapras.bin"),
        Duration::from_mins(45),
        steps,
    );
    fixture.step_until_exhausted();
    for _ in 0..500 { fixture.step(); }
    let s = fixture.game_state();
    println!("ended {} @ {} — party {:?}", s.map.map, s.map.player_position,
        s.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
    for action in s.map.actions() {
        println!("   {:?} @ {}", action.tile, action.destination);
    }
    assert_eq!(s.map.map, Map::FightingDojo, "the branch is in the dojo");
    assert_eq!(s.pokemon.len(), 5, "a slot has to be free or both arms go to the box");
    assert!(!s.pokedex_owned.contains(&PokemonSpecies::Hitmonlee));
    assert!(!s.pokedex_owned.contains(&PokemonSpecies::Hitmonchan));
    fixture.save_state_named("src/pokemon/data/branch-dojo-prize.bin").unwrap();
}

/// Cut the Cerulean Bike Shop with the Bike Voucher in the bag, one A press from the clerk.
///
/// `PolicyStep::bicycle_steps` without its `Interact`s. The voucher is what the two arms differ by:
/// hold it and `IsItemInBag` hands the Bicycle over as a gift, and the same clerk without it opens a
/// BICYCLE/CANCEL mart menu at ¥1,000,000 instead.
#[test]
#[cfg(feature = "regen-fixtures")]
fn regen_bike_shop_fixture() {
    let mut steps = PolicyStep::bicycle_steps();
    steps.retain(|step| !matches!(step, PolicyStep::Interact(_)));
    let mut fixture = TestFixture::new(
        include_bytes!("../data/postgame-bike-voucher.bin"),
        Duration::from_mins(45),
        steps,
    );
    fixture.step_until_exhausted();
    for _ in 0..500 { fixture.step(); }
    let s = fixture.game_state();
    println!("ended {} @ {} — money {}", s.map.map, s.map.player_position, s.money);
    for action in s.map.actions() {
        println!("   {:?} @ {}", action.tile, action.destination);
    }
    assert_eq!(s.map.map, Map::BikeShop, "the branch is in the shop");
    assert!(s.bag.iter().any(|i| i.id == ItemId::BikeVoucher), "the voucher is what one arm spends");
    assert!(!s.bag.iter().any(|i| i.id == ItemId::Bicycle), "the bike has not been collected yet");
    fixture.save_state_named("src/pokemon/data/branch-bike-shop.bin").unwrap();
}
