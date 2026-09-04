//! Stalls the fuzzer found, each frozen into a save state and re-run in a second.
//!
//! **Default tier on purpose.** `soak` is where a jam gets *found* — thirteen starting states under
//! one seed, a minute of wall clock, and it only visits what that seed happens to visit. That is the
//! wrong tool for proving a jam stays fixed. So every one it finds is promoted here: the emulator
//! state at the moment the agent went quiet, `include_bytes!`'d, replayed against a fresh agent, and
//! asserted to reach a decision point. Each costs about two seconds, so they run on every
//! `cargo test --release`.
//!
//! # Adding one
//!
//! `soak` drops `target/test-artifacts/soak-<state>-seed<N>.bin` when it fails. Copy it to
//! `src/pokemon/data/stall-<what>.bin`, add a case below, and **check it fails before the fix** —
//! that is the whole value, and it is not automatic (see the ⚠️). [`probe_stall_artifacts`] at the
//! foot of this file is the bulk version: it replays a whole directory of them and says which still
//! reproduce, which is what a sweep across seeds leaves you holding.
//!
//! ⚠️ **Not every stall survives the trip, and one that does not must not be committed anyway.** The
//! save state holds the *emulator*, not the agent: a fresh `PokemonAgent` starts `Idle` with an empty
//! world graph and no route in flight. A jam the game's own screen re-creates — a menu that bounces,
//! a battle the agent cannot leave — reproduces perfectly. A jam that lived in the agent's own state,
//! like `OverworldMovement` committed to a route, does not: replaying it just picks a fresh action. So
//! a test added here without watching it go red first may be asserting nothing at all.

use super::*;
use crate::pokemon::policy::RandomPolicy;

/// Game time each case is given to reach a decision point.
const ESCAPE_BUDGET: Duration = Duration::from_secs(120);

/// The longest silence a case may show before it counts as still stuck.
///
/// ⚠️ **Stricter than `soak`'s 300 s, and measured rather than picked.** These start *inside* a jam,
/// so a working agent leaves almost at once and there is nothing legitimate to wait for — a tighter
/// limit catches a regression sooner. But not arbitrarily tighter: five hours of clean random play
/// measured its longest *healthy* silence at **46.8 s** (a battle resolving through its animations),
/// so anything under about 60 s would fail on ordinary play. This is roughly twice the measured worst
/// and well under the watchdog. The first draft used 20 s and failed on a perfectly healthy battle.
const QUIET_LIMIT: Duration = Duration::from_secs(90);

/// Replay `state` against a fresh agent and return the longest it went without reaching a decision
/// point, plus where it ended up.
///
/// The policy is seeded so a case cannot pass or fail on the draw — these assert that the agent can
/// get *out*, which must not depend on what it chooses once it has.
fn longest_silence(state: &[u8], seed: u64) -> (Duration, String, String) {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(state).expect("a committed stall fixture should load");
    let mut cache = MapMetadataCache::default();
    let mut agent = PokemonAgent::new(Box::new(RandomPolicy::seeded(seed)));

    let budget = MachineCycles::from_duration(ESCAPE_BUDGET);
    let mut emulated = MachineCycles::ZERO;
    let mut worst = Duration::ZERO;
    let mut worst_state = String::new();

    while emulated < budget {
        let ran = gb.run(AGENT_RESOLUTION);
        emulated += ran;
        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        agent.update(&mut api, ran).ok();
        agent.drain_events();

        let gap = agent.since_last_policy_poll();
        if gap > worst {
            worst = gap;
            worst_state = agent.state_debug();
        }
    }

    let where_it_is = PokemonApi::with_cache(&mut gb, &mut cache)
        .game_state()
        .map_or_else(|_| "unreadable".into(), |s| format!("{} at {}", s.map.map, s.map.player_position));
    (worst, worst_state, where_it_is)
}

/// Assert a fixture is no longer a stall, reporting what it did if it still is.
fn assert_escapes(name: &str, state: &[u8]) {
    // Three seeds, because escaping must not depend on what the policy picks once it is free.
    for seed in [1, 2, 3] {
        let (worst, worst_state, where_it_is) = longest_silence(state, seed);
        assert!(
            worst < QUIET_LIMIT,
            "{name} (seed {seed}): the agent went {worst:?} of game time without reaching a decision \
             point — still stuck.\n  state: {worst_state}\n  where: {where_it_is}",
        );
        println!("[stall] {name} (seed {seed}): out in {worst:?}, {where_it_is}");
    }
}

