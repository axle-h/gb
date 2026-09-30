//! The battle animations: `AttackAnimationPointers`' command streams, `SubanimationPointers`,
//! `FrameBlockPointers`, `FrameBlockBaseCoords`, `MoveSoundTable` and the pic tile id lists of
//! `TileIDListPointerTable`.

use serde::{Deserialize, Serialize};

/// `FIRST_SE_ID`: a command byte from here on is a special effect, below it a subanimation.
pub const FIRST_SE_ID: u8 = 0xC0;
/// `NO_MOVE - 1`, the sound byte of a command that plays none.
pub const NO_SOUND: u8 = 0xFF;

/// One command of an animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimCommand {
    /// `battle_anim sound, subanimation, tileset, delay`: `sound` is the move whose sound plays,
    /// less one, and `delay` the frames each frame block is held.
    Subanimation { tileset: u8, delay: u8, sound: u8, id: u8 },
    /// `battle_anim sound, special_effect`.
    SpecialEffect { id: u8, sound: u8 },
}

/// Animation `id`'s commands, from 1 as `wAnimationID` counts, up to its `-1`.
pub fn attack_animation(id: u8) -> Vec<AnimCommand> {
    assert!(id != 0, "animation 0 is no animation");
    crate::tables::ATTACK_ANIMATIONS[id as usize - 1].to_vec()
}

/// `SUBANIMTYPE_*`, the top three bits of a subanimation's first byte.
pub mod subanim_type {
    pub const NORMAL: u8 = 0;
    pub const HVFLIP: u8 = 1;
    pub const HFLIP: u8 = 2;
    pub const COORDFLIP: u8 = 3;
    pub const REVERSE: u8 = 4;
    pub const ENEMY: u8 = 5;
}

/// `FRAMEBLOCKMODE_*`.
pub mod frame_block_mode {
    pub const MODE_00: u8 = 0;
    pub const MODE_02: u8 = 2;
    pub const MODE_03: u8 = 3;
    pub const MODE_04: u8 = 4;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubanimEntry {
    pub frame_block: u8,
    pub base_coord: u8,
    pub mode: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subanimation {
    /// `SUBANIMTYPE_*`.
    pub kind: u8,
    pub entries: Vec<SubanimEntry>,
}

/// `SubanimationPointers` entry `id`. The count is five bits, so 0 would be read as 256 by nothing:
/// `PlaySubanimation` stops when the count reaches 0 after a block.
pub fn subanimation(id: u8) -> Subanimation {
    let (kind, entries) = crate::tables::SUBANIMATIONS[id as usize];
    let entries = entries.iter().map(|&[frame_block, base_coord, mode]| SubanimEntry { frame_block, base_coord, mode }).collect();
    Subanimation { kind, entries }
}

/// One `dbsprite` of a frame block: offsets from the base coordinate, the tile past `$31` and the
/// OAM attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameBlockTile {
    pub y: u8,
    pub x: u8,
    pub tile: u8,
    pub flags: u8,
}

/// `FrameBlockPointers` entry `id`: as many tiles as its count byte says, whatever the source
/// lists, since a block that lists fewer runs on into the next one. `FrameBlock00` counts 0, which
/// `DrawFrameBlock` would take as 256, and no subanimation uses it.
pub fn frame_block(id: u8) -> Vec<FrameBlockTile> {
    crate::tables::FRAME_BLOCKS[id as usize].iter().map(|&[y, x, tile, flags]| FrameBlockTile { y, x, tile, flags }).collect()
}

/// `FrameBlockBaseCoords` entry `id`, as `(y, x)`.
pub fn base_coord(id: u8) -> (u8, u8) {
    crate::tables::FRAME_BLOCK_BASE_COORDS[id as usize]
}

/// A row of `MoveSoundTable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveSound {
    pub sound: u8,
    pub pitch: u8,
    pub tempo: u8,
}

/// `MoveSoundTable` row `index`, which is a command's sound byte.
pub fn move_sound(index: u8) -> MoveSound {
    let (sound, pitch, tempo) = crate::audio::MOVE_SOUND_TABLE[index as usize];
    MoveSound { sound, pitch, tempo }
}

/// `TILEMAP_*`.
pub mod tilemap {
    pub const MON_PIC: u8 = 0;
    pub const SLIDE_DOWN_MON_PIC_7X5: u8 = 1;
    pub const SLIDE_DOWN_MON_PIC_7X3: u8 = 2;
}

