
use super::super::*;

const PHASE0: &[u8] = include_bytes!("../../data/postgame-phase0.bin");

/// The PC every test here uses, and the map it is on.
const PC_MAP: Map = Map::ViridianPokecenter;

/// Slowpoke, the party's fourth member and its only non-essential one: Articuno/Venusaur/Vaporeon
/// carry Surf, Strength, Cut and every usable attack between them, so Slowpoke is what gets
/// banked.
const BANKED_SLOT: u8 = 3;
const BANKED: PokemonSpecies = PokemonSpecies::Slowpoke;

/// Screens the agent showed, deduplicated consecutively — the same "log every distinct screen"
/// idiom task 0.4 used.
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

// Do not call `step_until_exhausted` in a test that checks intermediate states.

/// Task A2 — reach the `BILL's PC` submenu *deliberately*.
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

/// Task A3 — deposit a party member into the box.
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

/// Task A4 — the same mon round-trips back into the party.
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

    // …and back.
    let state = fixture.run_until(|s| s.boxed_pokemon.is_empty() && s.pokemon.len() == 4);
    let back = state.pokemon.get(3).expect("withdrawn mon should be appended to the party");
    assert_eq!(back.species, BANKED, "a different mon came back");
    assert_eq!(back.level, 30, "level should survive the round trip");
    // Withdrawing recomputes the stats from the stored EVs/DVs, so the mon must come back usable.
    assert!(back.stats.hp > 0 && back.current_hp > 0, "{back:?} came back with no HP");
    println!("withdrew: {:?} lv{} {}/{}hp", back.species, back.level, back.current_hp, back.stats.hp);
}

/// Task A5 — switch boxes.
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

    // Box 2 is a different, empty box — and the deposit is no longer visible, because only the
    // open box lives in WRAM.
    let state = fixture.run_until(|s| s.current_box == 1);
    assert!(state.boxed_pokemon.is_empty(), "box 2 should be empty, holds {:?}", state.boxed_pokemon);
    println!("switched to box {} — empty", state.current_box + 1);

    // Back to box 1, and the banked mon is still there: the change wrote it out to SRAM rather
    // than losing it, and `EmptyAllSRAMBoxes` ran before that copy, not after.
    let state = fixture.run_until(|s| s.current_box == 0 && !s.boxed_pokemon.is_empty());
    assert_eq!(state.boxed_pokemon.len(), 1);
    assert_eq!(state.boxed_pokemon[0].species, BANKED, "box 1's contents did not survive the switch");
    println!("switched back to box {} — {:?} still banked", state.current_box + 1, BANKED);
}

/// Task A6 — release a boxed Pokémon.
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

/// Task A7 — the full chain in one run, checking the counts at every stage, and the workstream's
/// output fixture.
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

/// Task A1 — the box reader decodes a `box_struct`, not just an empty box.
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

    // `wBoxCount` is trusted only up to the box's capacity, so a corrupt count can't run off the
    // end into `wBoxMonOT`.
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
