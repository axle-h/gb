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
