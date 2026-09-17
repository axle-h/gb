//! Events against the cartridge, through `scripts.rs`'s lockstep: a Pokémon Center heal, a hidden item,
//! a bookshelf, poison to a blackout, a vending machine, an in-game trade and the day care, each from
//! the fixture nearest it with WRAM written to clear the event it tests.

use gb::game_boy::GameBoy;
use gb::ram::ROM;
use poke_core::map::Map;
use poke_core::move_name::PokemonMoveName;
use poke_core::species::PokemonSpecies;
use poke_core::sprite::SpriteFacing;
use pokered::input::Joypad;
use pokered::party::{BoxMon, Named};
use pokered::systems::stats::Dvs;
use crate::pokemon::symbols::{pokered_symbols as sym, DmgPointerRead};
use super::scripts::{lockstep, Action, Cartridge, Kind, Seen, PROMPT};

const NAME_LENGTH: u16 = 11;
const TERMINATOR: u8 = 0x50;
const PARTY_STRUCT: u16 = 0x2C;

/// A `box_struct` and its two names as WRAM lays them out.
pub(super) fn box_mon_at(gb: &GameBoy, at: u16, ot: u16, nick: u16) -> Named<BoxMon> {
    let mmu = gb.core().mmu();
    let b: Vec<u8> = (0..33).map(|i| mmu.read(at + i)).collect();
    let word = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
    let name = |at: u16| (0..NAME_LENGTH).map(|i| mmu.read(at + i)).take_while(|&byte| byte != TERMINATOR).collect();
    Named {
        mon: BoxMon {
            species: PokemonSpecies::from_repr(b[0]).expect("a species"),
            hp: word(1),
            box_level: b[3],
            status: b[4],
            types: [b[5], b[6]],
            catch_rate: b[7],
            moves: [0, 1, 2, 3].map(|i| PokemonMoveName::from_repr(b[8 + i])),
            ot_id: word(12),
            exp: u32::from_be_bytes([0, b[14], b[15], b[16]]),
            stat_exp: [0, 1, 2, 3, 4].map(|i| word(17 + 2 * i)),
            dvs: Dvs([b[27], b[28]]),
            pp: [b[29], b[30], b[31], b[32]],
        },
        ot: name(ot),
        nick: name(nick),
    }
}

/// Walks `route`, answering every prompt with A, and stops at the first overworld poll `done` accepts
/// once the route is walked.
fn route(route: &'static [(Action, &'static str)], done: impl Fn(&Seen) -> bool + 'static)
    -> impl FnMut(usize, &Seen) -> Option<(Action, &'static str)>
{
    let mut walked = 0;
    move |_, seen| match seen.kind {
        Kind::Prompt => Some((PROMPT, "a prompt")),
        Kind::MapChange | Kind::Bubble => Some((Action::Press(Joypad::empty(), 0), "a new map")),
        Kind::Overworld if walked >= route.len() && done(seen) => None,
        Kind::Overworld => {
            let step = route.get(walked).copied().unwrap_or((super::scripts::WAIT, "wait"));
            walked += 1;
            Some(step)
        }
    }
}

/// `choose`, asserting when it ends that a prompt showed `text` on the box's first line.
fn seeing_text(mut choose: impl FnMut(usize, &Seen) -> Option<(Action, &'static str)>, text: &'static str)
    -> impl FnMut(usize, &Seen) -> Option<(Action, &'static str)>
{
    let mut seen_text = false;
    move |i, seen| {
        seen_text |= seen.kind == Kind::Prompt && seen.screen.get(14).is_some_and(|row| super::battle::letters(row).contains(text));
        let step = choose(i, seen);
        assert!(step.is_some() || seen_text, "no prompt showed {text:?}");
        step
    }
}

const fn walk(button: Joypad, what: &'static str) -> (Action, &'static str) {
    (Action::Walk(button), what)
}

#[test]
#[ignore = "a probe: where every committed fixture stands"]
fn probe_fixture_positions() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pokemon/data");
    let mut paths: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "bin")).collect();
    paths.sort();
    for path in paths {
        let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
        if gb.load_state(&std::fs::read(&path).unwrap()).is_err() {
            continue;
        }
        let mmu = gb.core().mmu();
        println!("{:40} {:?} ({}, {})", path.file_name().unwrap().to_string_lossy(), Map::from_repr(mmu.read_pointer(&sym::wCurMap)),
            mmu.read_pointer(&sym::wXCoord), mmu.read_pointer(&sym::wYCoord));
    }
}

