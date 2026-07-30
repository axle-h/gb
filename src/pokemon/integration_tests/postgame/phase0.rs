//! Phase 0 of `docs/postgame-coverage-plan.md` — the foundation every workstream builds on.

use super::super::*;
use crate::pokemon::encoding::GameMode;

/// Drive `post-hall-of-fame.bin` forward until the player is standing in a playable overworld again,
/// and return the state it lands in.
///
/// The Hall of Fame is **not** a playable state, which is the thing the plan got wrong: pokered's
/// `HallOfFameResetEventsAndSaveScript` runs the ceremony and the credits, saves, then does
/// `WaitForTextScrollButtonPress` → `jp Init` — a soft reset to the title screen. The player has to
/// come back in through the main menu, and `main_menu.asm:116-125` special-cases exactly this: with
/// `wNumHoFTeams != 0` and a saved `wCurMap == HALL_OF_FAME`, CONTINUE does a dungeon warp instead of
/// restoring the saved position, which is how a Champion wakes up back home.
///
/// No title-screen or main-menu driver is needed for any of it: CONTINUE is the first entry, so a
/// plain A-mash carries the whole sequence. Mashing stops the moment the overworld comes back on a
/// map that isn't the Hall of Fame, so nothing gets talked to on arrival.
pub fn drive_out_of_hall_of_fame(fixture: &mut TestFixture) -> GameState {
    let mut tick = 0u32;
    loop {
        let (mode, map) = {
            let api = fixture.api();
            (api.game_mode(), api.mmu().read_pointer(&pokered_symbols::wCurMap))
        };
        if mode == Some(GameMode::Overworld) && map != Map::HallOfFame as u8 {
            fixture.api().release_all_buttons();
            return fixture.game_state();
        }
        // Mash: press one tick, release the next, so every input is a fresh rising edge.
        {
            let mut api = fixture.api();
            if tick % 2 == 0 { api.press_button(JoypadButton::A); } else { api.release_all_buttons(); }
        }
        fixture.step();
        tick += 1;
    }
}

/// **Task 0.35** — the postgame root fixture. Emulates ~3 min of game time (≈8 s wall clock).
///
/// `post-hall-of-fame.bin` is captured on *arrival* in the Hall of Fame, so it is three minutes of
/// ceremony, credits and a soft reset away from anything a workstream can drive. This produces the
/// state §4.2 of the plan assumed that file already was: the player standing in the overworld with
/// everything won and nothing left to do. Commit target: `postgame-post-credits.bin`.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_walk_out_of_the_hall_of_fame() {
    let mut fixture = TestFixture::new(
        include_bytes!("../../data/post-hall-of-fame.bin"),
        Duration::from_mins(20),
        vec![],
    );

    let state = drive_out_of_hall_of_fame(&mut fixture);
    println!("post-credits: {} @ {}", state.map.map, state.map.player_position);

    // The special warp lands the player outside their own front door in Pallet Town.
    assert_eq!(state.map.map, Map::PalletTown);
    assert_eq!(state.mode, GameMode::Overworld);
    // The reset must not have cost anything: this is a save/reload, not a new game.
    assert_eq!(state.badges.bits(), 255, "badges lost across the reset");
    assert_eq!(state.pokemon.len(), 4, "party lost across the reset");
    assert!(state.money > 0, "money lost across the reset");

    fixture.save_state_named("src/pokemon/data/postgame-post-credits.bin").unwrap();
}

/// The postgame root, for every Phase 0 test after 0.35.
const POST_CREDITS: &[u8] = include_bytes!("../../data/postgame-post-credits.bin");

