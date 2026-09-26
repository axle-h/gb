//! What a policy is shown, from both halves of the game, compared along the same walk.
//!
//! The emulated side reads WRAM and the drawn tilemap; the native side reads the recreation's own
//! fields. A policy is handed one `GameState` either way, so what has to agree is that state, and
//! above all the action rows the pathfinder offers: those are the menu the policy chooses from.

use poke_core::map::Map;
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::overworld::Overworld;
use pokered::rng::GameRng;
use pokered::{Game, Input, Pacing};

use crate::pokemon::native::NativeGame;
use gb::ram::{RAM, ROM};
use crate::pokemon::symbols::pokered_symbols as sym;
use crate::pokemon::{GameState, PokemonApi, PokemonApiTrait};

use super::overworld::Cartridge;

/// The part of a [`GameState`] this comparison covers. Everything in it is filled by both halves;
/// what is not in it is presentation, or the battle's, which `lockstep/battle.rs` compares.
#[derive(Debug, PartialEq)]
struct Compared {
    map: Map,
    position: poke_core::geometry::Point8,
    facing: poke_core::sprite::PlayerFacingDirection,
    badges: String,
    money: u32,
    coins: u16,
    player_name: String,
    rival_name: String,
    party: Vec<String>,
    bag: Vec<(crate::pokemon::item::ItemId, u8)>,
    pokedex_owned: Vec<poke_core::species::PokemonSpecies>,
    pokedex_seen: Vec<poke_core::species::PokemonSpecies>,
    can_use_cut: bool,
    can_use_surf: bool,
    on_bicycle: bool,
    strength_active: bool,
    repel_steps: u8,
    hall_of_fame_teams: u8,
    boxed: String,
    safari: String,
    /// Every row the pathfinder offers, with the length of the walk to it: the whole map pipeline
    /// in one field, since a row is a block map, a tileset, a warp table and the people on it.
    actions: Vec<(String, usize)>,
}

fn compared(state: &GameState) -> Compared {
    Compared {
        map: state.map.map,
        position: state.map.player_position,
        facing: state.map.player_direction,
        badges: format!("{:?}", state.badges),
        money: state.money,
        coins: state.coins,
        player_name: state.name.to_default_string(),
        rival_name: state.rival_name.to_default_string(),
        party: state.pokemon.iter().map(|mon| format!(
            "{} {:?} lv{} {}/{} {:?} exp{} dv{:?} ev{:?} {:?} ot={} id={}",
            mon.nickname.to_default_string(), mon.species, mon.level, mon.current_hp, mon.stats.hp, mon.status,
            mon.experience, mon.individual_values, mon.effort_values,
            mon.moves.iter().map(|m| m.map(|m| (m.name, m.pp))).collect::<Vec<_>>(),
            mon.trainer_name.to_default_string(), mon.trainer_id,
        )).collect(),
        bag: state.bag.iter().map(|slot| (slot.id, slot.quantity)).collect(),
        pokedex_owned: state.pokedex_owned.species(),
        pokedex_seen: state.pokedex_seen.species(),
        can_use_cut: state.can_use_cut,
        can_use_surf: state.can_use_surf,
        on_bicycle: state.on_bicycle,
        strength_active: state.strength_active,
        repel_steps: state.repel_steps,
        hall_of_fame_teams: state.hall_of_fame_teams,
        boxed: format!("{:?}", state.boxed_pokemon),
        safari: format!("{:?}", state.safari),
        actions: {
            let mut rows: Vec<(String, usize)> =
                state.map.actions().iter().map(|a| (a.id(), a.route.len())).collect();
            rows.sort();
            rows
        },
    }
}

/// Out of Red's house and up through Pallet Town: a warp, a connection and the people who wander
/// while it happens, which is every part of the map pipeline the rows rest on.
const ROUTE: &[(Joypad, &str)] = &[
    (Joypad::LEFT, "left, turning"),
    (Joypad::LEFT, "left"),
    (Joypad::LEFT, "left"),
    (Joypad::LEFT, "left to (8, 12)"),
    (Joypad::UP, "up, turning"),
    (Joypad::UP, "up"),
    (Joypad::UP, "up"),
    (Joypad::UP, "up"),
    (Joypad::UP, "up"),
    (Joypad::UP, "up to (8, 6)"),
    (Joypad::LEFT, "left, turning"),
    (Joypad::LEFT, "left"),
    (Joypad::LEFT, "left to (5, 6)"),
    (Joypad::UP, "through Red's door"),
    (Joypad::DOWN, "out off the mat and down from the door"),
    (Joypad::RIGHT, "right"),
    (Joypad::RIGHT, "right"),
    (Joypad::RIGHT, "right"),
    (Joypad::RIGHT, "right"),
    (Joypad::RIGHT, "right"),
    (Joypad::UP, "up"),
    (Joypad::UP, "up"),
    (Joypad::UP, "up"),
    (Joypad::UP, "up"),
    (Joypad::UP, "into the grass"),
    (Joypad::UP, "the edge of Pallet Town"),
    (Joypad::UP, "into Route 1"),
];