/// **`soak` seed 1, 3600 s in** — a Bulbasaur out of PP against a Weedle in Viridian Forest.
///
/// The move list refuses with "No PP left for this move!" and drops back to *itself*, cursor still on
/// the spent move, so the agent's A-mash re-selects it and bounces again — the same closed loop as the
/// PC menus, which is the second time that exact shape has wedged a run.
///
/// ⚠️ **The offered moves were already filtered on `pp > 0`** (`Pokemon::available_battle_moves`), so
/// this is not a bad choice by the policy — it is the game and the party data disagreeing, which no
/// filter over the party data can fix. The agent has to handle the refusal.
#[test]
fn a_move_with_no_pp_left_does_not_trap_the_battle() {
    assert_escapes("no-pp-move", include_bytes!("../data/stall-no-pp-move.bin"));
}

/// **`soak` seed 1, `at-vermilion`, 372 s in** — the S.S. Ticket used against a wild Oddish.
///
/// "OAK: <PLAYER>! This isn't the time to use that!" drops back to the *bag list* with the cursor
/// still on the key item, so the agent's A-mash re-selects it and bounces again. The third appearance
/// of the closed-loop-under-A shape, after the PC menus and the spent move — and the one with the
/// widest blast radius, because **every** bag holds something the game will refuse. Eleven of the
/// thirteen `soak` states wedged on it within six minutes of game time each; only the two whose
/// fixtures carry an empty bag survived, which is exactly why five hours from a fresh save had never
/// found it.
#[test]
fn a_key_item_used_in_battle_does_not_trap_the_bag() {
    assert_escapes("battle-key-item", include_bytes!("../data/stall-battle-key-item.bin"));
}

/// **`can_get_rainbow_badge`, Erika's Vileplume** — a party with no PP anywhere.
///
/// ⚠️ **The wedge is in the policy, not the agent, which is why this does not use
/// [`assert_escapes`].** `RandomPolicy` leaves at once by reaching for a bag item;
/// `DeterministicPolicy`'s last resort deliberately refuses to (the first bag entry is often a key
/// item the game will not use — the closed loop two tests above). With `Fight` filtered out by
/// `pp > 0`, `Run` absent because it is a trainer, and `SwitchPokemon` absent because everything else
/// has fainted, the whole option list was bag items, so it answered `None` — which means "still
/// thinking". The agent then sits in `BattleState::AwaitingPolicy` showing the main battle menu, the
/// emulator runs, the watchdog never fires because it *is* being polled, and **nothing is printed**.
/// Three runs died in that silence before `pick_battle_action` was made to say so.
///
/// The fix is in `battle_options`: all-zero PP is **Struggle**, which is a move, so the moves are
/// offered anyway and the cartridge substitutes it.
#[test]
fn a_party_with_no_pp_anywhere_still_gets_an_answer() {
    use crate::pokemon::policy::DeterministicPolicy;
    let state = include_bytes!("../data/stall-no-pp-trainer-battle.bin");
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(state).expect("a committed stall fixture should load");
    let mut cache = MapMetadataCache::default();
    // An empty queue: `pick_battle_action` does not read it, and what is under test is the answer it
    // gives when the active mon has nothing left, not the route it is on.
    let mut agent = PokemonAgent::new(Box::new(DeterministicPolicy::new(1, [])));

    // ⚠️ **Count *actions*, not silence, because the watchdog cannot see this one.**
    // `since_last_policy_poll` is reset by `poll_policy` whatever the answer is, so a policy that is
    // asked every tick and answers `None` every tick looks perfectly healthy to it — which is exactly
    // why the deployed shape of this bug is a run that stops moving and reports nothing at all.
    let budget = MachineCycles::from_duration(ESCAPE_BUDGET);
    let mut emulated = MachineCycles::ZERO;
    let mut actions = 0usize;
    while emulated < budget {
        let ran = gb.run(AGENT_RESOLUTION);
        emulated += ran;
        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        agent.update(&mut api, ran).ok();
        actions += agent.drain_events().iter()
            .filter(|e| matches!(e, AgentEvent::BattleActionStarted { .. })).count();
    }
    assert!(actions > 0,
        "the scripted policy took no battle action in {ESCAPE_BUDGET:?} of game time against a fight \
         it cannot win and cannot leave — it is waiting at the menu for ever.\n  state: {}",
        agent.state_debug());
    println!("[stall] no-pp-trainer-battle: {actions} battle actions taken");
}

