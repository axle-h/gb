//! `ItemUseBicycle` and `ItemUseSurfboard`, which the bag and the party menu's SURF call into: their
//! checks and what they change before `PlayDefaultMusic`. The caller runs the music's wait and the
//! text over its own screen; the overworld takes SURF's step forward once the start menu has closed
//! (`Overworld::surf_step`).

use poke_core::map_header::MapHeader;
use poke_core::symbols::{pokered_symbols, DmgPointer};
use crate::gfx::ui::{SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use crate::mode::Ctx;
use crate::systems::overworld::bike_surf::{is_bike_riding_allowed, no_place_to_get_off, no_surfing_here};
use crate::systems::overworld::location::{BIKING, SURFING, WALKING};
use crate::systems::overworld::sprites::load_walking_player_sprite_graphics;

/// What using the bike or the surfboard came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Used {
    /// `wActionResultOrTookBattleTurn`: the start menu closes after, rather than the menu coming back.
    pub result: bool,
    /// `PlayDefaultMusic` runs, before the text.
    pub music: bool,
    pub text: Option<DmgPointer>,
}

impl Used {
    fn failed(text: DmgPointer) -> Self {
        Self { result: false, music: false, text: Some(text) }
    }
}

fn tileset(ctx: &Ctx) -> poke_core::map_header::TileSetId {
    MapHeader::read(ctx.world.location.map).expect("the player stands on a map with a header").tileset
}

/// `ItemUseBicycle`, out of battle: on or off, with `ItemUseReloadOverworldData` putting the map back
/// over the menus first.
pub fn item_use_bicycle(ctx: &mut Ctx) -> Used {
    let location = &ctx.world.location;
    if location.walk_bike_surf == SURFING {
        return Used::failed(pokered_symbols::ItemUseNotTimeText);
    }
    let getting_on = location.walk_bike_surf != BIKING;
    if getting_on && !is_bike_riding_allowed(location.map, tileset(ctx)) {
        return Used::failed(pokered_symbols::NoCyclingAllowedHereText);
    }
    ctx.screen.ui.uncover(0, 0, SCREEN_TILES_X, SCREEN_TILES_Y);
    ctx.update_sprites = true;
    let text = if getting_on {
        ctx.pad.held = Joypad::empty();
        ctx.world.location.walk_bike_surf = BIKING;
        pokered_symbols::GotOnBicycleText
    } else {
        ctx.world.location.walk_bike_surf = WALKING;
        pokered_symbols::GotOffBicycleText
    };
    Used { result: true, music: true, text: Some(text) }
}

/// `ItemUseSurfboard`, judged on what was in front of the player as the start menu opened.
pub fn item_use_surfboard(ctx: &mut Ctx) -> Used {
    let tileset = tileset(ctx);
    let location = &mut ctx.world.location;
    if location.walk_bike_surf != SURFING {
        if no_surfing_here(tileset, &location.ahead) {
            return Used::failed(pokered_symbols::NoSurfingHereText);
        }
        location.walk_bike_surf = SURFING;
        return Used { result: true, music: true, text: Some(pokered_symbols::SurfingGotOnText) };
    }
    if no_place_to_get_off(tileset, &location.ahead) {
        return Used { result: true, music: false, text: Some(pokered_symbols::SurfingNoPlaceToGetOffText) };
    }
    location.walk_bike_surf = WALKING;
    ctx.pad.ignore = Joypad::all();
    // `PlayDefaultMusic` comes before this on the cartridge; nothing between them reads the tiles.
    load_walking_player_sprite_graphics(&mut ctx.screen.tiles);
    Used { result: true, music: true, text: None }
}