/// The cartridge→`World` bridges in the menu tests fill only what those menus read. This tops up
/// the fields this comparison covers, so a difference is the adapter's rather than the harness's.
fn top_up(world: &mut pokered::world::World, cartridge: &Cartridge) {
    let mmu = cartridge.gb.core().mmu();
    let name_at = |at: u16| (0..11u16).map(|i| mmu.read(at + i))
        .take_while(|&byte| byte != crate::pokemon::strings::PokemonString::TERMINATOR).collect();
    world.rival_name = name_at(sym::wRivalName.address);
    world.badges = cartridge.read(sym::wObtainedBadges.address);
    world.player_id = u16::from_be_bytes([cartridge.read(sym::wPlayerID.address),
                                          cartridge.read(sym::wPlayerID.address + 1)]);
    world.hall_of_fame_teams = cartridge.read(sym::wNumHoFTeams.address);
    world.current_box = cartridge.read(sym::wCurrentBoxNum.address) & 0x7F;
    world.pokedex.owned = mmu.read_slice(sym::wPokedexOwned.address, 19).try_into().unwrap();
    world.pokedex.seen = mmu.read_slice(sym::wPokedexSeen.address, 19).try_into().unwrap();
    // `item_menu`'s party carries no OT, which `status_screen`'s does.
    world.party = super::status_screen::the_party(&cartridge.gb);
}

const BUDGET: u32 = 600;

/// Holds the direction until a step begins, then lets go until the loop is asking again.
fn cartridge_step(cartridge: &mut Cartridge, button: Joypad, what: &str) {
    let from = Map::from_repr(cartridge.read(sym::wCurMap.address)).unwrap();
    cartridge.gb.hold_buttons(super::joypad(button));
    for held in 0.. {
        assert!(held < BUDGET, "{what}: the cartridge never moved");
        cartridge.frame();
        let moved = cartridge.read(sym::wWalkCounter.address) != 0
            || Map::from_repr(cartridge.read(sym::wCurMap.address)).unwrap() != from
            || cartridge.read(0xFF47) != 0xE4;
        if moved {
            break;
        }
    }
    cartridge.gb.hold_buttons(Default::default());
    for waited in 0.. {
        assert!(waited < BUDGET, "{what}: the cartridge never settled");
        if cartridge.frame() {
            break;
        }
    }
}

fn recreation_step(game: &mut Game, button: Joypad, what: &str) {
    use pokered::command::Decision;
    use pokered::mode::Status;
    let from = game.world().location.map;
    for held in 0.. {
        assert!(held < BUDGET, "{what}: the recreation never moved");
        game.frame(Input::Buttons(button));
        let overworld = match game.modes().last() {
            Some(Mode::Overworld(overworld)) => overworld,
            other => panic!("{what}: the overworld is not on top: {other:?}"),
        };
        if overworld.walk_counter() != 0 || game.world().location.map != from
            || game.screen().effects.bgp != 0xE4
        {
            break;
        }
    }
    for waited in 0.. {
        assert!(waited < BUDGET, "{what}: the recreation never settled");
        game.frame(Input::None);
        if game.status() == Status::Waiting(Decision::Overworld) {
            break;
        }
    }
}

#[test]
fn the_state_a_policy_is_shown_matches_the_cartridge_along_a_walk() {
    let mut cartridge = Cartridge::from_state(include_bytes!("../pokemon/data/pallet-town-state.bin"));
    // Grass on the way would start wild battles, which a walk of single steps cannot fight.
    let flags4 = sym::wStatusFlags4.address;
    let value = cartridge.read(flags4) | 1 << 4;
    cartridge.gb.core_mut().mmu_mut().write(flags4, value);
    while !cartridge.frame() {}

    let mut world = cartridge.world();
    top_up(&mut world, &cartridge);
    let (sprites, count, standing) =
        (cartridge.sprites(), cartridge.read(sym::wNumSprites.address), cartridge.standing());
    assert_eq!(world.location.map, Map::PalletTown);

    // The cartridge walks first, so its `Random` tape is what the recreation's NPCs wander on.
    let mut theirs = vec![compared(&PokemonApi::new(&mut cartridge.gb).game_state().unwrap())];
    for &(button, what) in ROUTE {
        cartridge_step(&mut cartridge, button, what);
        theirs.push(compared(&PokemonApi::new(&mut cartridge.gb).game_state().unwrap()));
    }

    let mut game = Game::new(world, GameRng::tape(cartridge.tape.clone()), Pacing::Faithful);
    game.push(Mode::Overworld(
        Overworld::standing(sprites, count, standing).with_battle_flags(true, false, 0),
    ));
    let mut native = NativeGame::new(game).unwrap();

    let mut ours = vec![compared(&native.game_state().unwrap())];
    for &(button, what) in ROUTE {
        recreation_step(native.game_mut(), button, what);
        ours.push(compared(&native.game_state().unwrap()));
    }

    let points = std::iter::once("standing where the save left off").chain(ROUTE.iter().map(|&(_, what)| what));
    for (what, (ours, theirs)) in points.zip(ours.iter().zip(&theirs)) {
        assert_eq!(ours, theirs, "{what}");
    }
}
