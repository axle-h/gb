//! `LoadSpinnerArrowTiles`, which is the whole of the spin an arrow tile puts the player in: it runs
//! once per pass of the walking animation and once per map-script pass, for as long as the arrow's
//! simulated presses last.

use poke_core::map_gfx::tileset_sheet;
use poke_core::map_header::TileSetId;
use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::symbols::{pokered_symbols, DmgPointer};
use crate::audio::data::sounds;
use crate::gfx::tiles::V_CHARS2;
use crate::input::Joypad;
use crate::mode::Ctx;
use crate::systems::overworld::spinners::spinner_facing;
use super::script::{Routine, Script, Then};
use super::Overworld;

/// `FacilitySpinnerArrows` and `GymSpinnerArrows`: the tile of `SpinnerArrowAnimTiles` that stands
/// in for each of the four arrow tiles, and which tile of the tileset that is.
const FACILITY_ARROWS: [(u8, u8); 4] = [(0, 0x20), (1, 0x21), (2, 0x30), (3, 0x31)];
const GYM_ARROWS: [(u8, u8); 4] = [(1, 0x3C), (3, 0x3D), (0, 0x4C), (2, 0x4D)];

impl Overworld {
    /// The player a quarter turn on, and the arrows under them swapped between the turned copy and
    /// the tileset's own tiles on alternate presses, which is what makes them spin as well.
    pub(crate) fn load_spinner_arrow_tiles(&mut self, ctx: &mut Ctx) {
        self.sprites[0].image_index = spinner_facing(self.sprites[0].image_index);
        let arrows = if self.view.tileset == TileSetId::Facility { FACILITY_ARROWS } else { GYM_ARROWS };
        let turned = self.simulated_index % 2 == 1;
        for (anim, tile) in arrows {
            let at = tile as usize * TILE_BYTES;
            let bytes = if turned {
                &rom_slice(pokered_symbols::SpinnerArrowAnimTiles + anim as u16 * TILE_BYTES as u16)[..TILE_BYTES]
            } else {
                &tileset_sheet(self.view.tileset)[at..at + TILE_BYTES]
            };
            ctx.screen.tiles.load(V_CHARS2 + tile as usize, bytes);
        }
    }
}

/// A spinner map's `DefaultScript`: the arrow tile under the player, if there is one, started and
/// the map's own `PlayerSpinningScript` left to run; otherwise the map's trainers, as usual.
pub(crate) fn arrow_tile_default(rt: &mut Script, table: DmgPointer, spinning: u8) -> Option<Then> {
    let Some(list) = rt.arrow_movement(table) else {
        return Some(Then::call(Routine::CheckFightingMapTrainers));
    };
    rt.set_spinning(true);
    rt.simulate_joypad_rle(list);
    rt.play_sound(sounds::SFX_ARROW_TILES);
    // `PAD_BUTTONS | PAD_CTRL_PAD`: nothing the player presses stops an arrow tile.
    rt.joy_ignore(Joypad::all());
    rt.set_cur_map_script(spinning);
    None
}

/// A spinner map's `PlayerSpinningScript`: the arrows animated for as long as the presses last, and
/// the pad given back when they run out.
pub(crate) fn player_spinning(rt: &mut Script, default: u8) {
    if rt.simulated_joypad_states_index() != 0 {
        rt.load_spinner_arrow_tiles();
        return;
    }
    rt.joy_ignore(Joypad::empty());
    rt.set_spinning(false);
    rt.set_cur_map_script(default);
}