/// **Task 0.4** — stand at a Pokémon Center PC and open it. Emulates ~4 min (≈11 s wall clock).
///
/// No storage logic yet: this only proves the agent can reach a PC now that
/// [`crate::pokemon::tile_map::pc_locations_for`] knows where they are (task 0.3), and it pins down
/// what the parent menu actually contains post-Champion — which 0.5 and workstream A both have to
/// navigate, and which A2 correctly flags as *not* safe to hard-code an index into.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_open_the_pokemon_center_pc() {
    // Viridian is the nearest centre to where the credits drop the player.
    let mut fixture = TestFixture::new(POST_CREDITS, Duration::from_mins(20), vec![
        PolicyStep::enter(Map::Route1),
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::ViridianPokecenter),
        PolicyStep::UsePc { map: Map::ViridianPokecenter },
    ]);

    // `UsePc` pops the moment it issues the walk, so drive the queue dry and then watch the screen
    // rather than the queue.
    fixture.step_until_exhausted();

    // Log every distinct screen for the next 30 s of game time. This is the deliverable of 0.4: what
    // the PC menu says, in this save's post-Champion configuration.
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..(30 * 50) {
        fixture.step();
        if let Some(text) = fixture.api().on_screen_text(false) {
            if seen.last() != Some(&text) && !text.is_empty() {
                println!("  screen: {text}");
                seen.push(text);
            }
        }
    }

    // The parent menu, post-Champion, in full. `DisplayPCMainMenu` builds it conditionally:
    // PROF.OAK's PC only once the Pokédex is owned, <PKMN>LEAGUE only once `wNumHoFTeams != 0` —
    // which is exactly why A2 is right that the index must not be hard-coded. Here it is five
    // entries, so PLAYER's PC (what 0.5/0.6 want) is index 1 and BILL's PC (workstream A) is index 0.
    // Text draws a character at a time, so most captured frames are half-rendered menus; take the
    // longest one that is still just the menu (no dialogue printed over it yet).
    let main_menu = seen.iter()
        .filter(|t| t.contains("LOG OFF") && !t.contains("Acc"))
        .max_by_key(|t| t.len())
        .unwrap_or_else(|| panic!("PC main menu never appeared; screens seen: {seen:#?}"));
    println!("PC main menu: {main_menu}");
    for entry in ["BILL's PC", "PROF.OAK's PC", "LEAGUE", "LOG OFF"] {
        assert!(main_menu.contains(entry), "{entry:?} missing from PC menu {main_menu:?}");
    }

    // The agent's generic text-advance keeps mashing A once the menu is up, so it walks straight into
    // the first entry. Harmless here, and it hands 0.5 and workstream A a free look at the next menu
    // down — which matches §6-A3's transcription exactly.
    assert!(
        seen.iter().any(|t| t.contains("DEPOSIT") && t.contains("CHANGE BOX") && t.contains("SEE YA!")),
        "Bill's PC submenu never appeared; screens seen: {seen:#?}"
    );
}

/// TM34 Bide — one of the six TMs sitting in the bag that no workstream has a plan for, and the one
/// `item.rs` already calls "the bag's most useless item". See the §11 entry for 0.1: the bag is a
/// quarter TMs, which is where Phase 0's slack comes from.
const SPARE_TM: ItemId = ItemId::Tm34Bide;

/// **Task 0.5** — deposit an item into PC storage. Emulates ~5 min (≈13 s wall clock).
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_deposit_an_item() {
    let mut fixture = TestFixture::new(POST_CREDITS, Duration::from_mins(20), vec![
        PolicyStep::enter(Map::Route1),
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::ViridianPokecenter),
        PolicyStep::deposit_item(SPARE_TM, 1, Map::ViridianPokecenter),
    ]);

    let before = bag_count(&mut fixture);
    assert_eq!(before, 20, "the postgame root should still have a full bag");
    assert_eq!(fixture.api().bag_item_quantity(SPARE_TM), 1);

    fixture.step_until_exhausted();
    // The step pops the moment the driver takes over, so wait on the effect, not the queue.
    run_until_bag(&mut fixture, before - 1);

    let api = fixture.api();
    assert_eq!(api.bag_item_quantity(SPARE_TM), 0, "{SPARE_TM:?} should have left the bag");
    assert_eq!(api.pc_box_item_quantity(SPARE_TM), 1, "{SPARE_TM:?} should be in PC storage");
    println!("bag {before} → {}, PC storage now holds {SPARE_TM:?}", bag_count(&mut fixture));
}

/// **Task 0.6** — withdraw it again. Same driver, other branch. Emulates ~6 min (≈15 s wall clock).
///
/// A round trip rather than a bare withdraw, because a withdraw on its own proves much less: the
/// interesting property is that the two are inverses and the bag count comes back to where it started.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn item_round_trips_through_pc_storage() {
    let mut fixture = TestFixture::new(POST_CREDITS, Duration::from_mins(30), vec![
        PolicyStep::enter(Map::Route1),
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::ViridianPokecenter),
        PolicyStep::deposit_item(SPARE_TM, 1, Map::ViridianPokecenter),
        PolicyStep::withdraw_item(SPARE_TM, 1, Map::ViridianPokecenter),
    ]);

    let before = bag_count(&mut fixture);
    fixture.step_until_exhausted();

    // Out…
    run_until_bag(&mut fixture, before - 1);
    assert_eq!(fixture.api().pc_box_item_quantity(SPARE_TM), 1, "should be banked");
    println!("deposited: bag {before} → {}", before - 1);

    // …and back. The withdraw step is still queued; it is issued once the driver returns to Idle.
    run_until_bag(&mut fixture, before);
    let api = fixture.api();
    assert_eq!(api.bag_item_quantity(SPARE_TM), 1, "{SPARE_TM:?} should be back in the bag");
    assert_eq!(api.pc_box_item_quantity(SPARE_TM), 0, "PC storage should be empty again");
    println!("withdrew: bag back to {before}");
}

