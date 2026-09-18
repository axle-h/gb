//! `VermilionDock_Script`: the S.S. Anne sailing off as the player comes down its gangway with
//! HM01, and the walk off the dock after.

use poke_core::rom_gfx::{rom_slice, TILE_BYTES};
use poke_core::symbols::pokered_events::{EVENT_GOT_HM01, EVENT_SS_ANNE_LEFT, EVENT_STARTED_WALKING_OUT_OF_DOCK,
    EVENT_WALKED_OUT_OF_DOCK};
use poke_core::symbols::pokered_symbols as sym;
use serde::{Deserialize, Serialize};
use crate::audio::data::{sounds, SoundId};
use crate::gfx::layers::{Object, TileMap, Window};
use crate::gfx::tiles::V_CHARS1;
use crate::gfx::ui::{SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::input::Joypad;
use super::{Flow, Script};

/// The warp from the S.S. Anne's 1F.
const SS_ANNE_WARP: u8 = 1;
const WATER_TILE: u8 = 0x14;
const WATER_BLOCK: u8 = 0x0d;
/// The screen rows the ship is drawn on, which `SyncScrollWithLY` scrolls from line `$50` to `$80`.
const SHIP_ROWS: std::ops::Range<usize> = 10..16;
/// `.shift_columns_up`'s count, and `.smoke_puff_drift_loop`'s and `.delay_between_drifts`'.
const COLUMN_SHIFTS: u8 = 8;
const DRIFTS: u8 = 16;
const DRIFT_FRAMES: u8 = 8;
/// `WriteOAMBlock`'s slot 1: OAM entries 4 to 7.
const SMOKE_OAM: usize = 4;
const SMOKE_Y: u8 = 100;
/// `wSSAnneSmokeX` before the first puff.
const SMOKE_X: u8 = 88;
/// `vChars1 tile $7c`, where the puff is loaded, as an object's tile.
const SMOKE_TILE: u8 = 0xFC;
const OAM_COUNT: usize = 40;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    /// `VermilionDock_Script` after its `Delay3`.
    WalkOut,
    /// `VermilionDockSSAnneLeavesScript` after `DelayFrames 120`.
    ShipLeaving,
    /// After `PlaySoundWaitForCurrent`'s wait.
    Horn,
    /// After a drift's frames: `column` of `.shift_columns_up`, `step` of `.smoke_puff_drift_loop`.
    Drift { column: u8, step: u8 },
    /// After `VermilionDock_EraseSSAnne`'s `DelayFrames 120`.
    Erased,
}

pub fn script(rt: &mut Script) -> Flow {
    rt.enable_auto_text_box_drawing();
    if rt.check_event(EVENT_STARTED_WALKING_OUT_OF_DOCK) {
        if rt.check_event(EVENT_WALKED_OUT_OF_DOCK) || rt.simulated_joypad_states_index() != 0 {
            return Flow::Return;
        }
        rt.joy_ignore(Joypad::empty());
        rt.set_event(EVENT_WALKED_OUT_OF_DOCK);
        return Flow::Return;
    }
    if !rt.check_event(EVENT_GOT_HM01) || rt.destination_warp_id() != SS_ANNE_WARP {
        return Flow::Return;
    }
    if !rt.check_event(EVENT_SS_ANNE_LEFT) {
        return ss_anne_leaves(rt);
    }
    rt.set_event(EVENT_STARTED_WALKING_OUT_OF_DOCK);
    rt.delay3().then(Label::WalkOut)
}

pub fn text(_rt: &mut Script, _text_id: u8) -> Option<Flow> {
    None
}

/// `VermilionDockSSAnneLeavesScript`.
fn ss_anne_leaves(rt: &mut Script) -> Flow {
    rt.set_event(EVENT_SS_ANNE_LEFT);
    rt.joy_ignore(Joypad::all());
    rt.play_new_sound(SoundId::STOP_ALL_MUSIC);
    rt.play_music(sounds::MUSIC_SURFING);
    // `LoadSmokeTileFourTimes`, over the player's walking pictures.
    let smoke = &rom_slice(sym::SSAnneSmokePuffTile)[..TILE_BYTES];
    for i in 0..4 {
        rt.screen().tiles.load(V_CHARS1 + 0x7C + i, smoke);
    }
    rt.set_player_image_index(0);
    rt.delay_frames(120).then(Label::ShipLeaving)
}

/// `wTileMap` with the ship's rows filled with water, as the script leaves it: what the window
/// shows at the end and what each redrawn column is taken from.
fn tile_map_without_ship(rt: &Script, column: usize, row: usize) -> u8 {
    if SHIP_ROWS.contains(&row) { WATER_TILE } else { rt.map_view_tile(column, row) }
}

