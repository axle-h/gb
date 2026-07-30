//! Tests for workstream `pc_box` — see `docs/postgame-coverage-plan.md` §6 and
//! [`crate::pokemon::postgame::pc_box`].

#[allow(unused_imports)]
use super::super::*;

/// Workstream A's entry fixture (§9): all 8 badges, party of 4, bag 14/20, `wBoxCount = 0`, and the
/// player already standing in the Viridian Pokémon Center — one tile-walk from the PC at (13,3). Not
/// `post-hall-of-fame.bin`, which is a cutscene; see the Phase 0 §11 entries.
const PHASE0: &[u8] = include_bytes!("../../data/postgame-phase0.bin");

/// The PC every test here uses, and the map it is on.
const PC_MAP: Map = Map::ViridianPokecenter;

/// Slowpoke, the party's fourth member and its only non-essential one: Articuno/Venusaur/Vaporeon
/// carry Surf, Strength, Cut and every usable attack between them, so Slowpoke is what gets banked.
const BANKED_SLOT: u8 = 3;
const BANKED: PokemonSpecies = PokemonSpecies::Slowpoke;

/// Screens the agent showed, deduplicated consecutively — the same "log every distinct screen" idiom
/// task 0.4 used. Text draws a character at a time, so most captured frames are half-rendered.
fn screens_while(fixture: &mut TestFixture, ticks: u32, done: impl Fn(&mut TestFixture) -> bool) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..ticks {
        fixture.step();
        if let Some(text) = fixture.api().on_screen_text(false) {
            if !text.is_empty() && seen.last() != Some(&text) {
                seen.push(text);
            }
        }
        if done(fixture) { break; }
    }
    seen
}

fn party_count(fixture: &mut TestFixture) -> u8 {
    fixture.api().mmu().read_pointer(&pokered_symbols::wPartyCount)
}

fn box_count(fixture: &mut TestFixture) -> u8 {
    fixture.api().mmu().read_pointer(&pokered_symbols::wBoxCount)
}

fn current_box(fixture: &mut TestFixture) -> u8 {
    crate::pokemon::postgame::pc_box::current_box_num(fixture.api().mmu())
}

// ⚠️ **Do not call `step_until_exhausted` in a test that checks intermediate states.** A `UsePcBox`
// step pops the moment the driver takes over (like `UseItemPc` and `MovePokemonToFront`), so the queue
// empties when the *last* step is issued — by which time every earlier operation has already run to
// completion. A three-step chain therefore blows straight past the states in the middle. Chain
// `run_until` instead: it checks its condition every tick while the policy keeps advancing.

/// **Task A2** — reach the `BILL's PC` submenu *deliberately*. Emulates ~2 min (≈5 s wall clock).
///
/// Task 0.4 already saw this submenu, but only because the agent's generic text-advance mashed A and
/// fell into the parent menu's first entry. This proves the driver navigates there on purpose: the
/// parent menu (`LOG OFF`) has to appear **before** the box submenu (`SEE YA!`), which is what tells
/// a deliberate selection apart from a lucky one.
///
/// A2 warns — correctly — that the parent menu's entry list varies with progress, so the index must
/// not be hard-coded. It is worth being precise about *which* index: `DisplayPCMainMenu` appends
/// `PROF.OAK's PC` only with `EVENT_GOT_POKEDEX` and `<PKMN>LEAGUE` only with `wNumHoFTeams != 0`, and
/// both come **after** the first two entries — so Bill's PC at 0 is safe and it is `LOG OFF` that
/// moves. What is *not* safe is the label: without `EVENT_MET_BILL` entry 0 reads `SOMEONE's PC`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_open_bills_pc() {
    let mut fixture = TestFixture::new(PHASE0, Duration::from_mins(10), vec![
        PolicyStep::deposit_pokemon(BANKED_SLOT, PC_MAP),
    ]);

    // The step pops the moment the driver takes over, so watch the screen, not the queue.
    fixture.step_until_exhausted();
    let seen = screens_while(&mut fixture, 90 * 50, |f| box_count(f) > 0);
    for screen in &seen {
        println!("  screen: {screen}");
    }

    let parent = seen.iter().position(|t| t.contains("LOG OFF"))
        .unwrap_or_else(|| panic!("PC parent menu never appeared; screens: {seen:#?}"));
    let submenu = seen.iter().position(|t| t.contains("SEE YA!"))
        .unwrap_or_else(|| panic!("Bill's PC submenu never appeared; screens: {seen:#?}"));
    assert!(parent < submenu, "the driver must select BILL's PC from the parent menu, not arrive first");

    // The submenu, in full, as `BillsPCMenuText` (`engine/pokemon/bills_pc.asm:341`) spells it.
    let full = seen.iter().filter(|t| t.contains("SEE YA!")).max_by_key(|t| t.len()).unwrap();
    println!("Bill's PC menu: {full}");
    for entry in ["WITHDRAW", "DEPOSIT", "RELEASE", "CHANGE BOX", "SEE YA!"] {
        assert!(full.contains(entry), "{entry:?} missing from the box menu {full:?}");
    }
}

