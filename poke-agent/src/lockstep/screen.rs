use gb::game_boy::GameBoy;
use gb::ram::ROM;
use pokered::gfx::compose::{HEIGHT, WIDTH};
use pokered::gfx::layers::{MapLayer, Object};
use pokered::gfx::tiles::TileData;
use pokered::gfx::Screen;
use crate::pokemon::map_header::TileSetId;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use super::to_vblank;

const MAP_BORDER: usize = 3;

fn settled(state: &[u8]) -> GameBoy {
    let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
    gb.load_state(state).unwrap();
    for _ in 0..60 {
        to_vblank(&mut gb);
    }
    gb
}

fn vram(gb: &GameBoy, address: u16, len: usize) -> Vec<u8> {
    gb.core().mmu().read_vram_slice(address, len).unwrap().to_vec()
}

/// What the cartridge drew from, as the compositor's inputs: its tile patterns, the overworld map
/// buffer and view, OAM and the palettes.
fn screen_from(gb: &GameBoy) -> Screen {
    let mmu = gb.core().mmu();
    let mut screen = Screen::default();
    screen.tiles.load(0, &vram(gb, 0x8000, 384 * 16));

    let tileset = TileSetId::from_repr(mmu.read_pointer(&pokered_symbols::wCurMapTileset)).unwrap();
    let blocks_wide = mmu.read_pointer(&pokered_symbols::wCurMapWidth) as usize + 2 * MAP_BORDER;
    let blocks_high = mmu.read_pointer(&pokered_symbols::wCurMapHeight) as usize + 2 * MAP_BORDER;
    let view = mmu.read_pointer_u16_le(&pokered_symbols::wCurrentTileBlockMapViewPointer)
        - pokered_symbols::wOverworldMap.address;
    let (x_half, y_half) = (mmu.read_pointer(&pokered_symbols::wXBlockCoord), mmu.read_pointer(&pokered_symbols::wYBlockCoord));
    screen.map = MapLayer {
        tileset: Some(tileset),
        blocks_wide,
        blocks: mmu.read_pointer_vec(&pokered_symbols::wOverworldMap, blocks_wide * blocks_high),
        camera: (
            (view as usize % blocks_wide) as i32 * 32 + x_half as i32 * 16,
            (view as usize / blocks_wide) as i32 * 32 + y_half as i32 * 16,
        ),
    };

    screen.sprites = (0..40u16).map(|i| {
        let at = 0xFE00 + i * 4;
        Object { y: mmu.read(at), x: mmu.read(at + 1), tile: mmu.read(at + 2), attributes: mmu.read(at + 3) }
    }).collect();
    screen.effects.bgp = mmu.read(0xFF47);
    screen.effects.obp0 = mmu.read(0xFF48);
    screen.effects.obp1 = mmu.read(0xFF49);
    screen
}

fn lcd_shades(gb: &GameBoy) -> Vec<u8> {
    gb.core().mmu().ppu().screenshot().pixels().map(|p| match p.0[0] {
        0xFF => 0,
        0xAA => 1,
        0x55 => 2,
        _ => 3,
    }).collect()
}

fn assert_same_frame(state: &[u8], name: &str) {
    let gb = settled(state);
    let composed = screen_from(&gb).frame();
    let lcd = lcd_shades(&gb);
    let wrong: Vec<(usize, usize)> = (0..WIDTH * HEIGHT)
        .filter(|&i| composed.shades[i] != lcd[i])
        .map(|i| (i % WIDTH, i / WIDTH))
        .collect();
    assert!(wrong.is_empty(), "{name}: {} pixels differ, first {:?}", wrong.len(), &wrong[..wrong.len().min(8)]);
}

#[test]
fn an_outdoor_frame_composes_to_the_cartridge_s_pixels() {
    assert_same_frame(include_bytes!("../pokemon/data/pallet-town-state.bin"), "Pallet Town");
}

#[test]
fn an_indoor_frame_composes_to_the_cartridge_s_pixels() {
    assert_same_frame(include_bytes!("../pokemon/data/oaks-lab-just-got-squirtle.bin"), "Oak's lab");
}