/// **`soak` seed 1, `postgame-pc-box`, 554 s in** — the Lift Key against a Bug Catcher's Weedle.
///
/// The same refusal in a **trainer** battle, which is a different escape: there is no RUN, so the
/// only way out of the turn is back up the bag list to the main menu and a different action. Kept
/// beside the wild case rather than instead of it because the two exercise different arms —
/// `battle_options` offers `Run` in one and not the other, so a fix that leans on fleeing would pass
/// one and wedge the other.
#[test]
fn a_key_item_used_against_a_trainer_does_not_trap_the_bag() {
    assert_escapes("battle-key-item-trainer", include_bytes!("../data/stall-battle-key-item-trainer.bin"));
}

/// **`soak` seed 1, `at-vermilion`, 1681 s in** — the man in the Cerulean badge house.
///
/// `CeruleanBadgeHouseMiddleAgedManText` is a `.loop`: print "Which of the 8 BADGEs should I
/// describe?", show the badge list, describe the one under the cursor, jump back. Its only exit is
/// `jr c, .done` — carry, which `DisplayListMenuID` sets on **B**. So the agent read badge
/// descriptions for the rest of the run. The fourth closed-loop-under-A in this project, and the
/// first one out of a *conversation* rather than a menu the agent opened itself.
#[test]
fn a_list_that_only_b_leaves_does_not_trap_a_conversation() {
    assert_escapes("badge-house-list", include_bytes!("../data/stall-badge-house-list.bin"));
}

/// **`soak` seed 1, `at-cinnabar`, 2258 s in** — SURF chosen in Saffron City.
///
/// The party menu's field-move box is the badge house's loop without the list: A on SURF prints "No
/// SURFing on <mon> here!", the box closes, and the party menu is underneath it with the cursor
/// exactly where it was. Every "you can't use that here" in the game has this shape, which is why the
/// fix is a bound on the conversation rather than a check for this message.
#[test]
fn a_field_move_the_game_refuses_does_not_trap_the_party_menu() {
    assert_escapes("field-move-refused", include_bytes!("../data/stall-field-move-refused.bin"));
}

/// **`soak` seeds 28/38/50/62, 1801 s in** — a fainted Pokémon chosen from the battle party menu.
///
/// "There's no will to fight!" drops back to the party list, and the driver *knows* what to do about
/// it: walk the cursor off the fainted mon and confirm a live one. It pressed Up. A message box
/// swallows directional input, so the box stayed, the cursor never moved, and the fight never
/// resumed. ⚠️ **The lesson is the message, not the menu** — a driver that is right about the next
/// button still has to clear what is on top of it first.
#[test]
fn a_fainted_pokemon_chosen_in_battle_does_not_trap_the_party_menu() {
    assert_escapes("fainted-switch", include_bytes!("../data/stall-fainted-switch.bin"));
}

/// **`soak` seeds 8/30/36/44, 1082 s in** — a Card Key door on Silph Co 2F, with no Card Key.
///
/// `handle_card_key_door` spends 40 A presses on a faced `$18`/`$24` tile and then declares it a wall,
/// blocking it so the router goes round. It also reset its counter and started another forty on the
/// very next tick, having never read the verdict it had just recorded — and every one of those
/// presses prints "Darn! It needs a CARD KEY!", which is a text box, which is another A — so the run
/// stands at that door for as long as it lasts.
#[test]
fn a_card_key_door_that_will_not_open_is_only_tried_once() {
    assert_escapes("card-key-door", include_bytes!("../data/stall-card-key-door.bin"));
}

/// **`soak` seed 119, `postgame-aides`, 223 s in** — a Ditto on Route 15, one frame, for ever.
///
/// The active Pokémon has fainted and "It's super effective!" is still on screen, but
/// `battle_menu_state` reads `wTopMenuItemX/Y` — which *linger* — and reports the party list that was
/// there before the box was drawn over it. The forced-switch arm then pressed a direction at a message
/// box, which swallows it, and nothing moved again: 180 s on a single frame.
///
/// ⚠️ **This is the fainted-switch lesson from the other side.** There the message was a refusal and
/// could be named; here it is an ordinary battle line, so the fix cannot be a string. What tells them
/// apart is the *screen*: a party list draws an HP bar per member, a battle box draws the active mon's
/// alone, so two or more `/` means the list is really there — the same heuristic the item driver uses.
#[test]
fn a_battle_message_over_the_party_list_is_cleared_first() {
    assert_escapes("battle-message-over-party", include_bytes!("../data/stall-battle-message-over-party.bin"));
}

/// **`soak` seed 11, `at-cinnabar`, 2301 s in** — the water current on Seafoam Islands B4F.
///
/// Not a menu at all: the current takes the player every few seconds, the agent passes through
/// `RunningScript` and back into a re-derived walk toward the same warp, and swims into it again. The
/// walk *had* a bound — 4500 ticks of `state_ticks` — but a state torn down by an interruption starts
/// its counter over, so each pass handed it a fresh 90 s. ⚠️ **`OverworldMovement` was the one state
/// believed safe for an agent-level counter** (it does not rebuild itself); this is the second way of
/// losing one. The bound is silence now, which only the policy being asked can clear.
#[test]
fn a_walk_the_current_keeps_interrupting_gives_up() {
    assert_escapes("seafoam-current", include_bytes!("../data/stall-seafoam-current.bin"));
}