/// The Cerulean nurse, asked for the first time in the game with the lead mon hurt: HEAL, the balls
/// onto the machine, the jingle, the bow.
#[test]
fn the_nurse_heals_the_party_as_the_cartridge_does() {
    const ROUTE: &[(Action, &str)] = &[
        walk(Joypad::UP, "up"), walk(Joypad::UP, "up"), walk(Joypad::UP, "up"), walk(Joypad::UP, "up to the counter"),
        (Action::Talk, "talk to the nurse"),
    ];
    let prepare = |cartridge: &mut Cartridge| {
        cartridge.write(sym::wPartyMon1HP.address + 1, 1);
        cartridge.write(sym::wPartyMon1HP.address, 0);
        let flags = cartridge.read(sym::wStatusFlags4.address);
        cartridge.write(sym::wStatusFlags4.address, flags & !(1 << 2));
    };
    lockstep(include_bytes!("../pokemon/data/post-cascade.bin"), prepare, route(ROUTE, |seen| seen.settled().party_hp[0] > 1));
}

/// Viridian Forest's hidden Antidote in the bush at (16, 42), forgotten first: round the Youngster
/// at (16, 43) to the square left of it, face it, and A.
#[test]
fn a_hidden_item_is_found_as_the_cartridge_does() {
    const ROUTE: &[(Action, &str)] = &[
        walk(Joypad::DOWN, "down"), walk(Joypad::LEFT, "left"), walk(Joypad::LEFT, "left"),
        walk(Joypad::UP, "up"), walk(Joypad::UP, "up to (15, 42)"),
        (Action::Press(Joypad::RIGHT, 2), "face the Antidote"),
        (Action::Talk, "look"),
    ];
    let prepare = |cartridge: &mut Cartridge| {
        let at = sym::wObtainedHiddenItemsFlags.address;
        let flags = cartridge.read(at);
        cartridge.write(at, flags & !(1 << 1));
    };
    lockstep(include_bytes!("../pokemon/data/viridian-forest.bin"), prepare, route(ROUTE, |seen| seen.settled().hidden[0] & 1 << 1 != 0));
}

/// A bookshelf in Red's house: down off the stairs, along to the shelf at (1, 1), face it, and A.
#[test]
fn a_bookshelf_is_read_as_the_cartridge_does() {
    const ROUTE: &[(Action, &str)] = &[
        walk(Joypad::DOWN, "down off the stairs"),
        walk(Joypad::LEFT, "left"), walk(Joypad::LEFT, "left"), walk(Joypad::LEFT, "left"),
        walk(Joypad::LEFT, "left"), walk(Joypad::LEFT, "left"), walk(Joypad::LEFT, "left to (1, 2)"),
        (Action::Press(Joypad::UP, 2), "face the shelf"),
        (Action::Talk, "read it"),
    ];
    lockstep(include_bytes!("../pokemon/data/reds-house-1f-state.bin"), |_| {}, seeing_text(route(ROUTE, |_| true), "Crammed full of"));
}

