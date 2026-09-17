//! What a map holds beside its blocks, read out of the cartridge: `<Map>_Object`'s warps, signs and
//! object events, the `warp_to` table behind them, and the per-map tables `LoadMapHeader` and
//! `InitMapSprites` consult (songs, sprite sets, toggleable objects).

use serde::{Deserialize, Serialize};
use crate::map::Map;
use crate::map_header::MapHeader;
use crate::rom_gfx::rom_slice;
use crate::symbols::{pokered_symbols, DmgBank, DmgPointer};

/// `LAST_MAP`: a warp back to whichever outside map was left for this one.
pub const LAST_MAP: u8 = 0xFF;
/// `WALK` and `STAY`, movement byte 1 of an object that is not being scripted.
pub const WALK: u8 = 0xFE;
pub const STAY: u8 = 0xFF;
/// `FIRST_INDOOR_MAP`: below it a map loads a fixed sprite set rather than its own pictures.
pub const FIRST_INDOOR_MAP: u8 = 0x25;
/// `FIRST_ROUTE_MAP`: below it a map is a town, which is marked visited.
pub const FIRST_ROUTE_MAP: u8 = 0x0C;
/// `FIRST_STILL_SPRITE`: from here a picture has one facing and four tiles.
pub const FIRST_STILL_SPRITE: u8 = 0x3D;
/// `SPRITE_SET_LENGTH`: nine walking pictures and two still ones.
pub const SPRITE_SET_LENGTH: usize = 11;

const BIT_TRAINER: u8 = 6;
const BIT_ITEM: u8 = 7;

/// `warp_event`: stand on `(x, y)` and arrive at `destination_warp` of `destination_map`, which is
/// the raw map id so that `LAST_MAP` survives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warp {
    pub y: u8,
    pub x: u8,
    /// Zero-based, as `wWarpEntries` keeps it.
    pub destination_warp: u8,
    pub destination_map: u8,
}

/// `bg_event`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sign {
    pub y: u8,
    pub x: u8,
    pub text_id: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectKind {
    Person,
    Trainer { class: u8, number: u8 },
    Item(u8),
}

/// `object_event`, with the coordinates carrying the `+ 4` the macro adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectEvent {
    pub picture: u8,
    pub map_y: u8,
    pub map_x: u8,
    /// `WALK`, `STAY`, or below them an index into a scripted path.
    pub movement1: u8,
    /// A range (`ANY_DIR`, `UP_DOWN`, `LEFT_RIGHT`) or a facing (`DOWN` ... `RIGHT`, `NONE`).
    pub movement2: u8,
    /// The low six bits of the text byte: an index into the map's text pointers.
    pub text_id: u8,
    pub kind: ObjectKind,
}

/// `warp_to`: where a player arriving at this warp stands and which block the view starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarpTo {
    /// `wCurrentTileBlockMapViewPointer`, an address in `wOverworldMap`.
    pub view: u16,
    pub y: u8,
    pub x: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapObjects {
    pub border_block: u8,
    pub warps: Vec<Warp>,
    pub signs: Vec<Sign>,
    pub objects: Vec<ObjectEvent>,
    /// One per warp, in the same order: the table `LoadDestinationWarpPosition` indexes.
    pub warp_to: Vec<WarpTo>,
}