pub fn resume(rt: &mut Script, label: Label) -> Flow {
    match label {
        Label::WalkOut => {
            rt.simulate_joypad_presses(vec![Joypad::UP; 3]);
            rt.joy_ignore(Joypad::all());
            Flow::Return
        }
        // The copy of the screen into `vBGMap1` and the water's transfer after it wait on VRAM
        // behind a hidden window, so they take no frames. `vBGMap0` is the screen as it stands.
        Label::ShipLeaving => {
            let mut background = TileMap::filled(WATER_TILE);
            for row in 0..SCREEN_TILES_Y {
                for column in 0..SCREEN_TILES_X {
                    background.set(column, row, rt.map_view_tile(column, row));
                }
            }
            let screen = rt.screen();
            screen.background = Some(background);
            screen.effects.obp1 = 0;
            rt.wait_for_sound_to_finish().then(Label::Horn)
        }
        Label::Horn => {
            rt.play_sound(sounds::SFX_SS_ANNE_HORN);
            rt.set_update_sprites_enabled(false);
            shift_columns(rt, 0)
        }
        Label::Drift { column, step } if step + 1 < DRIFTS => drift(rt, column, step + 1),
        Label::Drift { column, .. } if column + 1 < COLUMN_SHIFTS => shift_columns(rt, column + 1),
        Label::Drift { .. } => erase_ss_anne(rt),
        Label::Erased => {
            let screen = rt.screen();
            screen.window = None;
            screen.background = None;
            rt.set_update_sprites_enabled(true);
            rt.load_player_sprite_graphics();
            rt.decrement_number_of_warps();
            Flow::Return
        }
    }
}

/// `.shift_columns_up`: `wMapViewVRAMPointer` two tiles on and `ScheduleEastColumnRedraw` there,
/// wrapping round `vBGMap0`, then `VermilionDock_EmitSmokePuff`.
fn shift_columns(rt: &mut Script, column: u8) -> Flow {
    let dest = SCREEN_TILES_X + 2 * column as usize;
    let tiles: Vec<[u8; 2]> =
        (0..SCREEN_TILES_Y).map(|row| [tile_map_without_ship(rt, 18, row), tile_map_without_ship(rt, 19, row)]).collect();
    let screen = rt.screen();
    if let Some(background) = &mut screen.background {
        for (row, pair) in tiles.into_iter().enumerate() {
            background.set(dest, row, pair[0]);
            background.set(dest + 1, row, pair[1]);
        }
    }
    let x = SMOKE_X - 16 * (column + 1);
    screen.sprites.resize(OAM_COUNT, Object { y: 160, ..Object::default() });
    for (i, (dy, dx)) in [(0, 0), (0, 8), (8, 0), (8, 8)].into_iter().enumerate() {
        screen.sprites[SMOKE_OAM + i] = Object { y: SMOKE_Y + dy, x: x + dx, tile: SMOKE_TILE + i as u8, attributes: Object::OBP1 };
    }
    drift(rt, column, 0)
}

/// One pass of `.smoke_puff_drift_loop`: `VermilionDock_AnimSmokePuffDriftRight`, then the frames
/// of `VermilionDock_SyncScrollWithLY` with the ship's lines scrolled `d` pixels.
fn drift(rt: &mut Script, column: u8, step: u8) -> Flow {
    // `wSSAnneSmokeDriftAmount` times 16 entries drift, which runs past OAM into `wTileMap` from
    // the third puff; the tile map is rebuilt before it is next drawn, so only OAM is moved here.
    let entries = 16 * (column as usize + 1);
    let screen = rt.screen();
    for object in screen.sprites.iter_mut().skip(SMOKE_OAM).take(entries) {
        object.x = object.x.wrapping_add(2);
    }
    let d = column * DRIFTS + step;
    let top = SHIP_ROWS.start * 8;
    let bottom = SHIP_ROWS.end * 8;
    let scx = screen.effects.scx;
    screen.effects.line_scx = Some((0..144).map(|line| if line < top { scx } else if line < bottom { d } else { 0 }).collect());
    rt.delay_frames(DRIFT_FRAMES).then(Label::Drift { column, step })
}

/// `VermilionDock_EraseSSAnne`, behind the window: the ship's rows of the background become water,
/// and its blocks with them.
fn erase_ss_anne(rt: &mut Script) -> Flow {
    let mut window = TileMap::filled(WATER_TILE);
    for row in 0..SCREEN_TILES_Y {
        for column in 0..SCREEN_TILES_X {
            window.set(column, row, tile_map_without_ship(rt, column, row));
        }
    }
    let screen = rt.screen();
    screen.window = Some(Window { x: 7, y: 0, tiles: window });
    screen.effects.line_scx = None;
    if let Some(background) = &mut screen.background {
        for row in SHIP_ROWS {
            for column in 0..TileMap::SIZE {
                background.set(column, row, WATER_TILE);
            }
        }
    }
    // The cartridge replaces only the lower half's blocks and leaves the tiles above them water;
    // the screen here draws from blocks, so the ship's top row of tiles is kept as water by hand.
    for x in 5..9 {
        rt.replace_tile_block(x, 2, WATER_BLOCK);
    }
    for row in SHIP_ROWS {
        for column in 0..SCREEN_TILES_X {
            rt.overwrite_bg_tile(column as u8, row as u8, WATER_TILE);
        }
    }
    rt.play_sound(sounds::SFX_SS_ANNE_HORN);
    rt.delay_frames(120).then(Label::Erased)
}