/// **`soak` seeds 76…120, eleven of them** — Bill's own PC, in his house on Route 25.
///
/// "Which POKéMON do you want to see?" over EEVEE/FLAREON/JOLTEON/VAPOREON/**CANCEL**: A on the
/// resting cursor shows the Eevee page and returns to the menu with the cursor untouched. The fourth
/// closed loop under A, and the one that showed the escape hatch needed a third way in — this menu is
/// drawn in a plain message box, so neither "a list menu" nor "a field-move box" sees it. The CANCEL
/// entry is what does: every pick-one-of-these menu in the game offers one, and no conversation does.
#[test]
fn a_menu_offering_cancel_does_not_trap_a_conversation() {
    assert_escapes("bills-pc-list", include_bytes!("../data/stall-bills-pc-list.bin"));
}

/// **`soak` seed 70, 1500 s in** — BAIT thrown at the same Rhyhorn for ever.
///
/// The Safari menu keeps its cursor between turns, and `WaitingForMenu` only handed the turn to the
/// policy when the cursor was on the position a *fresh* menu opens with (FIGHT, or Safari BALL).
/// Anywhere else fell through to "press A", which re-threw whatever it was resting on. An ordinary
/// battle never showed it, because the game resets that cursor to FIGHT; the Safari Zone does not.
#[test]
fn a_safari_menu_cursor_left_on_bait_does_not_repeat_itself() {
    assert_escapes("safari-menu", include_bytes!("../data/stall-safari-menu.bin"));
}


/// Diagnostic, not a test: replay **every** `.bin` in a directory and print the longest silence each
/// one still shows. This is the triage half of the hunt loop.
///
/// A sweep across seeds drops one artifact per failure (`soak-<state>-seed<N>.bin`), and after a fix
/// the only question worth asking of the pile is *which of these still reproduce*. Promoting them one
/// at a time to answer that is far slower than reading the answer off a list — and the list is also
/// how a fix that helps four cases and misses the fifth shows itself.
///
/// ```shell
/// GB_STALL_DIR=/path/to/artifacts cargo test --release --features diagnostics --bin gb -- \
///   probe_stall_artifacts --exact --ignored --nocapture
/// ```
///
/// The budget is [`PROBE_BUDGET`] rather than [`ESCAPE_BUDGET`]: a case that is still stuck should be
/// given long enough that "it escaped after 100 s" cannot be mistaken for "it never escaped".
#[test]
#[cfg(feature = "diagnostics")]
#[ignore = "diagnostic, not a test; run with --ignored --nocapture"]
fn probe_stall_artifacts() {
    /// How much game time each artifact gets. Longer than a `stalls` case is allowed, on purpose.
    const PROBE_BUDGET: Duration = Duration::from_secs(180);

    let dir = std::env::var("GB_STALL_DIR").unwrap_or_else(|_| "target/test-artifacts".into());
    let Ok(entries) = std::fs::read_dir(&dir) else {
        println!("no such directory: {dir}");
        return;
    };
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map_or(false, |e| e == "bin"))
        .collect();
    paths.sort();
    println!("[probe] {} save states in {dir}", paths.len());
    for path in paths {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
        if gb.load_state(&bytes).is_err() {
            println!("[probe] {name}: not a save state");
            continue;
        }
        let mut cache = MapMetadataCache::default();
        let mut agent = PokemonAgent::new(Box::new(RandomPolicy::seeded(1)));
        let mut emulated = MachineCycles::ZERO;
        let (mut worst, mut worst_state) = (Duration::ZERO, String::new());
        while emulated < MachineCycles::from_duration(PROBE_BUDGET) {
            let ran = gb.run(AGENT_RESOLUTION);
            emulated += ran;
            let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
            agent.update(&mut api, ran).ok();
            agent.drain_events();
            if agent.since_last_policy_poll() > worst {
                worst = agent.since_last_policy_poll();
                worst_state = agent.state_debug();
            }
        }
        let where_it_is = PokemonApi::with_cache(&mut gb, &mut cache).game_state()
            .map_or_else(|_| "unreadable".into(),
                         |s| format!("{} at {}", s.map.map, s.map.player_position));
        let verdict = if worst < QUIET_LIMIT { "escapes" } else { "STILL STUCK" };
        println!("[probe] {name}: {verdict} — worst {worst:?} in {worst_state} — {where_it_is}");
    }
}

