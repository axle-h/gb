//! Cycling Road against the cartridge, from a rider stopped on Route 17's slope: the ride down it
//! with nothing held, a game continued from a save made there, and a biker's sight. The first two
//! are compared at every square the player reaches: where, when, and every sprite.

use gb::game_boy::GameBoy;
use gb::joypad::JoypadButtonState;
use gb::ram::RAM;
use poke_core::map::Map;
use poke_core::sprite::SpriteFacing;
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::overworld::Overworld;
use pokered::rng::GameRng;
use pokered::systems::overworld::sprites::{SpriteState, Sprites};
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::pokered_symbols as sym;
use super::harness::{clear_events, overworld, Start};
use super::scripts::{self, lockstep_holding, walk_and_answer, Action, Kind};
use super::overworld::Cartridge;
use super::{assert_late, breakpoint, joypad};

/// `wWalkBikeSurfState`'s.
const BIKING: u8 = 1;
/// `wStatusFlags6`'s.
const BIT_ALWAYS_ON_BIKE: u8 = 5;
/// `wCurrentMapScriptFlags`'.
const BIT_CUR_MAP_LOADED_1: u8 = 5;
const PLAYER_DIR_DOWN: u8 = 1 << 2;
const BUDGET: u32 = 1200;

/// A square reached, seen from one machine.
#[derive(Debug)]
struct Square {
    frames: u32,
    /// The cartridge's lag frames so far, which the recreation does not have.
    lag: u32,
    location: (Map, u8, u8, SpriteFacing),
    count: u8,
    sprites: Sprites,
}

/// The fixture's rider, held still with B until the loop polls: the slope answers a poll that finds
/// no direction, A or B with Down.
fn stopped_on_the_slope() -> Cartridge {
    let mut cartridge = Cartridge::from_state(include_bytes!("../pokemon/data/route17-slope.bin"));
    cartridge.gb.hold_buttons(joypad(Joypad::B));
    while !cartridge.frame() {}
    cartridge.gb.hold_buttons(JoypadButtonState::default());
    assert_eq!(cartridge.location().0, Map::Route17);
    assert_eq!(cartridge.read(sym::wWalkBikeSurfState.address), BIKING);
    assert_ne!(cartridge.read(sym::wStatusFlags6.address) & 1 << BIT_ALWAYS_ON_BIKE, 0);
    cartridge
}

/// `press` held until a step begins, then nothing, a frame at a time until `done` says where the
/// player is enough; every square on the way.
fn cartridge_ride(cartridge: &mut Cartridge, press: Option<Joypad>, done: impl Fn((Map, u8, u8)) -> bool) -> Vec<Square> {
    let mut held = press.is_some();
    cartridge.gb.hold_buttons(press.map(joypad).unwrap_or_default());
    let mut last = cartridge.location();
    let mut squares = Vec::new();
    let mut lag = 0;
    for frames in 1..BUDGET {
        cartridge.frame();
        // `DelayFrame` is the loop's only wait, so a VBlank that finds it anywhere else finds a pass
        // that has outrun its frame: a block's worth of map view drawn at the start of every square.
        if !(sym::DelayFrame.address..sym::LoadGBPal.address).contains(&cartridge.gb.return_address()) {
            lag += 1;
        }
        if held && cartridge.read(sym::wWalkCounter.address) != 0 {
            held = false;
            cartridge.gb.hold_buttons(JoypadButtonState::default());
        }
        let location = cartridge.location();
        if (location.0, location.1, location.2) != (last.0, last.1, last.2) {
            last = location;
            squares.push(Square { frames, lag, location, count: cartridge.read(sym::wNumSprites.address), sprites: cartridge.sprites() });
            if done((location.0, location.1, location.2)) {
                return squares;
            }
        }
    }
    panic!("the cartridge never got there: {last:?}");
}

fn recreation_ride(game: &mut Game, press: Option<Joypad>, done: impl Fn((Map, u8, u8)) -> bool) -> Vec<Square> {
    let at = |game: &Game| {
        let location = &game.world().location;
        (location.map, location.x, location.y, location.facing)
    };
    let mut held = press;
    let mut last = at(game);
    let mut squares = Vec::new();
    for frames in 1..BUDGET {
        game.frame(held.map(Input::Buttons).unwrap_or(Input::None));
        let overworld = overworld(game).expect("the overworld");
        if held.is_some() && overworld.walk_counter() != 0 {
            held = None;
        }
        let location = at(game);
        if (location.0, location.1, location.2) != (last.0, last.1, last.2) {
            last = location;
            squares.push(Square { frames, lag: 0, location, count: overworld.num_sprites(), sprites: *overworld.sprites() });
            if done((location.0, location.1, location.2)) {
                return squares;
            }
        }
    }
    panic!("the recreation never got there: {last:?}");
}