/// `GetTileIDList`: the tile ids, rows and columns of list `index`.
pub fn tile_id_list(index: u8) -> (&'static [u8], usize, usize) {
    let (ids, columns, rows) = crate::gfx::TILE_ID_LISTS[index as usize];
    (ids, rows, columns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn source(path: &str) -> String {
        std::fs::read_to_string(format!("{}/../vendor/pokered/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    /// A `const_def` list's names, numbered from its start.
    fn consts(text: &str, first: &str) -> HashMap<String, usize> {
        let mut names = HashMap::new();
        let mut next = 0;
        let mut started = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with(&format!("const {first}")) {
                started = true;
            }
            if !started {
                continue;
            }
            if let Some(name) = line.strip_prefix("const ") {
                names.insert(name.split_whitespace().next().unwrap().to_string(), next);
                next += 1;
            } else if line.starts_with("DEF ") || line.starts_with("const_def") {
                break;
            }
        }
        names
    }

    fn number(text: &str) -> usize {
        let text = text.trim();
        match text.strip_prefix('$') {
            Some(hex) => usize::from_str_radix(hex, 16).unwrap(),
            None => text.parse().unwrap(),
        }
    }

    fn label_blocks(text: &str) -> Vec<(String, Vec<String>)> {
        let mut blocks: Vec<(String, Vec<String>)> = vec![];
        for line in text.lines() {
            let line = line.split(';').next().unwrap().trim_end();
            if let Some(label) = line.strip_suffix(':') {
                blocks.push((label.to_string(), vec![]));
            } else if !line.trim().is_empty() && let Some(block) = blocks.last_mut() {
                block.1.push(line.trim().to_string());
            }
        }
        blocks
    }

    #[test]
    fn every_subanimation_is_the_source_s() {
        let constants = source("constants/move_animation_constants.asm");
        let frame_blocks = consts(&constants, "FRAMEBLOCK_00");
        let base_coords: HashMap<String, usize> = (0..=0xFF).map(|i| (format!("BASECOORD_{i:02X}"), i)).collect();
        let text = source("data/battle_anims/subanimations.asm");
        let pointers: Vec<String> = text.lines().filter_map(|line| line.trim().strip_prefix("dw ")).map(|s| s.split_whitespace().next().unwrap().to_string()).collect();
        let blocks: HashMap<String, Vec<String>> = label_blocks(&text).into_iter().collect();
        let types = consts(&constants, "SUBANIMTYPE_NORMAL");
        let mut checked = 0;
        for (id, label) in pointers.iter().enumerate() {
            let lines = &blocks[label];
            let header = lines[0].strip_prefix("subanim ").unwrap();
            let (kind, count) = header.split_once(',').unwrap();
            let decoded = subanimation(id as u8);
            assert_eq!(decoded.kind as usize, types[kind.trim()], "{label}");
            assert_eq!(decoded.entries.len(), number(count), "{label}");
            for (entry, line) in decoded.entries.iter().zip(&lines[1..]) {
                let fields: Vec<&str> = line.strip_prefix("db ").unwrap().split(',').map(str::trim).collect();
                assert_eq!(entry.frame_block as usize, frame_blocks[fields[0]], "{label}: {line}");
                assert_eq!(entry.base_coord as usize, base_coords[fields[1]], "{label}: {line}");
                assert_eq!(entry.mode as usize, number(fields[2].trim_start_matches("FRAMEBLOCKMODE_")), "{label}: {line}");
            }
            checked += 1;
        }
        assert_eq!(checked, 86);
    }

    #[test]
    fn every_frame_block_and_base_coordinate_is_the_source_s() {
        let text = source("data/battle_anims/frame_blocks.asm");
        let pointers: Vec<String> = text.lines().filter_map(|line| line.trim().strip_prefix("dw ")).map(str::to_string).collect();
        let blocks: HashMap<String, Vec<String>> = label_blocks(&text).into_iter().collect();
        for (id, label) in pointers.iter().enumerate() {
            let listed: Vec<FrameBlockTile> = blocks[label].iter().filter_map(|line| line.strip_prefix("dbsprite")).map(|args| {
                let n: Vec<usize> = args.split(',').map(|field| field.split('|').map(|flag| match flag.trim() {
                    "OAM_XFLIP" => 0x20,
                    "OAM_YFLIP" => 0x40,
                    "OAM_PAL1" => 0x10,
                    "OAM_PRIO" => 0x80,
                    flag => number(flag),
                }).fold(0, |a, b| a | b)).collect();
                FrameBlockTile { y: ((n[1] * 8) % 256 + n[3]) as u8, x: ((n[0] * 8) % 256 + n[2]) as u8, tile: n[4] as u8, flags: n[5] as u8 }
            }).collect();
            let count = number(blocks[label][0].strip_prefix("db ").unwrap());
            let decoded = frame_block(id as u8);
            assert_eq!(decoded.len(), count, "{label}");
            let shared = listed.len().min(decoded.len());
            assert_eq!(&decoded[..shared], &listed[..shared], "{label}");
        }
        let coords = source("data/battle_anims/base_coords.asm");
        for (id, line) in coords.lines().filter_map(|line| line.trim().strip_prefix("db ")).enumerate() {
            let line = line.split(';').next().unwrap();
            let (y, x) = line.split_once(',').unwrap();
            assert_eq!(base_coord(id as u8), (number(y) as u8, number(x) as u8), "BASECOORD_{id:02X}");
        }
    }

    #[test]
    fn every_attack_animation_is_the_source_s() {
        let constants = source("constants/move_animation_constants.asm");
        let subanims = consts(&constants, "SUBANIM_0_STAR");
        let moves = consts(&source("constants/move_constants.asm"), "POUND");
        let text = source("data/moves/animations.asm");
        let effects: HashMap<String, usize> = constants.lines().filter_map(|line| {
            let line = line.trim().strip_prefix("const SE_")?;
            let name = line.split_whitespace().next()?;
            let value = line.split('$').nth(1)?.split_whitespace().next()?;
            Some((format!("SE_{name}"), usize::from_str_radix(value, 16).ok()?))
        }).collect();
        let pointers: Vec<String> = text.lines().filter_map(|line| line.trim().strip_prefix("dw ")).map(str::to_string).collect();
        let blocks = label_blocks(&text);
        let body = |label: &str| {
            let at = blocks.iter().position(|(name, _)| name == label).unwrap();
            blocks[at..].iter().find(|(_, lines)| !lines.is_empty()).unwrap().1.clone()
        };
        for (index, label) in pointers.iter().enumerate() {
            let lines: Vec<String> = body(label).into_iter().take_while(|line| line != "db -1").collect();
            let decoded = attack_animation(index as u8 + 1);
            assert_eq!(decoded.len(), lines.len(), "{label}");
            for (command, line) in decoded.iter().zip(&lines) {
                let fields: Vec<&str> = line.strip_prefix("battle_anim ").unwrap().split(',').map(str::trim).collect();
                let sound = if fields[0] == "NO_MOVE" { NO_SOUND } else { moves[fields[0]] as u8 };
                let expected = if fields.len() == 4 {
                    AnimCommand::Subanimation { tileset: number(fields[2]) as u8, delay: number(fields[3]) as u8, sound, id: subanims[fields[1]] as u8 }
                } else {
                    AnimCommand::SpecialEffect { id: effects[fields[1]] as u8, sound }
                };
                assert_eq!(*command, expected, "{label}: {line}");
            }
        }
    }

    #[test]
    fn every_move_sound_is_the_source_s() {
        let text = source("data/moves/sfx.asm");
        for (index, line) in text.lines().filter_map(|line| line.trim().strip_prefix("db ")).enumerate() {
            let fields: Vec<&str> = line.split(';').next().unwrap().split(',').map(str::trim).collect();
            let row = move_sound(index as u8);
            assert_eq!((row.pitch, row.tempo), (number(fields[1]) as u8, number(fields[2]) as u8), "row {index}: {line}");
        }
    }

    #[test]
    fn the_pic_tile_lists_are_seven_by_seven_and_slid_down() {
        let (tiles, rows, columns) = tile_id_list(tilemap::MON_PIC);
        assert_eq!((rows, columns), (7, 7));
        assert_eq!(tiles, (0..49).map(|i| (i % 7) * 7 + i / 7).collect::<Vec<u8>>(), "row-major over column-major tiles");
        assert_eq!(tile_id_list(tilemap::SLIDE_DOWN_MON_PIC_7X5).1, 5);
        assert_eq!(tile_id_list(tilemap::SLIDE_DOWN_MON_PIC_7X3).1, 3);
    }
}