/// **The deployed run of 2026-09-01, checkpointed inside the jam** — a lv44 Charizard against a
/// Gastly on Pokémon Tower 3F, with no Silph Scope in the bag.
///
/// `IsGhostBattle` is true for **every** wild battle on Pokémon Tower 1F-7F until the Scope is
/// found, not merely for the Marowak, so no move executes: the player's turn prints "too scared to
/// move", the ghost's prints "Get out...", and neither side ever loses a hit point. The run picked
/// Slash every 3.3 s for as long as anyone watched, and nothing in the process could have noticed —
/// a battle script was answering on the emulator thread, so no request was made and the model was
/// never asked, while the watchdog stayed quiet because the agent was reaching a decision point
/// every tick. See `battle::is_ghost_battle`.
///
/// ⚠️ **The assertion is that the battle *ends*, not that decisions keep coming, and that is the
/// whole point of this case.** Every other test in this file can lean on the agent getting an answer
/// out of the policy; here the answers never stopped. Counting `BattleActionStarted` the way
/// `a_party_with_no_pp_anywhere_still_gets_an_answer` does would have passed on the deployed bug at
/// one action per turn for ever, and so would `assert_escapes` — there is no silence to measure.
///
/// ⚠️ **`DeterministicPolicy`, not `RandomPolicy`.** A random policy draws `Run` out of a short
/// option list within a few turns and passes whether or not the game is understood, which is a test
/// that asserts nothing. The scripted one picks its best damaging move against a full-HP lead with
/// full PP — its flee arms key on low HP and low PP and neither fires here — so before the fix it
/// fights for the whole budget.
#[test]
fn a_ghost_battle_is_left_rather_than_fought_for_ever() {
    use crate::pokemon::policy::DeterministicPolicy;
    let state = include_bytes!("../data/stall-ghost-battle.bin");
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(state).expect("a committed stall fixture should load");
    let mut cache = MapMetadataCache::default();
    // An empty queue, as in the no-PP case: what is under test is the answer `pick_battle_action`
    // gives to a fight that cannot be won, not the route it happens to be on.
    let mut agent = PokemonAgent::new(Box::new(DeterministicPolicy::new(1, [])));

    let budget = MachineCycles::from_duration(ESCAPE_BUDGET);
    let mut emulated = MachineCycles::ZERO;
    let mut left_at = None;
    let mut turns = 0usize;
    while emulated < budget {
        let ran = gb.run(AGENT_RESOLUTION);
        emulated += ran;
        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        agent.update(&mut api, ran).ok();
        turns += agent.drain_events().iter()
            .filter(|e| matches!(e, AgentEvent::BattleActionStarted { .. })).count();
        if api.game_state().is_ok_and(|s| s.battle.is_none()) {
            left_at = Some(emulated.to_duration());
            break;
        }
    }

    let left_at = left_at.unwrap_or_else(|| panic!(
        "still in the ghost battle after {ESCAPE_BUDGET:?} of game time and {turns} battle actions \
         — every one of them a move the cartridge refuses to execute.\n  state: {}",
        agent.state_debug()));
    println!("[stall] ghost-battle: out in {left_at:?} after {turns} battle actions");
}

