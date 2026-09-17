//! The naming screen a new game opens for the player's own name. No fixture reaches it: the one
//! cut nearest the start is already past it, in `RedsHouse2F` with the player named. So this boots
//! the cartridge cold and mashes its way through the title and Oak's speech to the name menu, whose
//! first entry is `NEW NAME`.

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
use super::{breakpoint, cartridge_until_polling, joypad, recreation_until_polling, tile_row, to_vblank};

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

#[test]
fn the_naming_screen_shows_what_the_cartridge_shows_at_every_poll() {
    let mut gb = open_the_naming_screen();
    let (kind, species) = {
        let mmu = gb.core().mmu();
        let kind = match mmu.read_pointer(&pokered_symbols::wNamingScreenType) {
            0 => NamingScreenType::Player,
            1 => NamingScreenType::Rival,
            _ => NamingScreenType::Mon,
        };
        (kind, PokemonSpecies::from_repr(mmu.read_pointer(&pokered_symbols::wCurPartySpecies)))
    };
    assert_eq!(kind, NamingScreenType::Player, "a new game names the player first");

    let mut game = Game::new(World::default(), GameRng::seeded(0), Pacing::Faithful);
    game.push(Mode::NamingScreen(NamingScreen::new(kind, species)));

    // The whole screen: `ClearScreen` runs on the way in, so nothing of the map is left behind.
    let cartridge = |gb: &GameBoy| (0..18).map(|y| tile_row(gb, y)).collect::<Vec<_>>();
    let recreation = |game: &Game| (0..18).map(|y| game.ui().row(y).to_vec()).collect::<Vec<_>>();
    // `DisplayNamingScreen` runs `LoadMonPartySpriteGfx` whatever it is naming, so the mon icons are
    // in `vSprites` even here, where nothing draws one. Only the tiles the table fills can be
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

    let presses = [Joypad::RIGHT, Joypad::DOWN, Joypad::A, Joypad::SELECT, Joypad::A, Joypad::B];
    for (step, press) in presses.into_iter().enumerate() {
        cartridge_until_polling(&mut gb);
        recreation_until_polling(&mut game, Decision::NamingScreen);
        assert_eq!(cartridge(&gb), recreation(&game), "polling before press {step}");
        let ids = loaded(&game);
        assert_eq!(ids.len(), ICONS_LOADED, "every pattern the table names, before press {step}");
        assert_eq!(cartridge_icons(&gb, &ids), recreation_icons(&game, &ids),
                   "the icon patterns before press {step}");
        gb.hold_buttons(joypad(press));
        game.frame(Input::Buttons(press));
        to_vblank(&mut gb);
        gb.hold_buttons(JoypadButtonState::default());
        game.frame(Input::None);
    }
}