impl MapObjects {
    /// `LoadMapHeader`'s walk over the object data, which runs straight on into `def_warps_to`.
    pub fn read(map: Map) -> Result<Self, String> {
        let header = MapHeader::read(map)?;
        let bytes = rom_slice(header.objects_pointer());
        let mut at = 0;
        let mut next = || {
            let byte = *bytes.get(at).ok_or_else(|| format!("{map}'s objects run off the bank"))?;
            at += 1;
            Ok::<u8, String>(byte)
        };
        let border_block = next()?;
        let warps = (0..next()?)
            .map(|_| Ok(Warp { y: next()?, x: next()?, destination_warp: next()?, destination_map: next()? }))
            .collect::<Result<Vec<_>, String>>()?;
        let signs = (0..next()?)
            .map(|_| Ok(Sign { y: next()?, x: next()?, text_id: next()? }))
            .collect::<Result<Vec<_>, String>>()?;
        let objects = (0..next()?)
            .map(|_| {
                let (picture, map_y, map_x, movement1, movement2, text) = (next()?, next()?, next()?, next()?, next()?, next()?);
                let kind = if text & 1 << BIT_TRAINER != 0 {
                    ObjectKind::Trainer { class: next()?, number: next()? }
                } else if text & 1 << BIT_ITEM != 0 {
                    ObjectKind::Item(next()?)
                } else {
                    ObjectKind::Person
                };
                Ok(ObjectEvent { picture, map_y, map_x, movement1, movement2, text_id: text & 0x3F, kind })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let warp_to = (0..warps.len())
            .map(|_| Ok(WarpTo { view: u16::from_le_bytes([next()?, next()?]), y: next()?, x: next()? }))
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self { border_block, warps, signs, objects, warp_to })
    }
}

/// `MapSongBanks`: the map's music id and the ROM bank of the audio engine it plays in.
pub fn map_song(map: Map) -> (u8, u8) {
    let row = rom_slice(pokered_symbols::MapSongBanks + map as u16 * 2);
    (row[0], row[1])
}

/// The text pointer a map's text id names, from the table its header puts in `wCurMapTextPtr`.
pub fn map_text_pointer(map: Map, text_id: u8) -> Result<DmgPointer, String> {
    let header = MapHeader::read(map)?;
    text_pointer_in(DmgPointer { bank: DmgBank::ROM { bank: header.header_bank }, address: header.text_address }, text_id)
}

/// The text pointer `text_id` names in the table at `table`, which a script may have put in
/// `wCurMapTextPtr` in place of the header's.
pub fn text_pointer_in(table: DmgPointer, text_id: u8) -> Result<DmgPointer, String> {
    let entries = rom_slice(table);
    let entry = (text_id as usize).checked_sub(1).ok_or("text id 0 is the start menu")? * 2;
    let address = u16::from_le_bytes([entries[entry], entries[entry + 1]]);
    // The shared sign texts, `MartSignText` and `PokeCenterSignText`, are in the home bank.
    Ok(DmgPointer { bank: if address < 0x4000 { DmgBank::ROM { bank: 0 } } else { table.bank }, address })
}

/// `FlyWarpDataPtr`'s entry for `map`: the square a fly or a blackout lands on and the view onto it.
/// The table has no terminator, so a map without an entry is `None` only because it is looked for
/// among the thirteen that have one.
pub fn fly_warp(map: Map) -> Option<WarpTo> {
    const ENTRIES: u16 = 13;
    let table = pokered_symbols::FlyWarpDataPtr;
    (0..ENTRIES).map(|i| rom_slice(table + i * 4)).find(|row| row[0] == map as u8).map(|row| {
        let data = rom_slice(DmgPointer { bank: table.bank, address: u16::from_le_bytes([row[2], row[3]]) });
        WarpTo { view: u16::from_le_bytes([data[0], data[1]]), y: data[2], x: data[3] }
    })
}

/// `ToggleableObjectMapPointers` for `map`: each toggleable object's map-local sprite id and its
/// index into the global `wToggleableObjectFlags`, as `MarkTownVisitedAndLoadToggleableObjects`
/// writes `wToggleableObjectList`.
pub fn toggleable_objects(map: Map) -> Vec<(u8, u8)> {
    let pointer = rom_slice(pokered_symbols::ToggleableObjectMapPointers + map as u16 * 2);
    let address = u16::from_le_bytes([pointer[0], pointer[1]]);
    let states = pokered_symbols::ToggleableObjectStates;
    let global = address.wrapping_sub(states.address) / 3;
    let rows = rom_slice(DmgPointer { bank: states.bank, address });
    rows.chunks_exact(3)
        .take_while(|row| row[0] != 0xFF && row[0] == map as u8)
        .enumerate()
        .map(|(i, row)| (row[1], (global as usize + i) as u8))
        .collect()
}

/// `InitializeToggleableObjectsFlags`: the flag array as a new game sets it, a bit set for every
/// object that starts `OFF`.
pub fn initial_toggleable_object_flags() -> Vec<u8> {
    const OFF: u8 = 0x11;
    let mut flags = vec![0; 32];
    let rows = rom_slice(pokered_symbols::ToggleableObjectStates);
    for (index, row) in rows.chunks_exact(3).take_while(|row| row[0] != 0xFF).enumerate() {
        if row[2] == OFF {
            flags[index / 8] |= 1 << (index % 8);
        }
    }
    flags
}

/// `InitOutsideMapSprites`' choice of sprite set for an outside map, `GetSplitMapSpriteSetID`
/// included; `None` indoors, where a map loads its own pictures.
pub fn sprite_set_id(map: Map, x: u8, y: u8) -> Option<u8> {
    const FIRST_SPLIT_SET: u8 = 0xF1;
    const SPLITSET_ROUTE_20: u8 = 0xF8;
    const EAST_WEST: u8 = 1;
    const SPRITESET_PALLET_VIRIDIAN: u8 = 0x01;
    const SPRITESET_FUCHSIA: u8 = 0x0A;
    if map as u8 >= FIRST_INDOOR_MAP {
        return None;
    }
    let id = rom_slice(pokered_symbols::MapSpriteSets + map as u16)[0];
    if id < FIRST_SPLIT_SET - 1 {
        return Some(id);
    }
    if id == SPLITSET_ROUTE_20 {
        return Some(match x {
            ..43 => SPRITESET_PALLET_VIRIDIAN,
            62.. => SPRITESET_FUCHSIA,
            _ if y < if x >= 55 { 8 } else { 13 } => SPRITESET_FUCHSIA,
            _ => SPRITESET_PALLET_VIRIDIAN,
        });
    }
    // `and $0f` then `dec a`: the split sets count from $F1.
    let row = rom_slice(pokered_symbols::SplitMapSpriteSets + ((id & 0x0F).wrapping_sub(1) as u16) * 4);
    let coordinate = if row[0] == EAST_WEST { x } else { y };
    Some(if coordinate < row[1] { row[2] } else { row[3] })
}

/// `SpriteSets`: the eleven pictures of a sprite set, one-based.
pub fn sprite_set(id: u8) -> [u8; SPRITE_SET_LENGTH] {
    let row = rom_slice(pokered_symbols::SpriteSets + (id as u16 - 1) * SPRITE_SET_LENGTH as u16);
    row[..SPRITE_SET_LENGTH].try_into().expect("a sprite set is eleven pictures")
}

/// A row of `SpriteSheetPointerTable`: where a picture's sheet is and how many bytes of it load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteSheet {
    pub pointer: DmgPointer,
    pub bytes: usize,
}