/// **W5 — the Route 8 gate doorstep, and it is a stall rather than a wasted turn.**
///
/// Leaving the gate's east door puts the player on Route 8 at (9, 9), which is itself a warp entry
/// back into the gate. Going straight back in never worked: the agent walked left and right on the
/// spot until the 60 s movement bound gave up, reported it as "there is no route to the warp to
/// Route8Gate", and the deployed run of 2026-09-02 read that as a pathfinder bug and filed an issue.
///
/// ⚠️ **Both halves of the answer are in `home/overworld.asm` and neither is about routing.** A
/// warp entry whose own tile is not in the tileset's warp list only fires under `ExtraWarpCheck`,
/// which on an overworld map needs a **warp carpet tile in front of the player and a direction
/// held**. Route 8's two east entries are raw $2C at (9, 9) and $39 at (9, 10); west of (9, 10) is
/// $4B, which is in the facing-left carpet list, and west of (9, 9) is $17, which is in no list at
/// all. So (9, 9) is a door the cartridge will not open from Route 8 by any approach, and the row
/// for it is now dropped in favour of the one beside it; and the way in is one held button rather
/// than the step-off/step-back the agent used to emit, which is a two-step route the agent re-derives
/// and re-heads every tick.
///
/// The four steps are the deployed sequence: in, out, and straight back in again.
#[test]
fn the_route_8_gate_can_be_re_entered_from_its_own_doorstep() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-lavender.bin"),
        Duration::from_secs(600),
        vec![
            PolicyStep::enter(Map::Route8),
            PolicyStep::enter(Map::Route8Gate),
            PolicyStep::enter(Map::Route8),
            PolicyStep::enter(Map::Route8Gate),
        ],
    );
    fixture.try_run_until(|state| state.map.map == Map::Route8Gate).expect("into the gate");
    // ⚠️ **Wait for the landing square, not merely for the map.** `try_run_until` returns on the
    // first frame the predicate holds, and during a warp the map id changes a few frames before
    // `wXCoord`/`wYCoord` settle, so a bare map test reads a coordinate that is on neither side of
    // the gate. Route 8's four entries into it are the only squares this can end on.
    let out = fixture.try_run_until(|state| state.map.map == Map::Route8 && matches!(
        (state.map.player_position.x, state.map.player_position.y), (2 | 9, 9 | 10)))
        .expect("out of the gate again");
    // And assert *which* door, or the test passes on a run that left by the west one and never
    // stood on the square this is about.
    assert_eq!(out.map.player_position.x, 9,
               "the east door puts the player on the entry the cartridge will not open");

    fixture.try_run_until(|state| state.map.map == Map::Route8Gate)
        .expect("and straight back in from the doorstep, which is what never used to happen");
}


/// **The deployed run of 2026-09-03, crossing Route 21 to Cinnabar Island.** Three defects met on
/// one map, and the visible symptom was a run that looked frozen: the same walk issued over and over,
/// a paid request each time, while the player drifted a few tiles south between them.
///
/// Route 21 is ninety tiles of sea with two four-tile islands in it, at y = 25/26. The search priced
/// a step onto water at 1, exactly like a step onto grass, so the route from the south end to Pallet
/// Town went straight up x = 7 — over both islands. Each one meant stepping ashore, walking two
/// tiles, and mounting Surf again to leave, where going round cost *nothing*: the water at x = 8 is
/// the same three steps.
///
/// The mounts were then far worse than slow. `ItemUseSurfboard` ends by simulating a joypad press to
/// step the player onto the water, which runs as `GameMode::Script`, and `Surfing` was not on
/// `assert_script_state`'s exemption list — so the agent handed itself to `RunningScript` from a state
/// whose rollback window is 40 ms, the step outlasted that, the script "committed", and the walk was
/// dropped in favour of `AwaitingOverworldAction`. The policy was asked again, said the same thing,
/// and the whole crossing went that way.
///
/// What this asserts is the routing half: **the way north touches no land at all.** The mount half is
/// [`a_surf_mount_hands_the_walk_back_to_itself_rather_than_to_the_policy`], which needs a player
/// standing on dry ground and so cannot be this fixture.
///
/// The fixture is the deployed run's own checkpoint, at (7, 72) mid-crossing and mid-battle; like
/// `pocket-route14.bin` it is evidence rather than a link in the leg chain, and the property it
/// carries is *which map*, so it must not be re-cut somewhere tidier.
#[test]
fn a_water_route_does_not_climb_out_onto_route_21s_islands() {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(include_bytes!("../data/route21-islands.bin")).expect("the Route 21 fixture loads");
    let mut cache = MapMetadataCache::default();
    let map = PokemonApi::with_cache(&mut gb, &mut cache).game_state().expect("a map to route on").map;
    assert_eq!(map.map, Map::Route21);
    assert!(map.can_surf, "the fixture is mid-crossing, so the search must believe it can surf");

    let north = map.water_connection_action(Map::PalletTown)
        .expect("the way back to Pallet Town is a water crossing and it is reachable");
    // Walk the route and collect every square it puts the player on. The last button is the press
    // *into* the connection, which steps off the map, so it is dropped.
    let mut at = map.player_position;
    let ashore: Vec<(Point8, Option<MetaTile>)> = north.route[..north.route.len() - 1].iter()
        .map(|b| {
            at = match b {
                JoypadButton::Up    => Point8 { x: at.x, y: at.y - 1 },
                JoypadButton::Down  => Point8 { x: at.x, y: at.y + 1 },
                JoypadButton::Left  => Point8 { x: at.x - 1, y: at.y },
                _                   => Point8 { x: at.x + 1, y: at.y },
            };
            (at, map.tile_at_checked(at))
        })
        .filter(|(_, t)| !matches!(t, Some(MetaTile::Water) | Some(MetaTile::ConnectionWater(_))))
        .collect();
    assert!(ashore.is_empty(),
            "the crossing steps ashore at {ashore:?}, which is a Surf mount apiece to leave again — \
             the islands at y = 25/26 are what the search's mount price is for");
}