/// The slots in use. `PrepareOAMData` and `DetectCollisionBetweenSprites` both write the adjusted
/// coordinates, in the other order on the cartridge, and nothing reads them before rewriting them.
fn compared(sprites: &Sprites, count: u8) -> Vec<SpriteState> {
    sprites.iter().take(count as usize + 1).map(|&s| SpriteState { y_adjusted: 0, x_adjusted: 0, ..s }).collect()
}

fn compare_sprites(cartridge: (&Sprites, u8), recreation: (&Sprites, u8), what: &str) {
    assert_eq!(recreation.1, cartridge.1, "{what}: wNumSprites");
    let (theirs, ours) = (compared(cartridge.0, cartridge.1), compared(recreation.0, recreation.1));
    if let Some(slot) = (0..ours.len()).find(|&slot| ours[slot] != theirs[slot]) {
        panic!("{what}: sprite slot {slot}\ncartridge  {:?}\nrecreation {:?}", theirs[slot], ours[slot]);
    }
}

fn compare_rides(cartridge: &[Square], recreation: &[Square]) {
    let trace = |squares: &[Square]| squares.iter().map(|s| (s.frames, s.lag, s.location)).collect::<Vec<_>>();
    assert_eq!(recreation.len(), cartridge.len(), "squares reached\ncartridge  {:?}\nrecreation {:?}",
        trace(cartridge), trace(recreation));
    for (theirs, ours) in cartridge.iter().zip(recreation) {
        let what = format!("{:?}", theirs.location);
        assert_eq!(ours.location, theirs.location, "{what}");
        compare_sprites((&theirs.sprites, theirs.count), (&ours.sprites, ours.count), &what);
        assert_late(theirs.frames, ours.frames, theirs.lag, &what);
    }
}

/// From (11, 121) one square right, into the column through the fence's gap, then nothing: the slope
/// carries the rider down past the biker and over the bottom row's ledge into Route 18.
#[test]
fn an_idle_rider_rolls_down_the_cycling_road_and_over_its_ledge_as_the_cartridge_does() {
    let mut cartridge = stopped_on_the_slope();
    let start = Start::take(&cartridge.gb);
    cartridge.tape.clear();
    let theirs = cartridge_ride(&mut cartridge, Some(Joypad::RIGHT), |(map, _, _)| map == Map::Route18);
    assert_eq!(theirs.last().unwrap().location.0, Map::Route18, "over the ledge and off the road");

    let mut game = start.game(cartridge.tape.clone());
    let ours = recreation_ride(&mut game, Some(Joypad::RIGHT), |(map, _, _)| map == Map::Route18);
    compare_rides(&theirs, &ours);
}

/// The fixture saved where it stands and continued: `SpecialEnterMap` and `EnterMap` reload the map
/// with the NPCs placed by its `UpdateSprites`, and the slope takes the rider down into the fence.
#[test]
fn a_game_continued_on_the_slope_places_its_npcs_and_rolls_as_the_cartridge_does() {
    let mut cartridge = stopped_on_the_slope();
    // Out of VBlank and back to the top of the loop, still stopped, so the jump away is the main
    // loop's own and leaves interrupts on.
    let looping = breakpoint(sym::OverworldLoop);
    cartridge.gb.hold_buttons(joypad(Joypad::B));
    cartridge.run_to(&[looping]);
    cartridge.gb.hold_buttons(JoypadButtonState::default());
    // What CONTINUE does before `SpecialEnterMap`.
    let flags = cartridge.read(sym::wCurrentMapScriptFlags.address) | 1 << BIT_CUR_MAP_LOADED_1;
    write(&mut cartridge.gb, sym::wCurrentMapScriptFlags.address, flags);
    write(&mut cartridge.gb, sym::wPlayerDirection.address, PLAYER_DIR_DOWN);
    let mut world = super::bridge::world(&cartridge.gb);
    super::learn_move::hijack(&mut cartridge.gb, sym::SpecialEnterMap);
    cartridge.tape.clear();
    cartridge.run_to(&[looping]);
    let entered = (cartridge.sprites(), cartridge.read(sym::wNumSprites.address));
    super::to_vblank(&mut cartridge.gb);
    let theirs = cartridge_ride(&mut cartridge, None, |(_, _, y)| y == 122);

    let continued = Overworld::reset_player_sprite_data(&mut world.location);
    let mut game = Game::new(world, GameRng::tape(cartridge.tape.clone()), Pacing::Faithful);
    game.push(Mode::Overworld(continued));
    let ours = overworld(&game).expect("the overworld");
    compare_sprites((&entered.0, entered.1), (ours.sprites(), ours.num_sprites()), "the map entered");
    let ours = recreation_ride(&mut game, None, |(_, _, y)| y == 122);
    compare_rides(&theirs, &ours);
}