/// **Task A3** — deposit a party member into the box. Emulates ~2 min (≈5 s wall clock).
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_deposit_a_pokemon() {
    let mut fixture = TestFixture::new(PHASE0, Duration::from_mins(10), vec![
        PolicyStep::deposit_pokemon(BANKED_SLOT, PC_MAP),
    ]);

    assert_eq!(party_count(&mut fixture), 4);
    assert_eq!(box_count(&mut fixture), 0, "the entry fixture's box has never been opened");

    fixture.step_until_exhausted();
    let state = fixture.run_until(|s| s.boxed_pokemon.len() == 1);

    assert_eq!(state.pokemon.len(), 3, "the deposited mon should have left the party");
    assert!(state.pokemon.iter().all(|p| p.species != BANKED), "{BANKED:?} is still in the party");
    assert_eq!(state.boxed_pokemon[0].species, BANKED);
    assert_eq!(state.boxed_pokemon[0].level, 30, "the banked mon's level should survive the transfer");
    println!("deposited {:?} lv{} — party {}, box {}",
        state.boxed_pokemon[0].species, state.boxed_pokemon[0].level,
        state.pokemon.len(), state.boxed_pokemon.len());
}

/// **Task A4** — the same mon round-trips back into the party. Emulates ~3 min (≈7 s wall clock).
///
/// A bare withdraw would prove much less than a round trip: the interesting property is that the two
/// are inverses and the party comes back to where it started, carrying the same mon at the same level.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn pokemon_round_trips_through_the_box() {
    let mut fixture = TestFixture::new(PHASE0, Duration::from_mins(15), vec![
        PolicyStep::deposit_pokemon(BANKED_SLOT, PC_MAP),
        PolicyStep::withdraw_pokemon(0, PC_MAP),
    ]);

    // Out…
    let banked = fixture.run_until(|s| s.boxed_pokemon.len() == 1);
    assert_eq!(banked.pokemon.len(), 3);
    println!("deposited: party {} box {}", banked.pokemon.len(), banked.boxed_pokemon.len());

    // …and back. The withdraw step is still queued; it is issued once the driver returns to `Idle`.
    let state = fixture.run_until(|s| s.boxed_pokemon.is_empty() && s.pokemon.len() == 4);
    let back = state.pokemon.get(3).expect("withdrawn mon should be appended to the party");
    assert_eq!(back.species, BANKED, "a different mon came back");
    assert_eq!(back.level, 30, "level should survive the round trip");
    // Withdrawing recomputes the stats from the stored EVs/DVs, so the mon must come back usable.
    assert!(back.stats.hp > 0 && back.current_hp > 0, "{back:?} came back with no HP");
    println!("withdrew: {:?} lv{} {}/{}hp", back.species, back.level, back.current_hp, back.stats.hp);
}

/// **Task A5** — switch boxes. Emulates ~4 min (≈9 s wall clock).
///
/// ⚠️ The plan's warning is right and then some: `ChangeBox` (`engine/menus/save.asm:358`) prints a
/// YES/NO, **saves the game**, and on the first change ever also calls `EmptyAllSRAMBoxes`. The mon
/// deposited a moment earlier survives that — the wipe happens *before* the open box is copied out to
/// SRAM — which is what the second half of this test checks by switching back and finding it again.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_change_box() {
    let mut fixture = TestFixture::new(PHASE0, Duration::from_mins(20), vec![
        PolicyStep::deposit_pokemon(BANKED_SLOT, PC_MAP),
        PolicyStep::change_box(1, PC_MAP),
        PolicyStep::change_box(0, PC_MAP),
    ]);

    assert_eq!(current_box(&mut fixture), 0, "the entry fixture has never changed box");

    fixture.run_until(|s| s.boxed_pokemon.len() == 1);

    // Box 2 is a different, empty box — and the deposit is no longer visible, because only the open
    // box lives in WRAM.
    let state = fixture.run_until(|s| s.current_box == 1);
    assert!(state.boxed_pokemon.is_empty(), "box 2 should be empty, holds {:?}", state.boxed_pokemon);
    println!("switched to box {} — empty", state.current_box + 1);

    // Back to box 1, and the banked mon is still there: the change wrote it out to SRAM rather than
    // losing it, and `EmptyAllSRAMBoxes` ran before that copy, not after.
    let state = fixture.run_until(|s| s.current_box == 0 && !s.boxed_pokemon.is_empty());
    assert_eq!(state.boxed_pokemon.len(), 1);
    assert_eq!(state.boxed_pokemon[0].species, BANKED, "box 1's contents did not survive the switch");
    println!("switched back to box {} — {:?} still banked", state.current_box + 1, BANKED);
}