/// **The other half of the Route 21 crossing: a mount must not end the walk it is part of.**
///
/// Every other menu driver in `agent.rs` is entered from `Idle` because the policy asked for it, so
/// dropping back to `Idle` afterwards costs nothing. `Surfing` is the exception — the route follower
/// enters it mid-walk, without anybody deciding anything — and dropping to `Idle` there put the
/// identical question back to the policy: free for the scripted one, a whole request against a 100 k
/// token history for the model. `Surfing::resume` carries the walk through the mount; without it this
/// reads `move→ surf idle wait move→` and the extra `wait` is the request.
///
/// ⚠️ **It does not cover the `assert_script_state` hijack, though that is the same bug's third
/// cause** — Pallet Town's people all `STAY`, so `wScriptedNPCWalkCounter` is zero and the mount is
/// never seen as a script here. [`a_surf_mount_is_not_taken_over_by_the_script_handler`] is that half,
/// on a map where somebody walks.
///
/// ⚠️ **The settle is load-bearing and its absence looks like success.** Without
/// [`MOUNT_SETTLE_TICKS`] the walk *is* handed back, and then aborted a tick later by "<mon> got on!"
/// being drawn on top of it — so the sequence below reads `move→ surf move→ text wait move→`, which
/// still ends in an extra poll. Asserting arrival alone would pass on that.
///
/// `postgame-fishing.bin` is the fixture because the player is standing on Pallet Town's dry land
/// with Surf in the party, which is where the deployed run was when it crossed.
#[test]
fn a_surf_mount_hands_the_walk_back_to_itself_rather_than_to_the_policy() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/postgame-fishing.bin"),
        Duration::from_secs(300),
        vec![PolicyStep::enter(Map::Route21)],
    );
    assert_eq!(fixture.game_state().map.map, Map::PalletTown);

    // The distinct agent states from the decision to the landing, in order. The sequence is the
    // assertion: `wait` is `AwaitingOverworldAction`, which is where the policy is asked, so counting
    // them counts the requests a model would have paid for.
    let mut seen: Vec<String> = vec![];
    let mut arrived = false;
    for _ in 0..12_000 {
        fixture.step();
        let state = fixture.agent.state_debug();
        if seen.last() != Some(&state) { seen.push(state); }
        if fixture.try_game_state().map(|g| g.map.map) == Ok(Map::Route21) { arrived = true; break }
    }
    assert!(arrived, "the crossing has to finish, or the sequence below is about nothing: {seen:?}");
    assert!(seen.iter().any(|s| s == "surf"),
            "the walk had to mount Surf to leave Pallet Town, or this proves nothing: {seen:?}");
    assert_eq!(seen.first().map(String::as_str), Some("wait"),
               "the crossing opens on the one decision that starts it: {seen:?}");
    assert!(!seen[1..].iter().any(|s| s.starts_with("wait")),
            "the whole crossing is one decision: a second `wait` is the walk being thrown away and \
             the identical question put back to the policy, which is a paid request. {seen:?}");
}