/// The six TMs the save arrives carrying that no workstream in the plan has any use for. Banking all
/// six is where Phase 0's bag slack comes from: 20/20 → 14/20, without giving up a single item any
/// workstream needs. (See the §11 entry for 0.1 — §2's bag listing omitted these entirely.)
const SPARE_TMS: [ItemId; 6] = [
    ItemId::Tm06Toxic, ItemId::Tm11Bubblebeam, ItemId::Tm21MegaDrain,
    ItemId::Tm24Thunderbolt, ItemId::Tm27Fissure, ItemId::Tm34Bide,
];

/// **Task 0.9** — ship the entry fixture every workstream A–H starts from. Emulates ~12 min
/// (≈32 s wall clock). Commit target: `postgame-phase0.bin`.
///
/// Two things happen here, and both matter to whoever picks up a workstream next:
///
/// 1. **Six bag slots are freed** by banking the six spare TMs, so items can actually be picked up.
///    Nothing needed by any workstream is deposited, and anything banked can be fetched back with
///    `PolicyStep::withdraw_item` from any Pokémon Center.
/// 2. **The party is healed.** The credits leave Venusaur at 0 HP and Articuno with no usable attack
///    (every offensive move at 0 PP), which would make the first battle any workstream started
///    behave strangely. The nurse is in the same room as the PC.
#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow — run with --features slow-tests")]
fn can_ship_the_phase0_entry_fixture() {
    let mut steps = vec![
        PolicyStep::enter(Map::Route1),
        PolicyStep::enter(Map::ViridianCity),
        PolicyStep::enter(Map::ViridianPokecenter),
    ];
    steps.extend(SPARE_TMS.map(|tm| PolicyStep::deposit_item(tm, 1, Map::ViridianPokecenter)));
    // Heal last: the deposits leave the player standing at the PC, one tile from the nurse.
    steps.push(PolicyStep::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE));

    let mut fixture = TestFixture::new(POST_CREDITS, Duration::from_mins(40), steps);
    let before = bag_count(&mut fixture);
    assert_eq!(before, 20);

    fixture.step_until_exhausted();
    run_until_bag(&mut fixture, before - SPARE_TMS.len() as u8);

    // Then wait for the heal to land — `Interact` pops when it issues the walk, not when the nurse
    // is done, so gate on the party actually being at full health.
    let state = fixture.run_until(|s| {
        s.mode == GameMode::Overworld && s.pokemon.iter().all(|p| p.current_hp == p.stats.hp)
    });

    let api = fixture.api();
    for tm in SPARE_TMS {
        assert_eq!(api.bag_item_quantity(tm), 0, "{tm:?} should be banked");
        assert_eq!(api.pc_box_item_quantity(tm), 1, "{tm:?} should be in PC storage");
    }
    // Everything a workstream might want must have survived.
    for keep in [ItemId::Hm01Cut, ItemId::Hm03Surf, ItemId::Hm04Strength, ItemId::PokeFlute,
                 ItemId::SilphScope, ItemId::CardKey, ItemId::SecretKey, ItemId::HelixFossil,
                 ItemId::TownMap, ItemId::SSTicket, ItemId::LiftKey] {
        assert!(api.bag_item_quantity(keep) > 0, "{keep:?} must stay in the bag");
    }
    assert_eq!(state.badges.bits(), 255);

    let after = bag_count(&mut fixture);
    println!("phase 0 entry fixture: bag {before} → {after}, party healed, at {} @ {}",
        state.map.map, state.map.player_position);
    assert!(after <= 14, "bag should be well under 20, is {after}");

    fixture.save_state_named("src/pokemon/data/postgame-phase0.bin").unwrap();
}

/// Raw `wNumBagItems` — *not* `GameState::bag`, which drops every id `ItemId` cannot name and so
/// under-reports occupancy against the 20-slot ceiling this whole phase exists to relieve.
fn bag_count(fixture: &mut TestFixture) -> u8 {
    fixture.api().mmu().read_pointer(&pokered_symbols::wNumBagItems)
}

/// Drive until the bag holds exactly `target` items. No step cap — `TestFixture`'s cycle budget is the
/// failsafe, and it fails with a screenshot instead of a bare assertion.
fn run_until_bag(fixture: &mut TestFixture, target: u8) {
    while bag_count(fixture) != target {
        fixture.step();
    }
}