/// The tenth biker stands at (10, 118) looking down his column with a sight of four. A rider who
/// steps into it beside him is seen, stops rolling, and is walked up to: compared to his battle's
/// first prompt.
#[test]
fn a_biker_sees_a_rider_step_into_his_column_and_walks_up_as_the_cartridge_does() {
    use poke_core::symbols::pokered_events::EVENT_BEAT_ROUTE_17_TRAINER_9;
    const ROUTE: &[(Action, &str)] = &[(Action::Walk(Joypad::LEFT), "left into the biker's sight")];
    let forget = |cartridge: &mut scripts::Cartridge| clear_events(&mut cartridge.gb, &[EVENT_BEAT_ROUTE_17_TRAINER_9]);
    let mut seen_by_him = false;
    lockstep_holding(include_bytes!("../pokemon/data/route17-slope.bin"), Joypad::B, forget,
        walk_and_answer(ROUTE, move |seen| {
            seen_by_him |= seen.kind == Kind::Bubble;
            let fighting = seen.kind == Kind::Prompt && seen.sprites.is_none();
            assert!(!fighting || seen_by_him, "the battle began unseen");
            fighting
        }));
}

fn write(gb: &mut GameBoy, at: u16, value: u8) {
    gb.core_mut().mmu_mut().write(at, value);
}

/// Recuts `route17-slope.bin` from `a4a-route-18-frozen-lead.bin`, east of Route 18's gate: through it,
/// where the bike is forced on, west and up onto Cycling Road, then up through the gap in the fence
/// to (11, 121), three squares below the tenth biker and one column out of his sight. B is held to
/// stop there, as the slope stops for no other button with no direction, and let go before the save.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "tool: recuts route17-slope.bin; needs GB_REGEN_FIXTURES=1"]
fn regen_route17_slope_fixture() {
    type Done = fn((Map, u8, u8)) -> bool;
    const LEGS: [(Joypad, Done); 10] = [
        (Joypad::RIGHT, |(_, x, _)| x == 41),
        (Joypad::UP, |(_, _, y)| y == 9),
        (Joypad::LEFT, |(map, _, _)| map == Map::Route18Gate1F),
        (Joypad::LEFT, |(map, _, _)| map == Map::Route18),
        (Joypad::LEFT, |(_, x, _)| x == 10),
        (Joypad::UP, |(map, _, _)| map == Map::Route17),
        (Joypad::UP, |(_, _, y)| y == 124),
        (Joypad::RIGHT, |(_, x, _)| x == 12),
        (Joypad::UP, |(_, _, y)| y == 121),
        (Joypad::LEFT, |(_, x, _)| x == 11),
    ];
    let mut cartridge = Cartridge::from_state(include_bytes!("../pokemon/data/a4a-route-18-frozen-lead.bin"));
    let at = |cartridge: &Cartridge| {
        let (map, x, y, _) = cartridge.location();
        (map, x, y)
    };
    for (button, done) in LEGS {
        cartridge.gb.hold_buttons(joypad(button));
        for frame in 0.. {
            assert!(frame < 3000, "{button:?} stuck at {:?}", at(&cartridge));
            cartridge.frame();
            if done(at(&cartridge)) {
                break;
            }
        }
    }
    cartridge.gb.hold_buttons(joypad(Joypad::B));
    while !cartridge.frame() {}
    cartridge.gb.hold_buttons(JoypadButtonState::default());
    assert_eq!(at(&cartridge), (Map::Route17, 11, 121));
    assert_eq!(cartridge.read(sym::wWalkBikeSurfState.address), BIKING);
    assert_ne!(cartridge.read(sym::wStatusFlags6.address) & 1 << BIT_ALWAYS_ON_BIKE, 0);
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/pokemon/data/route17-slope.bin");
    if crate::pokemon::integration_tests::fixture::regenerating_fixtures() {
        cartridge.gb.save_state_to_file(path).unwrap();
    } else {
        println!("skipping fixture write to {path} (set GB_REGEN_FIXTURES=1)");
    }
}
