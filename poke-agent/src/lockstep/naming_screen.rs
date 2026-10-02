//! The naming screen a new game opens for the player's own name. No fixture reaches it: the one
//! cut nearest the start is already past it, in `RedsHouse2F` with the player named. So this boots
//! the cartridge cold and mashes its way through the title and Oak's speech to the name menu, whose
//! first entry is `NEW NAME`. Then a nickname's, the Charmander ball's in Oak's lab, whose icon
//! bobs in OAM.

use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::joypad::JoypadButtonState;
use poke_core::rom_gfx::TILE_BYTES;
use poke_core::species::PokemonSpecies;
use pokered::command::Decision;
use pokered::input::Joypad;
use pokered::mode::Mode;
use pokered::modes::naming_screen::{NamingScreen, NamingScreenType};
use pokered::rng::GameRng;
use pokered::world::World;
use pokered::{Game, Input, Pacing};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use super::{assert_late, breakpoint, cartridge_until_polling, joypad, recreation_until_polling, tile_row, to_vblank, DELAY3};
use super::battle::letters;
use super::harness::assert_same_oam;
use super::scripts::{Action, Cartridge, Kind, PROMPT};

/// Past the last pattern `MonPartySpritePointers` names, frame two included.
const ICON_TILES: u8 = 0x7C;
/// How many of those the table actually fills. Its destinations leave gaps, because an icon drawn
/// from one pattern a row loads only even tiles and nothing reads the odd one between.
const ICONS_LOADED: usize = 72;

/// Mashes A until the screen opens. A held button is no use here: every step of the conversation
/// wants its own rising edge, so this alternates press and release.
fn open_the_naming_screen() -> GameBoy {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    let screen = breakpoint(pokered_symbols::DisplayNamingScreen);
    // The title wants START and everything after it wants A, so both are offered in turn; every
    // step needs its own rising edge, hence the release between.
    for round in 0..1500 {
        for held in [true, false] {
            let a = held && round % 2 == 0;
            let start = held && round % 2 == 1;
            gb.hold_buttons(JoypadButtonState { a, start, ..Default::default() });
            let (stop, _) = gb.run_until(&[screen], MachineCycles::PER_FRAME * 3);
            if stop == Stop::Breakpoint(screen) {
                gb.hold_buttons(JoypadButtonState::default());
                return gb;
            }
        }
    }
    panic!("a new game never reached the naming screen");
}

/// The kind of naming screen the cartridge has open, and the species a nickname is for.
fn what_is_named(gb: &GameBoy) -> (NamingScreenType, Option<PokemonSpecies>) {
    let mmu = gb.core().mmu();
    let kind = match mmu.read_pointer(&pokered_symbols::wNamingScreenType) {
        0 => NamingScreenType::Player,
        1 => NamingScreenType::Rival,
        _ => NamingScreenType::Mon,
    };
    let species = PokemonSpecies::from_repr(mmu.read_pointer(&pokered_symbols::wCurPartySpecies));
    (kind, species.filter(|_| kind == NamingScreenType::Mon))
}