/// **Task A6** — release a boxed Pokémon. Emulates ~3 min (≈7 s wall clock).
///
/// Release has no confirm-menu of its own: the mon list is followed straight by a YES/NO
/// (`OnceReleasedText` → `YesNoChoice`), where the deposit/withdraw path gets a
/// `DEPOSIT|WITHDRAW / STATS / CANCEL` box instead. Both are handled by the one driver.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_release_a_pokemon() {
    let mut fixture = TestFixture::new(PHASE0, Duration::from_mins(15), vec![
        PolicyStep::deposit_pokemon(BANKED_SLOT, PC_MAP),
        PolicyStep::release_pokemon(0, PC_MAP),
    ]);

    fixture.run_until(|s| s.boxed_pokemon.len() == 1);
    println!("banked {BANKED:?}, releasing it");

    let state = fixture.run_until(|s| s.boxed_pokemon.is_empty());
    assert_eq!(box_count(&mut fixture), 0, "wBoxCount should have dropped");
    // Released, not withdrawn: it must not have reappeared in the party either.
    assert_eq!(state.pokemon.len(), 3, "release must not put the mon back in the party");
    assert!(state.pokemon.iter().all(|p| p.species != BANKED), "{BANKED:?} came back into the party");
    println!("released — party {}, box empty", state.pokemon.len());
}

/// **Task A7** — the full chain in one run, checking the counts at every stage, and the workstream's
/// output fixture. Emulates ~6 min (≈14 s wall clock). Commit target: `postgame-pc-box.bin`.
///
/// Deposit → change box → change back → withdraw, with `(party, box, open box)` asserted between each.
/// The interesting property is that the three operations compose: a mon banked in box 1 is still there
/// after a detour through box 2, and comes back into the party unharmed.
///
/// **What the fixture is for.** It ends with the party *restored* — Slowpoke carries Strength and Dig,
/// and nothing else in the party knows either — so this is `postgame-phase0.bin` plus two things a
/// workstream can now rely on:
///
/// 1. **Box storage works and is initialised.** `BIT_HAS_CHANGED_BOXES` is set, so the SRAM boxes have
///    already been emptied and no later `change_box` triggers the one-time `EmptyAllSRAMBoxes`.
/// 2. **Party space is one step away.** `PolicyStep::deposit_pokemon(slot, map)` frees a slot at any
///    Pokémon Center, so a stream that wants to hold more than six no longer has to arrange it here.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_round_trip_a_pokemon_through_two_boxes() {
    let mut fixture = TestFixture::new(PHASE0, Duration::from_mins(25), vec![
        PolicyStep::deposit_pokemon(BANKED_SLOT, PC_MAP),
        PolicyStep::change_box(1, PC_MAP),
        PolicyStep::change_box(0, PC_MAP),
        PolicyStep::withdraw_pokemon(0, PC_MAP),
    ]);

    let stage = |f: &mut TestFixture| (party_count(f), box_count(f), current_box(f));
    assert_eq!(stage(&mut fixture), (4, 0, 0), "entry: party 4, box 1 empty and never opened");

    fixture.run_until(|s| s.boxed_pokemon.len() == 1);
    assert_eq!(stage(&mut fixture), (3, 1, 0), "after deposit");

    fixture.run_until(|s| s.current_box == 1);
    assert_eq!(stage(&mut fixture), (3, 0, 1), "in box 2: a different, empty box");

    fixture.run_until(|s| s.current_box == 0 && !s.boxed_pokemon.is_empty());
    assert_eq!(stage(&mut fixture), (3, 1, 0), "back in box 1, mon still banked");

    let state = fixture.run_until(|s| s.pokemon.len() == 4 && s.boxed_pokemon.is_empty());
    assert_eq!(stage(&mut fixture), (4, 0, 0), "after withdraw: back where we started");

    let back = state.pokemon.get(3).expect("the withdrawn mon");
    assert_eq!(back.species, BANKED);
    assert_eq!(back.level, 30);
    assert!(back.moves.iter().flatten().any(|m| m.name == PokemonMoveName::Strength),
        "{BANKED:?} must come back still knowing Strength — nothing else in the party does");
    assert_eq!(state.badges.bits(), 255, "badges lost");
    assert_eq!(state.map.map, PC_MAP);
    println!("round trip complete: party {}, box empty, standing at the {PC_MAP} PC", state.pokemon.len());

    fixture.save_state_named("src/pokemon/data/postgame-pc-box.bin").unwrap();
}