pub fn sprite_sheet(picture: u8) -> SpriteSheet {
    let row = rom_slice(pokered_symbols::SpriteSheetPointerTable + (picture as u16 - 1) * 4);
    SpriteSheet {
        pointer: DmgPointer { bank: DmgBank::ROM { bank: row[3] }, address: u16::from_le_bytes([row[0], row[1]]) },
        bytes: row[2] as usize,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `.PalletTown: fly_warp PALLET_TOWN, 5, 6`, in front of Red's house; the generated local label
    /// names the same row.
    #[test]
    fn a_blackout_in_kanto_s_south_lands_in_front_of_red_s_house() {
        let warp = fly_warp(Map::PalletTown).unwrap();
        assert_eq!((warp.x, warp.y), (5, 6));
        let width = MapHeader::read(Map::PalletTown).unwrap().width as u16;
        assert_eq!(warp.view, pokered_symbols::wOverworldMap.address + 7 + width + (width + 6) * 3 + 2);
        assert_eq!(rom_slice(crate::symbols::pokered_local_labels::FlyWarpDataPtr::PalletTown)[2..4], [6, 5]);
        assert!(fly_warp(Map::Route1).is_none());
    }

    #[test]
    fn pallet_town_s_objects_are_the_ones_its_file_lists() {
        let objects = MapObjects::read(Map::PalletTown).unwrap();
        assert_eq!(objects.border_block, 0x0B);
        assert_eq!(objects.warps, [
            Warp { y: 5, x: 5, destination_warp: 0, destination_map: Map::RedsHouse1F as u8 },
            Warp { y: 5, x: 13, destination_warp: 0, destination_map: Map::BluesHouse as u8 },
            Warp { y: 11, x: 12, destination_warp: 1, destination_map: Map::OaksLab as u8 },
        ]);
        assert_eq!(objects.signs.len(), 4);
        assert_eq!(objects.objects.iter().map(|o| (o.map_x - 4, o.map_y - 4, o.movement1)).collect::<Vec<_>>(),
            [(8, 5, STAY), (3, 8, WALK), (11, 14, WALK)]);
        // `event_displacement`: `wOverworldMap + 7 + width + (width + 6) * (y / 2) + x / 2`.
        let width = MapHeader::read(Map::PalletTown).unwrap().width as u16;
        assert_eq!(objects.warp_to[0], WarpTo {
            view: pokered_symbols::wOverworldMap.address + 7 + width + (width + 6) * 2 + 2, y: 5, x: 5,
        });
    }

    #[test]
    fn a_trainer_carries_two_more_bytes_and_an_item_one() {
        let objects = MapObjects::read(Map::ViridianForest).unwrap();
        assert!(objects.objects.iter().any(|o| matches!(o.kind, ObjectKind::Trainer { .. })));
        assert!(objects.objects.iter().any(|o| matches!(o.kind, ObjectKind::Item(_))));
        assert_eq!(objects.warp_to.len(), objects.warps.len());
    }

    #[test]
    fn every_map_s_objects_decode() {
        for map in Map::all().filter(|map| map.header_pointer().is_some()) {
            let objects = MapObjects::read(map).unwrap();
            for object in &objects.objects {
                assert!(object.picture > 0 && object.text_id > 0 || matches!(object.kind, ObjectKind::Item(_) | ObjectKind::Trainer { .. }),
                    "{map}: {object:?}");
            }
        }
    }

    #[test]
    fn route_2_splits_its_sprite_sets_at_row_37() {
        assert_eq!(sprite_set_id(Map::Route2, 5, 36), Some(0x02));
        assert_eq!(sprite_set_id(Map::Route2, 5, 37), Some(0x01));
        assert_eq!(sprite_set_id(Map::RedsHouse1F, 0, 0), None);
        assert_eq!(sprite_set(0x01)[0], crate::sprite::PictureId::Blue as u8);
    }

    #[test]
    fn oak_waits_hidden_in_pallet_town_until_he_is_needed() {
        let toggles = toggleable_objects(Map::PalletTown);
        assert_eq!(toggles, [(1, 0)], "PALLETTOWN_OAK is the first toggleable object in the game");
        assert_eq!(initial_toggleable_object_flags()[0] & 1, 1);
        let music = (pokered_symbols::Music_PalletTown.address - pokered_symbols::SFX_Headers_1.address) / 3;
        assert_eq!(map_song(Map::PalletTown), (music as u8, pokered_symbols::Music_PalletTown.bank.id()));
    }
}