/// **Cinnabar Island's gym doorstep, and the general rule it is the test case for.**
///
/// Found by the mount price above: once a Surf mount cost something, the walk from the Pokémon Centre
/// to the Pokémon Mansion stopped going round by sea and went overland, over raw (18, 4) — the square
/// below the gym door. `CinnabarIslandDefaultScript` watches for exactly that square, and without the
/// Secret Key prints "The door is locked..." and simulates a Down press to shove the player off it.
/// The walk is stopped by the text box, re-planned identically, and loops for ever: 100 iterations
/// before the probe was cut off.
///
/// ⚠️ **The first fix was a hard-coded obstacle at (18, 4) and it was the wrong shape.** Two reasons,
/// and the second is the one that matters. It is one square of hand-transcribed ROM knowledge and
/// `grep StartSimulatingJoypadStates pokered/scripts` finds a dozen more of the same kind — the
/// Route 22 gate, Route 23's badge checkpoints, Viridian City, the Museum — so the table would never
/// have been finished. And the square is not reached by one route: the walk that steps on it and the
/// walk that avoids it are **both 29 steps**, differing by a one-tile dogleg, so which one comes out
/// is a tie-break and any unrelated change can flip it. Coming at it from a different direction,
/// after a battle, or from a model's own choice of row would all have gone straight past the fix.
///
/// So what is asserted here is the general mechanism: the agent is walked back off the square, sees
/// that it was put down exactly where it stepped from, and treats it as a wall for the rest of this
/// visit (`PokemonAgent::turned_back_tiles`). One interrupted walk, then a route that arrives.
#[test]
fn a_square_the_game_walks_you_back_off_is_learned_and_routed_around() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-cinnabar.bin"),
        Duration::from_secs(300),
        vec![PolicyStep::enter(Map::CinnabarPokecenter),
             PolicyStep::enter(Map::CinnabarIsland),
             PolicyStep::enter(Map::PokemonMansion1F)],
    );
    // The fixture is before the Mansion, so the key cannot be in the bag — if it ever is, the gym
    // doorstep is ordinary floor and this test is about nothing.
    assert!(!fixture.game_state().bag.iter()
                .any(|i| i.id == crate::pokemon::item::ItemId::SecretKey),
            "the Secret Key is inside the Mansion, so a fixture standing outside it must not hold one");

    let mut learned = vec![];
    for _ in 0..40_000 {
        fixture.step();
        learned.extend(fixture.agent.drain_events().into_iter().map(|e| format!("{e}"))
                           .filter(|e| e.contains("walked you back")));
        if fixture.agent.policy_steps_remaining() == Some(0) { break }
    }
    assert_eq!(fixture.try_game_state().map(|g| g.map.map), Ok(Map::PokemonMansion1F),
               "the walk to the Mansion has to arrive; over the doorstep it is stopped by \
                \"The door is locked...\" and re-planned identically for ever");
    // ⚠️ Arrival alone would also pass if the router simply never chose that square — which is a
    // tie-break away from being true again. The learning is the thing under test.
    assert!(learned.iter().any(|e| e.contains("(18, 5)")),
            "the doorstep has to be recognised from the shove rather than avoided by luck: {learned:?}");
}

/// **The mount must not be handed to `RunningScript`, and whether it *is* depends on the map's NPCs.**
///
/// `ItemUseSurfboard` finishes with `.makePlayerMoveForward`, which sets `wStatusFlags5`'
/// `BIT_SCRIPTED_MOVEMENT_STATE` and a simulated button press to step the player onto the water.
/// `read_game_mode` calls that `GameMode::Script` when `wScriptedNPCWalkCounter` is also non-zero —
/// and that counter "cycles 8→1 and never resets to 0", so it is non-zero on any map where an NPC has
/// walked. ⚠️ **That is why this needed a second fixture.**
/// [`a_surf_mount_hands_the_walk_back_to_itself_rather_than_to_the_policy`] mounts in Pallet Town,
/// whose people all `STAY`, so the counter is zero and the hijack never fires there — it went green
/// against the unfixed agent. Cinnabar Island has a `WALK`ing girl, so it fires every time, which is
/// where the deployed run met it.
///
/// Hijacked, the mount is replaced by `RunningScript` on a 40 ms rollback it cannot possibly beat, the
/// script "commits", and the driver is thrown away mid-menu: the sequence becomes
/// `surf script text script` and the agent finishes the crossing by reading its own mount message off
/// the screen. Exempting `Surfing` in `assert_script_state` is what this asserts.
#[test]
fn a_surf_mount_is_not_taken_over_by_the_script_handler() {
    let mut fixture = TestFixture::new(
        include_bytes!("../data/at-cinnabar.bin"),
        Duration::from_secs(300),
        // Cinnabar's east shore *is* the seam into Route 20, so the mount's own step crosses it —
        // there is no walk left to resume, which is the other half of why this is a separate case.
        vec![PolicyStep::enter(Map::Route20)],
    );
    assert_eq!(fixture.game_state().map.map, Map::CinnabarIsland);

    let mut seen: Vec<String> = vec![];
    let mut arrived = false;
    for _ in 0..12_000 {
        fixture.step();
        let state = fixture.agent.state_debug();
        if seen.last() != Some(&state) { seen.push(state); }
        if fixture.try_game_state().map(|g| g.map.map) == Ok(Map::Route20) { arrived = true; break }
    }
    assert!(arrived, "the crossing has to finish: {seen:?}");
    let mount = seen.iter().position(|s| s == "surf")
        .unwrap_or_else(|| panic!("it has to mount Surf to get there: {seen:?}"));
    // ⚠️ **From the mount onward, not from the decision.** Getting to the shore crosses the gym
    // doorstep, which costs one text box and one re-plan while the agent learns it
    // (`a_square_the_game_walks_you_back_off_is_learned_and_routed_around`) — a different fact, and
    // asserting over the whole sequence made this test fail for that instead.
    assert!(!seen[mount..].iter().any(|s| s == "script" || s == "text"),
            "the mount drives its own menus and ends in its own scripted step, so nothing between \
             it and the landing may be `script` or `text`: {seen:?}");
}