/// **Task A1** — the box reader decodes a `box_struct`, not just an empty box.
///
/// The probe printing `box1: empty` from every committed fixture is A1's stated observable, but an
/// empty box is exactly what a reader with the offsets wrong would also print. So this plants two
/// synthetic members in `wBoxMons` and reads them back.
///
/// The offsets under test are the ones a `box_struct` does *not* share with a `party_struct`: the
/// level lives at **offset 3** (`BoxLevel`) where a party mon keeps a duplicate of its offset-33
/// `Level`, and the struct stops after `PP` at 32 — read at a party mon's 44-byte stride, slot 1
/// would land 11 bytes into slot 0's neighbour. Default tier: no emulation, only RAM.
#[test]
fn reads_a_boxed_pokemon_out_of_wram() {
    use crate::pokemon::move_name::PokemonMoveName;
    use crate::pokemon::postgame::pc_box::{read_current_box, BOX_CAPACITY};
    use crate::pokemon::symbols::DmgPointer;

    let mut mmu = MMU::from_rom(crate::pokemon::roms::POKERED).unwrap();
    let write = |mmu: &mut MMU, ptr: DmgPointer, offset: u16, value: u8| {
        mmu.write(ptr.address + offset, value);
    };

    // Two members: the count, the FF-terminated species list, then the two 33-byte structs.
    write(&mut mmu, pokered_symbols::wBoxCount, 0, 2);
    write(&mut mmu, pokered_symbols::wBoxSpecies, 0, PokemonSpecies::Lapras as u8);
    write(&mut mmu, pokered_symbols::wBoxSpecies, 1, PokemonSpecies::Omanyte as u8);
    write(&mut mmu, pokered_symbols::wBoxSpecies, 2, 0xFF);

    const MON: u16 = 0x21;
    for (slot, (species, hp, level, first_move, pp)) in [
        (PokemonSpecies::Lapras, 137u16, 34u8, PokemonMoveName::Surf, 15u8),
        (PokemonSpecies::Omanyte, 21, 5, PokemonMoveName::WaterGun, 25),
    ].into_iter().enumerate()
    {
        let base = slot as u16 * MON;
        write(&mut mmu, pokered_symbols::wBoxMons, base, species as u8);
        write(&mut mmu, pokered_symbols::wBoxMons, base + 1, (hp >> 8) as u8);
        write(&mut mmu, pokered_symbols::wBoxMons, base + 2, hp as u8);
        write(&mut mmu, pokered_symbols::wBoxMons, base + 3, level);
        write(&mut mmu, pokered_symbols::wBoxMons, base + 4, 0); // no status
        write(&mut mmu, pokered_symbols::wBoxMons, base + 8, first_move as u8);
        write(&mut mmu, pokered_symbols::wBoxMons, base + 29, pp);
        for m in 1..4u16 {
            write(&mut mmu, pokered_symbols::wBoxMons, base + 8 + m, 0);
        }
    }

    let boxed = read_current_box(&mmu);
    assert_eq!(boxed.len(), 2, "box should hold both planted members");

    assert_eq!(boxed[0].species, PokemonSpecies::Lapras);
    assert_eq!(boxed[0].level, 34, "level must come from BoxLevel at offset 3");
    assert_eq!(boxed[0].current_hp, 137);
    assert_eq!(boxed[0].moves[0].map(|m| (m.name, m.pp)), Some((PokemonMoveName::Surf, 15)));
    assert!(boxed[0].moves[1].is_none());

    // The second member is what pins the stride.
    assert_eq!(boxed[1].species, PokemonSpecies::Omanyte);
    assert_eq!(boxed[1].level, 5);
    assert_eq!(boxed[1].current_hp, 21);
    assert_eq!(boxed[1].moves[0].map(|m| m.name), Some(PokemonMoveName::WaterGun));

    // `wBoxCount` is trusted only up to the box's capacity, so a corrupt count can't run off the end
    // into `wBoxMonOT`. Fill the remaining slots first, or the read would stop at the first blank one.
    for slot in 2..BOX_CAPACITY as u16 {
        write(&mut mmu, pokered_symbols::wBoxMons, slot * MON, PokemonSpecies::Omanyte as u8);
    }
    write(&mut mmu, pokered_symbols::wBoxCount, 0, 200);
    assert_eq!(read_current_box(&mmu).len(), BOX_CAPACITY);

    // And a blank slot inside the count ends the list rather than shifting the slots after it —
    // the entry at index `i` must always be box slot `i`, because that is what the menus address.
    write(&mut mmu, pokered_symbols::wBoxMons, 5 * MON, 0x00);
    assert_eq!(read_current_box(&mmu).len(), 5, "read should stop at the undecodable slot");
}