/// Cuts `celadon-mart-roof.bin` from `at-celadon.bin`: the Pokémon Center door's entry in
/// `wWarpEntries` pointed at the roof's warp, a step up, and the state saved at the roof's first
/// overworld poll, a square below the stairs: a warp from an outside map steps out of a door tile,
/// and the roof's stairs are one. Only under `GB_REGEN_FIXTURES=1`.
#[test]
#[cfg(feature = "slow-tests")]
#[ignore = "tool: recuts celadon-mart-roof.bin; needs GB_REGEN_FIXTURES=1"]
fn regen_celadon_mart_roof_fixture() {
    const WARP_ENTRY: u16 = 4;
    let mut cartridge = Cartridge::from_state(include_bytes!("../pokemon/data/at-celadon.bin"));
    while cartridge.to_poll().0 != Kind::Overworld {}
    let warps = cartridge.read(sym::wNumberOfWarps.address) as u16;
    let door = (0..warps).map(|i| sym::wWarpEntries.address + i * WARP_ENTRY)
        .find(|&at| (cartridge.read(at), cartridge.read(at + 1)) == (9, 41))
        .expect("the Pokémon Center door");
    cartridge.write(door + 2, 0);
    cartridge.write(door + 3, Map::CeladonMartRoof as u8);
    let mut kind = cartridge.act(Action::Walk(Joypad::UP)).0;
    while kind != Kind::Overworld {
        kind = cartridge.to_poll().0;
    }
    let mmu = cartridge.gb.core().mmu();
    let at = (Map::from_repr(mmu.read_pointer(&sym::wCurMap)), mmu.read_pointer(&sym::wXCoord), mmu.read_pointer(&sym::wYCoord));
    assert_eq!(at, (Some(Map::CeladonMartRoof), 15, 3));
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/pokemon/data/celadon-mart-roof.bin");
    if crate::pokemon::integration_tests::fixture::regenerating_fixtures() {
        cartridge.gb.save_state_to_file(path).unwrap();
    } else {
        println!("skipping fixture write to {path} (set GB_REGEN_FIXTURES=1)");
    }
}

/// The first vending machine on the Celadon department store's roof: along to it, up, a Fresh Water
/// chosen, the sixty rattles, and ¥200 paid.
#[test]
fn a_vending_machine_sells_a_drink_as_the_cartridge_does() {
    const ROUTE: &[(Action, &str)] = &[
        walk(Joypad::LEFT, "left"), walk(Joypad::LEFT, "left"), walk(Joypad::LEFT, "left"),
        walk(Joypad::LEFT, "left"), walk(Joypad::LEFT, "left to (10, 3)"),
        walk(Joypad::UP, "up to the machine"),
        (Action::Talk, "use it"),
    ];
    let fresh_water = poke_core::item::ItemId::FreshWater as u8;
    lockstep(include_bytes!("../pokemon/data/celadon-mart-roof.bin"), |_| {},
        route(ROUTE, move |seen| seen.bag.iter().any(|&(item, _)| item == fresh_water)));
}

/// The day care, both ways: in off Route 5, up to the man, Hitmonlee left with him, then asked for
/// back at once, its experience written three levels past its level first, so the man says it grew by
/// three, asks ¥400 and hands it back at level 33.
#[test]
fn the_day_care_takes_a_mon_and_gives_it_back_grown_as_the_cartridge_does() {
    const SLOT: u16 = 5;
    const EXP: u32 = 33 * 33 * 33;
    const ROUTE: &[(Action, &str)] = &[
        walk(Joypad::UP, "up into the day care"),
        walk(Joypad::UP, "up"), walk(Joypad::UP, "up"), walk(Joypad::UP, "up to (2, 4)"),
        (Action::Talk, "leave a mon"),
        (Action::Talk, "ask for it back"),
    ];
    let prepare = |cartridge: &mut Cartridge| {
        let mon = sym::wPartyMon1.address + SLOT * PARTY_STRUCT;
        assert_eq!(cartridge.read(mon), PokemonSpecies::Hitmonlee as u8);
        for (i, byte) in EXP.to_be_bytes()[1..].iter().enumerate() {
            cartridge.write(mon + 14 + i as u16, *byte);
        }
        cartridge.write(sym::wPartyAndBillsPCSavedMenuItem.address, SLOT as u8);
    };
    lockstep(include_bytes!("../pokemon/data/postgame-daycare.bin"), prepare,
        route(ROUTE, |seen| seen.settled().day_care.is_none()
            && seen.settled().party.last() == Some(&(PokemonSpecies::Hitmonlee, 33))));
}