#[test]
fn the_loaders_put_the_same_patterns_where_the_cartridge_does() {
    let gb = settled(include_bytes!("../pokemon/data/pallet-town-state.bin"));
    let tileset = TileSetId::from_repr(gb.core().mmu().read_pointer(&pokered_symbols::wCurMapTileset)).unwrap();
    let mut tiles = TileData::default();
    tiles.load_tileset(tileset);
    tiles.load_text_box_tiles();
    let mut cartridge = TileData::default();
    cartridge.load(0, &vram(&gb, 0x8000, 384 * 16));
    // Tiles $03 and $14 of the tileset are the flower and water, which animate.
    for id in (0..0x80u8).filter(|&id| id != 0x03 && id != 0x14) {
        assert_eq!(tiles.bg(id), cartridge.bg(id), "vChars2 tile ${id:02X}");
    }
}

/// The overworld keeps NPC patterns in `vFont` until a text box loads the font over them.
#[test]
fn the_font_loads_where_the_cartridge_puts_it() {
    let mut gb = super::text_box::boot_to_oaks_first_text();
    to_vblank(&mut gb);
    let mut tiles = TileData::default();
    tiles.load_font();
    let mut cartridge = TileData::default();
    cartridge.load(0, &vram(&gb, 0x8000, 384 * 16));
    for id in 0x80..=0xFFu8 {
        assert_eq!(tiles.bg(id), cartridge.bg(id), "vFont tile ${id:02X}");
    }
}

#[test]
fn water_and_flowers_animate_on_the_cartridge_s_frames() {
    let mut gb = settled(include_bytes!("../pokemon/data/pallet-town-state.bin"));
    let mut tiles = TileData::default();
    tiles.load(0, &vram(&gb, 0x8000, 384 * 16));
    let mmu = gb.core().mmu();
    tiles.animation.kind = mmu.read_pointer(&pokered_symbols::hTileAnimations);
    tiles.animation.counter1 = mmu.read_pointer(&pokered_symbols::hMovingBGTilesCounter1);
    tiles.animation.counter2 = mmu.read_pointer(&pokered_symbols::wMovingBGTilesCounter2);
    assert_eq!(tiles.animation.kind, 2, "Pallet Town has water and flowers");
    let mut changes = 0;
    for frame in 0..200 {
        let before = *tiles.bg(0x14);
        to_vblank(&mut gb);
        tiles.update_moving_bg_tiles();
        let mut cartridge = TileData::default();
        cartridge.load(0, &vram(&gb, 0x8000, 384 * 16));
        for id in [0x03, 0x14] {
            assert_eq!(tiles.bg(id), cartridge.bg(id), "tile ${id:02X}, frame {frame}");
        }
        changes += (before != *tiles.bg(0x14)) as u32;
    }
    assert!(changes > 5, "the water moved {changes} times");
}

#[test]
fn the_overworld_buffer_is_built_as_the_cartridge_builds_it() {
    use crate::pokemon::map::Map;
    for state in [
        &include_bytes!("../pokemon/data/pallet-town-state.bin")[..],
        include_bytes!("../pokemon/data/oaks-lab-just-got-squirtle.bin"),
        include_bytes!("../pokemon/data/at-celadon.bin"),
        include_bytes!("../pokemon/data/at-cerulean.bin"),
    ] {
        let gb = settled(state);
        let mmu = gb.core().mmu();
        let map = Map::from_repr(mmu.read_pointer(&pokered_symbols::wCurMap)).unwrap();
        let ours = pokered::systems::map_data::tile_block_map(map).unwrap();
        assert_eq!(ours, mmu.read_pointer_vec(&pokered_symbols::wOverworldMap, ours.len()), "{map}");
        let (x, y) = (mmu.read_pointer(&pokered_symbols::wXCoord), mmu.read_pointer(&pokered_symbols::wYCoord));
        assert_eq!(pokered::systems::map_data::camera(x, y), screen_from(&gb).map.camera, "{map} at ({x}, {y})");
    }
}