/// Both screens, the icon patterns and OAM compared at every poll from the cartridge entering
/// `DisplayNamingScreen`, through `presses` with `idle` polls before each and after the last.
fn compare_at_every_poll(mut gb: GameBoy, presses: &[Joypad], idle: usize) {
    let (kind, species) = what_is_named(&gb);
    let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::NamingScreen(NamingScreen::new(kind, species)));

    // The whole screen: `ClearScreen` runs on the way in, so nothing of the map is left behind.
    let cartridge = |gb: &GameBoy| (0..18).map(|y| tile_row(gb, y)).collect::<Vec<_>>();
    let recreation = |game: &Game| (0..18).map(|y| game.ui().row(y).to_vec()).collect::<Vec<_>>();
    // `DisplayNamingScreen` runs `LoadMonPartySpriteGfx` whatever it is naming, so the mon icons are
    // in `vSprites` even for a player, where nothing draws one. Only the tiles the table fills can be
    // compared: the cartridge's gaps still hold whatever VRAM held before it ran.
    let loaded = |game: &Game| (0..ICON_TILES)
        .filter(|&id| *game.screen().tiles.obj(id) != [0; TILE_BYTES])
        .collect::<Vec<_>>();
    let cartridge_icons = |gb: &GameBoy, ids: &[u8]| ids.iter()
        .flat_map(|&id| gb.core().mmu().ppu().vram()[id as usize * TILE_BYTES..][..TILE_BYTES].to_vec())
        .collect::<Vec<_>>();
    let recreation_icons = |game: &Game, ids: &[u8]| ids.iter()
        .flat_map(|&id| *game.screen().tiles.obj(id))
        .collect::<Vec<_>>();
    let compare = |gb: &GameBoy, game: &Game, what: &str| {
        assert_eq!(cartridge(gb), recreation(game), "{what}");
        let ids = loaded(game);
        assert_eq!(ids.len(), ICONS_LOADED, "every pattern the table names, {what}");
        assert_eq!(cartridge_icons(gb, &ids), recreation_icons(game, &ids), "the icon patterns {what}");
        assert_same_oam(gb, game, what);
    };

    cartridge_until_polling(&mut gb);
    recreation_until_polling(&mut game, Decision::NamingScreen);
    for (step, press) in presses.iter().map(Some).chain([None]).enumerate() {
        for poll in 0..idle {
            compare(&gb, &game, &format!("idle poll {poll} before press {step}"));
            cartridge_until_polling(&mut gb);
            recreation_until_polling(&mut game, Decision::NamingScreen);
        }
        compare(&gb, &game, &format!("polling before press {step}"));
        let Some(&press) = press else { break };
        // A frame each with the press held, then each until it polls again: the icon moves once a
        // frame, so neither may run one the other does not.
        gb.hold_buttons(joypad(press));
        game.frame(Input::Buttons(press));
        to_vblank(&mut gb);
        gb.hold_buttons(JoypadButtonState::default());
        let cartridge = cartridge_until_polling(&mut gb);
        let recreation = recreation_until_polling(&mut game, Decision::NamingScreen);
        // SELECT's `PrintAlphabet` waits out a `Delay3` before the loop goes on.
        let loading = if press == Joypad::SELECT { DELAY3 } else { 0 };
        assert_late(cartridge, recreation, loading, &format!("press {step}"));
    }
}

#[test]
fn the_naming_screen_shows_what_the_cartridge_shows_at_every_poll() {
    let gb = open_the_naming_screen();
    assert_eq!(what_is_named(&gb).0, NamingScreenType::Player, "a new game names the player first");
    compare_at_every_poll(gb, &[Joypad::RIGHT, Joypad::DOWN, Joypad::A, Joypad::SELECT, Joypad::A, Joypad::B], 0);
}

/// The Charmander ball taken in Oak's lab and YES to a nickname: stopped as `DisplayNamingScreen`
/// begins.
fn open_a_nickname_screen() -> GameBoy {
    let mut cartridge = Cartridge::from_state(include_bytes!("../pokemon/data/branch-oaks-lab.bin"));
    while cartridge.to_poll().0 != Kind::Overworld {}
    cartridge.act(Action::Press(Joypad::RIGHT, 2));
    cartridge.act(Action::Talk);
    for _ in 0..40 {
        let asked = letters(&tile_row(&cartridge.gb, 14)).contains("nickname");
        if asked && cartridge.gb.core().mmu().read_pointer(&pokered_symbols::wMaxMenuItem) == 1 {
            let mut gb = cartridge.gb;
            gb.hold_buttons(joypad(Joypad::A));
            let screen = breakpoint(pokered_symbols::DisplayNamingScreen);
            let (stop, _) = gb.run_until(&[screen], MachineCycles::PER_FRAME * 120);
            assert_eq!(stop, Stop::Breakpoint(screen), "YES never opened the naming screen");
            gb.hold_buttons(JoypadButtonState::default());
            return gb;
        }
        cartridge.act(PROMPT);
    }
    panic!("the nickname was never asked for");
}

/// A mon's nickname: its icon in OAM, bobbing at `AnimatePartyMon_ForceSpeed1`'s rate through idle
/// polls and presses alike.
#[test]
fn the_nickname_screen_s_icon_bobs_as_the_cartridge_s_does() {
    let gb = open_a_nickname_screen();
    assert_eq!(what_is_named(&gb), (NamingScreenType::Mon, Some(PokemonSpecies::Charmander)));
    compare_at_every_poll(gb, &[Joypad::RIGHT, Joypad::DOWN, Joypad::A, Joypad::SELECT, Joypad::A, Joypad::B], 20);
}