/// Poison walked to a blackout: the lead poisoned on 2 HP and the other fainted, back and forth in
/// Mr. Fuji's house until the lead faints, the player blacks out and wakes in the last town's
/// Pokémon Center's town.
#[test]
fn poison_walks_the_party_to_a_blackout_as_the_cartridge_does() {
    let prepare = |cartridge: &mut Cartridge| {
        let first = sym::wPartyMon1HP.address;
        cartridge.write(first, 0);
        cartridge.write(first + 1, 2);
        cartridge.write(first + 3, 1 << 3);
        let second = first + PARTY_STRUCT;
        cartridge.write(second, 0);
        cartridge.write(second + 1, 0);
        cartridge.write(second + 3, 0);
    };
    let mut left = true;
    lockstep(include_bytes!("../pokemon/data/post-poke-flute.bin"), prepare, move |_, seen| match seen.kind {
        Kind::Prompt => Some((PROMPT, "a prompt")),
        Kind::MapChange => Some((Action::Press(Joypad::empty(), 0), "a new map")),
        Kind::Bubble => None,
        Kind::Overworld if seen.location.0 != Map::MrFujisHouse => None,
        Kind::Overworld => {
            left = !left;
            Some(if left { walk(Joypad::LEFT, "left") } else { walk(Joypad::RIGHT, "right") })
        }
    });
}

/// The Underground Path girl's trade, forgotten first, with a Nidoran♂ written into the lead slot and
/// the party menu opening on it: in, up to her, yes, the lead, and the Nidoran♀ back.
#[test]
fn an_in_game_trade_as_the_cartridge_does() {
    const ROUTE: &[(Action, &str)] = &[
        walk(Joypad::UP, "up into the path's entrance"),
        walk(Joypad::UP, "up"), walk(Joypad::UP, "up"), walk(Joypad::UP, "up to (3, 4)"), walk(Joypad::LEFT, "left below the girl"),
        (Action::Press(Joypad::UP, 2), "face her"),
        (Action::Talk, "talk to her"),
    ];
    let prepare = |cartridge: &mut Cartridge| {
        let at = sym::wCompletedInGameTradeFlags.address + 1;
        let flags = cartridge.read(at);
        cartridge.write(at, flags & !(1 << 1));
        let nidoran = PokemonSpecies::NidoranMale as u8;
        cartridge.write(sym::wPartySpecies.address, nidoran);
        cartridge.write(sym::wPartyMon1.address, nidoran);
        cartridge.write(sym::wPartyAndBillsPCSavedMenuItem.address, 0);
    };
    lockstep(include_bytes!("../pokemon/data/postgame-trades.bin"), prepare,
        route(ROUTE, |seen| seen.settled().trades & 1 << 9 != 0
            && seen.settled().party.last().is_some_and(|&(species, _)| species == PokemonSpecies::NidoranFemale)));
}


/// Prints which way each square of a map can be walked from, as the recreation judges it: `LOG_MAP`
/// names the map.
#[test]
#[ignore = "a probe: a map's walkable squares"]
fn probe_map_walkability() {
    use pokered::mode::Mode;
    use pokered::modes::overworld::Overworld;
    use pokered::systems::overworld::{Direction, Location};
    let name = std::env::var("LOG_MAP").unwrap_or("UndergroundPathRoute5".into());
    let map = Map::all().find(|map| format!("{map:?}") == name).unwrap();
    let header = poke_core::map_header::MapHeader::read(map).unwrap();
    for y in 0..header.height * 2 {
        let mut row = String::new();
        for x in 0..header.width * 2 {
            let mut world = pokered::world::World::default();
            world.location = Location { map, x, y, facing: SpriteFacing::Down, ..Location::default() };
            let mut game = pokered::Game::new(world, pokered::rng::GameRng::seeded(0), pokered::Pacing::Faithful);
            game.push(Mode::Overworld(Overworld::new()));
            let Some(Mode::Overworld(overworld)) = game.modes().last() else { unreachable!() };
            let open: String = [Direction::Up, Direction::Down, Direction::Left, Direction::Right].iter()
                .zip("UDLR".chars()).map(|(&d, letter)| if overworld.step_refusal(d, game.world()).is_none() { letter } else { '.' })
                .collect();
            row.push_str(&format!("{open} "));
        }
        println!("{y:3} {row}");
    }
}


