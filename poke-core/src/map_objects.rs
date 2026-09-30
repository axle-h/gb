//! What a map holds beside its blocks: `<Map>_Object`'s warps, signs and
//! object events, the `warp_to` table behind them, and the per-map tables `LoadMapHeader` and
//! `InitMapSprites` consult (songs, sprite sets, toggleable objects).

use serde::{Deserialize, Serialize};
use crate::map::Map;
use crate::tables::TEXT_POINTERS;

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
    /// `wCurrentTileBlockMapViewPointer`, as an offset into `wOverworldMap`.
    pub view: u16,
    pub y: u8,
    pub x: u8,
}

impl WarpTo {
    /// `event_displacement`: the view that puts `(x, y)` of a map `width` blocks wide on the
    /// player's square.
    pub fn at(width: u8, x: u8, y: u8) -> Self {
        let width = width as u16;
        WarpTo { view: 7 + width + (width + 6) * (y >> 1) as u16 + (x >> 1) as u16, y, x }
    }

    /// A `fly_warp`'s landing: `(x, y)` on `map`.
    pub fn fly_warp((map, x, y): (u8, u8, u8)) -> Self {
        let header = crate::tables::MAP_HEADERS[map as usize].expect("a fly warp lands on a map with a header");
        WarpTo::at(header.width, x, y)
    }
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
    /// `map`'s events as `LoadMapHeader` reads them: the macros' `+ 4` on an object's coordinates,
    /// `- 1` on a warp's destination and bits on a trainer's or item's text id applied, and the
    /// `warp_to` rows `def_warps_to` generates.
    pub fn read(map: Map) -> Result<Self, String> {
        let header = crate::tables::MAP_HEADERS[map as usize].ok_or_else(|| format!("{map} has no header of its own"))?;
        let source = header.objects;
        Ok(Self {
            border_block: source.border_block,
            warps: source.warps.iter()
                .map(|warp| Warp { y: warp.y, x: warp.x, destination_warp: warp.warp.wrapping_sub(1), destination_map: warp.map })
                .collect(),
            signs: source.signs.iter().map(|sign| Sign { y: sign.y, x: sign.x, text_id: sign.text_id }).collect(),
            objects: source.objects.iter().map(|object| ObjectEvent {
                picture: object.sprite,
                map_y: object.y + 4,
                map_x: object.x + 4,
                movement1: object.movement,
                movement2: object.range_or_direction,
                text_id: object.text_id,
                kind: object.kind,
            }).collect(),
            warp_to: source.warps.iter().map(|warp| WarpTo::at(header.width, warp.x, warp.y)).collect(),
        })
    }
}

/// `MapSongBanks`: the map's music id and the ROM bank of the audio engine it plays in.
pub fn map_song(map: Map) -> (u8, u8) {
    crate::audio::MAP_SONG_BANKS[map as usize]
}

/// A map's text pointer table, what `wCurMapTextPtr` holds: one of `TEXT_POINTERS`. Saved by its
/// label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextPointers(u16);

impl TextPointers {
    pub(crate) const fn new(index: u16) -> Self {
        Self(index)
    }

    /// The table `map`'s header names.
    pub fn of(map: Map) -> Option<Self> {
        let header = crate::tables::MAP_HEADERS[map as usize]?;
        Self::named(&format!("{}_TextPointers", header.name))
    }

    pub fn named(label: &str) -> Option<Self> {
        TEXT_POINTERS.binary_search_by(|(name, _)| (*name).cmp(label)).ok().map(|at| Self(at as u16))
    }

    pub fn label(self) -> &'static str {
        TEXT_POINTERS[self.0 as usize].0
    }

    /// The label of the text `text_id` names.
    pub fn text(self, text_id: u8) -> Result<&'static str, String> {
        let (table, texts) = TEXT_POINTERS[self.0 as usize];
        let entry = (text_id as usize).checked_sub(1).ok_or("text id 0 is the start menu")?;
        texts.get(entry).copied().ok_or_else(|| format!("{table} has no text {text_id}"))
    }
}

impl Serialize for TextPointers {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.label())
    }
}

impl<'de> Deserialize<'de> for TextPointers {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        crate::symbols::SavedLabel::resolve(deserializer, Self::named)
    }
}

/// `FlyWarpDataPtr`'s entry for `map`: the square a fly or a blackout lands on and the view onto it.
pub fn fly_warp(map: Map) -> Option<WarpTo> {
    crate::tables::FLY_WARP_DATA.iter().find(|&&(to, _, _)| to == map as u8).map(|&warp| WarpTo::fly_warp(warp))
}

/// `ToggleableObjectMapPointers` for `map`: each toggleable object's map-local sprite id and its
/// index into the global `wToggleableObjectFlags`, as `MarkTownVisitedAndLoadToggleableObjects`
/// writes `wToggleableObjectList`. The pointer names the map's first row, and its rows run to the
/// first row of another map.
pub fn toggleable_objects(map: Map) -> Vec<(u8, u8)> {
    let states = &crate::tables::TOGGLEABLE_OBJECT_STATES;
    let Some(first) = states.iter().position(|&(of, _, _)| of == map as u8) else { return Vec::new() };
    states[first..].iter().take_while(|&&(of, _, _)| of == map as u8)
        .enumerate()
        .map(|(i, &(_, object, _))| (object, (first + i) as u8))
        .collect()
}

/// `InitializeToggleableObjectsFlags`: the flag array as a new game sets it, a bit set for every
/// object that starts `OFF`.
pub fn initial_toggleable_object_flags() -> Vec<u8> {
    let mut flags = vec![0; 32];
    for (index, &(_, _, shown)) in crate::tables::TOGGLEABLE_OBJECT_STATES.iter().enumerate() {
        if !shown {
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
    let id = crate::tables::MAP_SPRITE_SETS[map as usize];
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
    let (split, line, before, after) = crate::tables::SPLIT_MAP_SPRITE_SETS[(id & 0x0F) as usize - 1];
    let coordinate = if split == EAST_WEST { x } else { y };
    Some(if coordinate < line { before } else { after })
}

/// `SpriteSets`: the eleven pictures of a sprite set, one-based.
pub fn sprite_set(id: u8) -> [u8; SPRITE_SET_LENGTH] {
    crate::tables::SPRITE_SETS[id as usize - 1]
}

/// A row of `SpriteSheetPointerTable`: a picture's whole sheet, and how many bytes of it load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteSheet {
    pub tiles: &'static [u8],
    pub bytes: usize,
}

/// `None` for picture 0 and past the table.
pub fn sprite_sheet(picture: u8) -> Option<SpriteSheet> {
    let &(tiles, count) = crate::gfx::SPRITE_SHEETS.get((picture as usize).checked_sub(1)?)?;
    Some(SpriteSheet { tiles, bytes: count * crate::rom_gfx::TILE_BYTES })
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
        let width = crate::map_header::MapHeader::read(Map::PalletTown).unwrap().width as u16;
        assert_eq!(warp.view, 7 + width + (width + 6) * 3 + 2);
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
        // `event_displacement`: `7 + width + (width + 6) * (y / 2) + x / 2`.
        let width = crate::map_header::MapHeader::read(Map::PalletTown).unwrap().width as u16;
        assert_eq!(objects.warp_to[0], WarpTo {
            view: 7 + width + (width + 6) * 2 + 2, y: 5, x: 5,
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
        for map in Map::all().filter(|map| map.has_header()) {
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
        assert_eq!(map_song(Map::PalletTown), (crate::audio::sound::MUSIC_PALLET_TOWN, crate::audio::AUDIO_BANKS[0].rom_bank));
    }
}
