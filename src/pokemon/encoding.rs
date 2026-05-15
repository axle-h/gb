use std::collections::{HashMap, HashSet};
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::mmu::MMU;
use crate::pokemon::font::FontAware;
use crate::pokemon::map::Map;
use crate::pokemon::map_header::{MapConnectionDirection, MapHeader, MapHeaderReader, TileSetId};
use crate::pokemon::symbols::{DmgBank, DmgPointer, DmgPointerRead};
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::party::PokemonParty;
use crate::pokemon::pokemon::{Pokemon, PokemonStats, PokemonType};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::sprite::{PictureId, Sprite};
use crate::pokemon::symbols::{pokered_symbols};
use crate::ram::{RAM, ROM};

pub trait PokemonEncoding {

    fn read_pokemon_party(&self, base_pointer: &DmgPointer) -> Result<PokemonParty, String>;
    
    fn read_player_pokemon_party(&self) -> Result<PokemonParty, String> {
        self.read_pokemon_party(&pokered_symbols::wPartyDataStart)
    }

    fn read_pokemon(&self, party_base_pointer: &DmgPointer, index: u16) -> Result<Pokemon, String>;

    fn write_pokemon_party(&mut self, base_pointer: &DmgPointer, party: &PokemonParty) -> Result<(), String>;
    
    fn write_player_pokemon_party(&mut self, party: &PokemonParty) -> Result<(), String>{
        self.write_pokemon_party(&pokered_symbols::wPartyDataStart, party)
    }

    fn write_pokemon(&mut self, party_base_pointer: &DmgPointer, index: u16, pokemon: &Pokemon) -> Result<(), String>;

    fn read_sprites(&self) -> Result<Vec<Sprite>, String>;

    fn read_warp_events(&self, cur_map: Map, map_bank: usize, objects_address: u16) -> Result<Vec<WarpEvent>, String>;

    fn read_game_mode(&self) -> GameMode;

    fn read_current_map(&self) -> Result<CurrentMap, String>;
}

impl PokemonEncoding for MMU {

    fn read_pokemon_party(&self, base_pointer: &DmgPointer) -> Result<PokemonParty, String> {
        let count = self.read_pointer(base_pointer);
        let mut party = PokemonParty::default();
        let pokemon_pointer = *base_pointer + 8;
        for i in 0..count {
            let pokemon = self.read_pokemon(&pokemon_pointer, i as u16)?;
            party.push(pokemon)?;
        }
        Ok(party)
    }

    fn read_pokemon(&self, party_base_pointer: &DmgPointer, index: u16) -> Result<Pokemon, String> {
        let addresses = PokemonBlockAddresses::of_indexed(*party_base_pointer, index);

        fn parse_move(pokemon_bytes: &[u8], offset: u16) -> Option<PokemonMove> {
            if let Some(name) = PokemonMoveName::from_repr(pokemon_bytes.read(8 + offset)) {
                Some(
                    PokemonMove {
                        name,
                        pp: pokemon_bytes.read(29 + offset)
                    }
                )
            } else {
                None
            }
        }

        fn read_stats(pokemon_bytes: &[u8], offset: u16) -> PokemonStats {
            PokemonStats {
                hp: pokemon_bytes.read_u16_be(offset),
                attack: pokemon_bytes.read_u16_be(offset + 2),
                defense: pokemon_bytes.read_u16_be(offset + 4),
                speed: pokemon_bytes.read_u16_be(offset + 6),
                special: pokemon_bytes.read_u16_be(offset + 8),
            }
        }

        let pokemon_bytes = self.read_pointer_vec(&addresses.pokemon, PokemonBlockAddresses::POKEMON_BLOCK_SIZE as usize);
        Ok(Pokemon {
            nickname: self.read_pointer_pokemon_string(&addresses.nickname),
            trainer_name: self.read_pointer_pokemon_string(&addresses.trainer_name),
            species: PokemonSpecies::from_repr(pokemon_bytes.read(0)).ok_or_else(|| "Invalid Pokemon species".to_string())?,
            current_hp: pokemon_bytes.read_u16_be(1),
            status: pokemon_bytes.read(4).into(),
            types: [
                PokemonType::from_repr(pokemon_bytes.read(5))
                    .ok_or_else(|| "Invalid Pokemon type".to_string())?,
                PokemonType::from_repr(pokemon_bytes.read(6))
                    .ok_or_else(|| "Invalid Pokemon type".to_string())?,
            ],
            moves: std::array::from_fn(|i| parse_move(&pokemon_bytes, i as u16)),
            trainer_id: pokemon_bytes.read_u16_be(12),
            experience: pokemon_bytes.read_u32_be(13) & 0xFFFFFF, // 3 bytes so read as u32 offset -1 and trim top byte
            effort_values: read_stats(&pokemon_bytes, 17),
            individual_values: PokemonStats::from_iv_bytes(
                pokemon_bytes.read(27),
                pokemon_bytes.read(28)
            ),
            level: pokemon_bytes.read(33),
            stats: read_stats(&pokemon_bytes, 34),
        })
    }

    fn write_pokemon_party(&mut self, base_pointer: &DmgPointer, party: &PokemonParty) -> Result<(), String> {
        self.write_pointer(base_pointer, party.len() as u8)?; // length

        let mut species_pointer = *base_pointer + 1;
        let pokemon_pointer = *base_pointer + 8;

        for (index, pokemon) in party.pokemon().iter().enumerate() {
            self.write_pokemon(&pokemon_pointer, index as u16, pokemon)?;
            self.write_pointer(&species_pointer, pokemon.species as u8)?;
            species_pointer += 1;
        }

        // write list end
        self.write_pointer(&species_pointer, 0xFF)
    }

    fn write_pokemon(&mut self, party_base_pointer: &DmgPointer, index: u16, pokemon: &Pokemon) -> Result<(), String> {
        let addresses = PokemonBlockAddresses::of_indexed(*party_base_pointer, index);

        fn write_move(pokemon_bytes: &mut Vec<u8>, offset: u16, move_: Option<PokemonMove>) {
            if let Some(move_) = move_ {
                pokemon_bytes.write(8 + offset, move_.name as u8);
                pokemon_bytes.write(29 + offset, move_.pp);
            } else {
                pokemon_bytes.write(8 + offset, 0x00);
                pokemon_bytes.write(29 + offset, 0x00);
            }
        }

        fn write_stats(pokemon_bytes: &mut Vec<u8>, offset: u16, stats: PokemonStats) {
            pokemon_bytes.write_u16_be(offset, stats.hp);
            pokemon_bytes.write_u16_be(offset + 2, stats.attack);
            pokemon_bytes.write_u16_be(offset + 4, stats.defense);
            pokemon_bytes.write_u16_be(offset + 6, stats.speed);
            pokemon_bytes.write_u16_be(offset + 8, stats.special);
        }

        self.write_pointer_pokemon_string(&addresses.nickname, &pokemon.nickname)?;
        self.write_pointer_pokemon_string(&addresses.trainer_name, &pokemon.trainer_name)?;

        let mut pokemon_bytes = self.read_pointer_vec(&addresses.pokemon, PokemonBlockAddresses::POKEMON_BLOCK_SIZE as usize);
        pokemon_bytes.write(0, pokemon.species as u8);
        pokemon_bytes.write_u16_be(1, pokemon.current_hp);
        pokemon_bytes.write(4, pokemon.status.into());
        pokemon_bytes.write(5, pokemon.types[0] as u8);
        pokemon_bytes.write(6, pokemon.types[1] as u8);
        for i in 0..4 {
            write_move(&mut pokemon_bytes, i as u16, pokemon.moves[i]);
        }
        pokemon_bytes.write_u32_be(13, pokemon.experience & 0xFFFFFF);
        pokemon_bytes.write_u16_be(12, pokemon.trainer_id);
        write_stats(&mut pokemon_bytes,17, pokemon.effort_values);

        let (attack_defense, speed_special) = pokemon.individual_values.into_iv_bytes();
        pokemon_bytes.write(27, attack_defense);
        pokemon_bytes.write(28, speed_special);
        pokemon_bytes.write(33, pokemon.level);
        write_stats(&mut pokemon_bytes, 34, pokemon.stats);

        self.write_pointer_slice(&addresses.pokemon, &pokemon_bytes)?;
        Ok(())
    }

    fn read_sprites(&self) -> Result<Vec<Sprite>, String> {
        let map = Map::from_repr(self.read(0xD35E)).ok_or_else(|| "Invalid map number".to_string())?;
        let map_sprites = map.sprites();

        let missable_objects = self.read_slice(0xD5A6, 31);

        let mut sprites: Vec<Sprite> = Vec::new();
        for index in 1..=0xFu16 { // do not read index=0 as it is always the player
            let offset = index << 4;
            let picture_id = match PictureId::from_repr(self.read(0xC100 | offset)) {
                Some(picture_id) => picture_id,
                None => continue
            };
            let map_sprite = match map_sprites.get(index as usize - 1) {
                Some(map_sprite) => map_sprite,
                None => continue
            };

            let hidden = match map_sprite.hidden_object_id {
                Some(hidden_object_bit) => {
                    let mask = 1 << hidden_object_bit % 8;
                    (missable_objects[(hidden_object_bit / 8) as usize] & mask) == mask
                }
              None => false,
            };

            let sprite_image_index = self.read(0xC102 | offset);

            let sprite = Sprite {
                index: index as u8,
                picture_id,
                position: if picture_id == PictureId::Red {
                    // Read player position from the map state
                    Point8 {
                        x: self.read(0xD362),
                        y: self.read(0xD361)
                    }
                } else {
                    Point8 {
                        x: self.read(0xC205 | offset) - 4,
                        y: self.read(0xC204 | offset) - 4
                    }
                },
                on_screen: sprite_image_index != 0xFF,
                hidden,
                name: map_sprite.name
            };
            sprites.push(sprite);
        }
        Ok(sprites)
    }

    fn read_warp_events(&self, cur_map: Map, map_bank: usize, objects_address: u16) -> Result<Vec<WarpEvent>, String> {
        // Read directly from ROM so we get the raw destination byte (including 0xFF / self-ref)
        // before pokémon Red's runtime wLastMap resolution, which becomes stale when
        // navigating between indoor floors (e.g. Red's House 1F → 2F → 1F).
        //
        // Object-data layout: [border_block(1), warp_count(1), warp_entries(4 each), ...]
        let warp_count = self.rom_data_from_pointer(map_bank, objects_address + 1, 1)[0] as u16;
        let mut result = vec![];
        for index in 0..warp_count {
            let base = objects_address + 2 + index * 4;
            let entry = self.rom_data_from_pointer(map_bank, base, 4);
            let raw_map_id = entry[3];
            // 0xFF = LAST_MAP sentinel; self-referential (raw_map_id == cur_map) is
            // pokémon Red's building-exit convention meaning the same thing.
            // In both cases the true destination is the outdoor map whose warp table
            // points back to this indoor map.
            let map_id = if raw_map_id == 0xFF || raw_map_id == cur_map as u8 {
                self.find_outdoor_entry_map(cur_map)
                    .ok_or_else(|| format!("No outdoor map found for {cur_map}"))?
            } else {
                Map::from_repr(raw_map_id)
                    .ok_or_else(|| format!("Invalid map number {raw_map_id}"))?
            };
            result.push(WarpEvent {
                position: Point8 { y: entry[0], x: entry[1] },
                map_id,
            });
        }
        Ok(result)
    }

    fn read_game_mode(&self) -> GameMode {
        // ; lost battle, this is -1
        // ; no battle, this is 0
        // ; wild battle, this is 1
        // ; trainer battle, this is 2
        match self.read_pointer(&pokered_symbols::wIsInBattle) {
            1 => GameMode::WildBattle,
            2 => GameMode::TrainerBattle,
            _ => {
                // wFontLoaded infers a text box is open
                // it is set in DisplayTextIDInit and reset in ReloadMapSpriteTilePatterns
                let font_loaded = self.read_pointer(&pokered_symbols::wFontLoaded) & 0x01 == 1;
                if font_loaded {
                    // TODO menu vs dialogue
                    // e.g. the game seems to set the textbox type like this for a message box
                    // 	ld a, MESSAGE_BOX
                    // 	ld [wTextBoxID], a
                    // see TextBoxFunctionTable:
                    GameMode::TextBox
                } else if self.read_pointer(&pokered_symbols::wScriptedNPCWalkCounter) != 0 {
                    // wScriptedNPCWalkCounter is non-zero while an NPC scripted walk is in
                    // progress (e.g. clerk NPC approaching the player in a PokéMART).
                    // The player is frozen; the agent should press A to advance the script.
                    GameMode::Script
                } else {
                    GameMode::Overworld
                }
            }
        }
    }

    fn read_current_map(&self) -> Result<CurrentMap, String> {
        // any rom data we read must be directly from the rom banks as the game is not guaranteed to have the correct bank loaded
        let map = Map::from_repr(self.read_pointer(&pokered_symbols::wCurMap)).ok_or_else(|| "Invalid map number".to_string())?;
        let map_bank = self.rom_data_from_rom_pointer(&pokered_symbols::MapHeaderBanks, Map::COUNT)[map as usize] as usize;
        let tileset_bank = self.read_pointer(&pokered_symbols::wTilesetBank) as usize;

        // collision data is always in bank 0
        let collision_address = self.read_pointer_u16_le(&pokered_symbols::wTilesetCollisionPtr);
        let collision_tiles = self.read_collision_tiles(collision_address);

        let map_header_pointer = map.header_pointer().ok_or_else(|| format!("Map has no header pointer: {}", map))?;
        let map_header = self.read_map_header(map_header_pointer).ok_or_else(|| "Invalid map header".to_string())?;

        let map_data_address = self.read_pointer_u16_le(&pokered_symbols::wCurMapDataPtr);
        let map_data = self.rom_data_from_pointer(map_bank, map_data_address, map_header.height as usize * map_header.width as usize).to_vec();

        let max_block_id = *map_data.iter().max().unwrap() as usize;
        let tileset_data_address = self.read_pointer_u16_le(&pokered_symbols::wTilesetBlocksPtr);
        let tileset_data = self.rom_data_from_pointer(tileset_bank, tileset_data_address, (max_block_id + 1) * CurrentMap::BLOCK_TILES).to_vec();

        let warp_events = self.read_warp_events(map, map_bank, map_header.objects_address)?;

        let sprites = self.read_sprites()?;

        // Check if the current tileset is in the WaterTilesets table (FF-terminated ROM list).
        // Mirrors the `IsNextTileShoreOrWater` logic in item_effects.asm.
        let tileset_id = map_header.tileset as u8;
        let water_tilesets = self.rom_data_from_rom_pointer(&pokered_symbols::WaterTilesets, 16);
        let is_water_tileset = water_tilesets.iter()
            .take_while(|&&b| b != 0xFF)
            .any(|&b| b == tileset_id);

        // wTilesetTalkingOverTiles: 3 inline bytes in RAM holding tile IDs for counter/desk
        // tiles that the player can interact through (pokered "talking over" mechanic).
        // 0xFF = unused slot (no tile).
        let talking_over_tiles: HashSet<u8> = {
            let base = pokered_symbols::wTilesetTalkingOverTiles.address;
            (0u16..3).map(|i| self.read(base + i))
                     .filter(|&b| b != 0xFF)
                     .collect()
        };
        let player_position = Point8 {
            x: self.read_pointer(&pokered_symbols::wXCoord),
            y: self.read_pointer(&pokered_symbols::wYCoord),
        };

        let player_direction_raw = self.read_pointer(&pokered_symbols::wPlayerDirection);
        let player_direction = PlayerFacingDirection::from_repr(player_direction_raw)
            .ok_or_else(|| format!("Invalid player facing direction {}", player_direction_raw))?;

        let connected_strips = self.load_connected_strips(&map_header);

        // HandleLedges in pokered only fires for tileset 0 (Overworld); other tilesets have no ledges.
        let ledge_tiles = if map_header.tileset == TileSetId::Overworld {
            self.read_ledge_tiles()
        } else {
            HashMap::new()
        };

        Ok(CurrentMap {
            map,
            map_header,
            player_position,
            player_direction,
            map_data,
            tileset_data,
            collision_tiles,
            talking_over_tiles,
            warp_events,
            sprites,
            is_water_tileset,
            connected_strips,
            ledge_tiles,
        })
    }
}

impl MMU {
    /// Scans every Overworld-tileset map in ROM for a warp tile whose destination map
    /// equals `indoor_map`.  Returns the first match — this is the outdoor map the
    /// player should be returned to when they step on a self-referential / LAST_MAP exit warp.
    fn find_outdoor_entry_map(&self, indoor_map: Map) -> Option<Map> {
        let map_banks = self.rom_data_from_rom_pointer(&pokered_symbols::MapHeaderBanks, Map::COUNT);
        (0..Map::COUNT)
            .filter_map(|id| {
                let outdoor_map = Map::from_repr(id as u8)?;
                let header_ptr  = outdoor_map.header_pointer()?;
                let header      = self.read_map_header(header_ptr)?;
                if header.tileset != TileSetId::Overworld { return None; }
                let bank        = map_banks[id] as usize;
                let warp_count  = self.rom_data_from_pointer(bank, header.objects_address + 1, 1)[0] as u16;
                for wi in 0..warp_count {
                    let entry = self.rom_data_from_pointer(bank, header.objects_address + 2 + wi * 4, 4);
                    if entry[3] == indoor_map as u8 {
                        return Some(outdoor_map);
                    }
                }
                None
            })
            .next()
    }

    /// Reads the `LedgeTiles` ROM table and returns a map of tile_id → JumpDirection.
    ///
    /// The table has 4-byte entries terminated by 0xFF:
    ///   [facing_direction, tile_under_player, tile_in_front, jump_direction_flags]
    /// `tile_in_front` (byte 2) is the ledge tile ID; `jump_direction_flags` (byte 3) encodes
    /// the direction: 0x80=south, 0x20=west, 0x10=east (same bit layout as wPlayerMovingDirection).
    fn read_ledge_tiles(&self) -> HashMap<u8, JumpDirection> {
        let data = self.rom_data_from_rom_pointer(&pokered_symbols::LedgeTiles, 64);
        let mut result = HashMap::new();
        let mut i = 0;
        while i + 3 < data.len() {
            if data[i] == 0xFF { break; }
            let tile_in_front   = data[i + 2];
            let dir_flags       = data[i + 3];
            let dir = match dir_flags {
                0x80 => JumpDirection::South,
                0x20 => JumpDirection::West,
                0x10 => JumpDirection::East,
                _    => { i += 4; continue; }
            };
            result.insert(tile_in_front, dir);
            i += 4;
        }
        result
    }

    /// Reads an FF-terminated list of walkable tile IDs from a bank-0 ROM address.
    fn read_collision_tiles(&self, ptr: u16) -> HashSet<u8> {
        let mut tiles = HashSet::new();
        for index in 0..20u16 {
            let byte = self.read(ptr + index);
            if byte == 0xFF { break; }
            tiles.insert(byte);
        }
        tiles
    }

    fn load_connected_strips(&self, map_header: &MapHeader) -> Vec<ConnectedMapStrip> {
        // Each entry in the Tilesets ROM table is 12 bytes:
        //   [0]     bank
        //   [1..2]  blocks_ptr (LE)
        //   [3..4]  gfx_ptr    (LE, unused here)
        //   [5..6]  coll_ptr   (LE)
        //   [7..]   remaining fields (unused here)
        // 24 tilesets × 12 bytes = 288 bytes matches the gap to the next symbol.
        const TILESET_ENTRY_SIZE: u16 = 12;

        let all_map_banks: Vec<u8> = self
            .rom_data_from_rom_pointer(&pokered_symbols::MapHeaderBanks, Map::COUNT)
            .to_vec();
        let water_tilesets: Vec<u8> = self
            .rom_data_from_rom_pointer(&pokered_symbols::WaterTilesets, 16)
            .to_vec();

        map_header.connections()
            .into_iter()
            .filter_map(|connection| {
                let connected_map_bank = all_map_banks[connection.map as usize] as usize;
                let connected_header = self.read_map_header(connection.map.header_pointer()?)?;

                // Read the single block row/column that borders the current map.
                let (border_blocks, block_sub_offset): (Vec<u8>, u8) = match connection.direction {
                    MapConnectionDirection::South => {
                        // First block row of the connected map.
                        let blocks = self.rom_data_from_pointer(
                            connected_map_bank,
                            connection.strip_src,
                            connection.strip_length as usize,
                        ).to_vec();
                        (blocks, 0)
                    }
                    MapConnectionDirection::North => {
                        // strip_src points to the start of the 3-block-deep strip
                        // (connected_height − 3 rows in). The border row is the 3rd row.
                        let addr = connection.strip_src + 2 * connection.strip_length as u16;
                        let blocks = self.rom_data_from_pointer(
                            connected_map_bank,
                            addr,
                            connection.strip_length as usize,
                        ).to_vec();
                        (blocks, 1)
                    }
                    MapConnectionDirection::East => {
                        // strip_src points to column 0 of each row; stride by connected_map_width.
                        let blocks = (0..connection.strip_length as u16)
                            .map(|row| {
                                self.rom_data_from_pointer(
                                    connected_map_bank,
                                    connection.strip_src + row * connection.connected_map_width as u16,
                                    1,
                                )[0]
                            })
                            .collect();
                        (blocks, 0)
                    }
                    MapConnectionDirection::West => {
                        // strip_src points to column (width−3); border column is +2 (column width−1).
                        let blocks = (0..connection.strip_length as u16)
                            .map(|row| {
                                self.rom_data_from_pointer(
                                    connected_map_bank,
                                    connection.strip_src + row * connection.connected_map_width as u16 + 2,
                                    1,
                                )[0]
                            })
                            .collect();
                        (blocks, 1)
                    }
                };

                if border_blocks.is_empty() {
                    return None;
                }
                let max_block_id = *border_blocks.iter().max().unwrap() as usize;

                let tileset = connected_header.tileset;
                let entry   = pokered_symbols::Tilesets + tileset as u16 * TILESET_ENTRY_SIZE;
                let tileset_bank       = self.read_pointer(&entry) as usize;
                let tileset_blocks_ptr = self.read_pointer_u16_le(&(entry + 1));
                let tileset_coll_ptr   = self.read_pointer_u16_le(&(entry + 5));

                let tileset_data = self.rom_data_from_pointer(
                    tileset_bank,
                    tileset_blocks_ptr,
                    (max_block_id + 1) * CurrentMap::BLOCK_TILES,
                ).to_vec();

                let collision_tiles = self.read_collision_tiles(tileset_coll_ptr);

                let tileset_id_byte = tileset as u8;
                let is_water_tileset = water_tilesets.iter()
                    .take_while(|&&b| b != 0xFF)
                    .any(|&b| b == tileset_id_byte);

                let meta_align_offset = match connection.direction {
                    MapConnectionDirection::North | MapConnectionDirection::South =>
                        (-(connection.x_alignment as i32)).max(0) as usize,
                    MapConnectionDirection::East | MapConnectionDirection::West =>
                        (-(connection.y_alignment as i32)).max(0) as usize,
                };

                Some(ConnectedMapStrip {
                    direction: connection.direction,
                    map: connection.map,
                    border_blocks,
                    tileset_data,
                    collision_tiles,
                    is_water_tileset,
                    tileset,
                    block_sub_offset,
                    strip_length: connection.strip_length,
                    meta_align_offset,
                })
            })
            .collect()
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, strum_macros::Display, Default)]
pub enum GameMode {
    #[default]
    Overworld,
    #[strum(serialize = "Wild Pokemon Battle")]
    WildBattle,
    #[strum(serialize = "Trainer Battle")]
    TrainerBattle,
    Menu,
    TextBox,
    /// A map script or NPC scripted walk is running (`wCurrentMapScriptFlags` is non-zero).
    /// The player is frozen; the agent should advance the script by pressing A.
    Script,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct WarpEvent {
    pub position: Point8,
    pub map_id: Map,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum_macros::Display, Default)]
pub enum MetaTile {
    #[default]
    Empty,
    Obstacle,
    /// Water tile (tile ID 0x14) — can only be crossed while surfing.
    Water,
    /// Ledge tile — can only be crossed by jumping in the specified direction.
    Jump(JumpDirection),
    Sprite(&'static str),
    Warp(Map),
    /// Walkable entry point into an adjacent map.
    Connection(Map),
    /// Water entry point into an adjacent map — only reachable while surfing.
    ConnectionWater(Map),
    /// Counter / desk tile listed in `wTilesetTalkingOverTiles`.
    /// The player cannot walk on it, but can interact with a sprite one tile behind it
    /// by facing the counter and pressing A — pokered's "talking over" mechanic.
    Counter,
    /// A shrub that blocks passage until the player uses HM Cut.
    /// Treated as impassable until `can_use_cut` is true.
    CutTree,
}

/// The direction a ledge can be jumped over.
#[derive(Clone, Copy, Debug, Eq, PartialEq, strum_macros::Display)]
pub enum JumpDirection {
    South,
    West,
    East,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum PlayerFacingDirection {
    #[default]
    Up = 8,
    Down = 4,
    Left = 2,
    Right = 1,
}

impl Into<JoypadButton> for PlayerFacingDirection {
    fn into(self) -> JoypadButton {
        match self {
            PlayerFacingDirection::Up => JoypadButton::Up,
            PlayerFacingDirection::Down => JoypadButton::Down,
            PlayerFacingDirection::Left => JoypadButton::Left,
            PlayerFacingDirection::Right => JoypadButton::Right,
        }
    }
}

/// Returns true if `tile_id` is a water or shore tile for the given tileset.
///
/// Mirrors `IsNextTileShoreOrWater` from `engine/items/item_effects.asm`:
/// - Tile `$14` is the universal water tile.
/// - Tiles `$32` (eastern shore) and `$48` (Safari Zone shore) are shore tiles,
///   **unless** the tileset is `ShipPort` (Vermilion Dock), where `$32` is dock planks.
/// - Returns false for any tileset not listed in the `WaterTilesets` ROM table.
fn is_water_tile_id(tile_id: u8, is_water_tileset: bool, tileset: TileSetId) -> bool {
    const WATER: u8 = 0x14;
    const EASTERN_SHORE: u8 = 0x32;
    const SAFARI_ZONE_EASTERN_SHORE: u8 = 0x48;
    if !is_water_tileset {
        return false;
    }
    if tileset != TileSetId::ShipPort && (tile_id == EASTERN_SHORE || tile_id == SAFARI_ZONE_EASTERN_SHORE) {
        return true;
    }
    tile_id == WATER
}

/// Loaded border strip from an adjacent connected map, used to expand the rendered tilemap
/// by one meta-tile row (N/S connections) or column (E/W connections).
struct ConnectedMapStrip {
    direction: MapConnectionDirection,
    map: Map,
    /// Block IDs along the single-block-deep border row/column of the connected map.
    border_blocks: Vec<u8>,
    tileset_data: Vec<u8>,
    collision_tiles: HashSet<u8>,
    is_water_tileset: bool,
    tileset: TileSetId,
    /// Sub-position within each block:
    /// N/S: 0 = top meta-row (south connection), 1 = bottom meta-row (north connection).
    /// E/W: 0 = left meta-col (east connection),  1 = right meta-col (west connection).
    block_sub_offset: u8,
    strip_length: u8,
    /// Offset (in meta-tiles) along the perpendicular axis where the strip begins.
    /// x-offset for N/S connections, y-offset for E/W connections.
    meta_align_offset: usize,
}

impl ConnectedMapStrip {
    /// Returns the MetaTile for position `strip_idx` (0..strip_length*2) in this border strip.
    fn meta_tile_at(&self, strip_idx: usize) -> MetaTile {
        let block_idx = strip_idx / 2;
        if block_idx >= self.border_blocks.len() {
            return MetaTile::Obstacle;
        }
        let block_id = self.border_blocks[block_idx];

        // For N/S: strip_idx selects left(0)/right(1) within each block; sub_offset is the row.
        // For E/W: strip_idx selects top(0)/bottom(1) within each block; sub_offset is the col.
        let (sub_col, sub_row) = match self.direction {
            MapConnectionDirection::North | MapConnectionDirection::South =>
                (strip_idx % 2, self.block_sub_offset as usize),
            MapConnectionDirection::East | MapConnectionDirection::West =>
                (self.block_sub_offset as usize, strip_idx % 2),
        };

        // Indices of the four graphical tiles that form this 2×2 meta-tile within the block.
        // tile_offset = tx_in_block + ty_in_block * BLOCK_TILE_WIDTH (4)
        let base = sub_col * 2 + sub_row * 8;
        let tile_indices = [base, base + 1, base + 4, base + 5];
        let block_start = block_id as usize * CurrentMap::BLOCK_TILES;

        let mut has_water = false;
        let mut has_walkable = false;
        for &idx in &tile_indices {
            let pos = block_start + idx;
            if pos >= self.tileset_data.len() {
                continue;
            }
            let tile_id = self.tileset_data[pos];
            if self.is_water_tile(tile_id) {
                has_water = true;
            }
            if self.collision_tiles.contains(&tile_id) {
                has_walkable = true;
            }
        }

        if has_water {
            MetaTile::ConnectionWater(self.map)
        } else if has_walkable {
            MetaTile::Connection(self.map)
        } else {
            MetaTile::Obstacle
        }
    }

    fn is_water_tile(&self, tile_id: u8) -> bool {
        is_water_tile_id(tile_id, self.is_water_tileset, self.tileset)
    }
}

pub struct CurrentMap {
    pub map: Map,
    pub map_header: MapHeader,
    pub player_position: Point8,
    pub player_direction: PlayerFacingDirection,
    pub map_data: Vec<u8>,
    pub tileset_data: Vec<u8>,
    pub collision_tiles: HashSet<u8>,
    /// Tile IDs from `wTilesetTalkingOverTiles` — counters/desks the player can
    /// interact through by facing them and pressing A (pokered "talking over" mechanic).
    pub talking_over_tiles: HashSet<u8>,
    pub warp_events: Vec<WarpEvent>,
    pub sprites: Vec<Sprite>,
    /// True if the current tileset is listed in the WaterTilesets table,
    /// meaning tile $14/$32/$48 should be treated as water/shore.
    pub is_water_tileset: bool,
    connected_strips: Vec<ConnectedMapStrip>,
    /// Maps ledge tile IDs to their jump direction. Only populated for the Overworld tileset,
    /// which is the only tileset where HandleLedges fires.
    ledge_tiles: HashMap<u8, JumpDirection>,
}

impl CurrentMap {
    pub const BLOCK_TILE_WIDTH: usize = 4; // a block is 4x4 tiles
    pub const BLOCK_TILES: usize = Self::BLOCK_TILE_WIDTH * Self::BLOCK_TILE_WIDTH;
    pub const TILES_PER_META: usize = 2; // a meta tile on the map is 2x2 graphical tiles

    pub fn tile_id(&self, tile_x: usize, tile_y: usize) -> u8 {
        let block_x = tile_x / Self::BLOCK_TILE_WIDTH;
        let block_y = tile_y / Self::BLOCK_TILE_WIDTH;
        let block_index = self.map_data[block_x + block_y * self.map_header.width as usize] as usize;
        let block_offset = block_index * Self::BLOCK_TILES;
        let tile_offset = (tile_x % Self::BLOCK_TILE_WIDTH) + (tile_y % Self::BLOCK_TILE_WIDTH) * Self::BLOCK_TILE_WIDTH;
        self.tileset_data[block_offset + tile_offset]
    }

    pub fn is_empty(&self, tile_x: usize, tile_y: usize) -> bool {
        self.collision_tiles.contains(&self.tile_id(tile_x, tile_y))
    }

    /// Returns true if this sub-tile is a water or shore tile.
    ///
    /// Mirrors `IsNextTileShoreOrWater` from `engine/items/item_effects.asm`.
    pub fn is_water(&self, tile_x: usize, tile_y: usize) -> bool {
        is_water_tile_id(self.tile_id(tile_x, tile_y), self.is_water_tileset, self.map_header.tileset)
    }

    pub fn meta_width(&self) -> usize {
        self.map_header.width as usize * Self::TILES_PER_META
    }

    pub fn meta_height(&self) -> usize {
        self.map_header.height as usize * Self::TILES_PER_META
    }

    /// Extra meta-tile rows/columns added for each connected direction.
    pub fn north_extra(&self) -> usize { self.map_header.north_connection.is_some() as usize }
    pub fn south_extra(&self) -> usize { self.map_header.south_connection.is_some() as usize }
    pub fn east_extra(&self)  -> usize { self.map_header.east_connection.is_some()  as usize }
    pub fn west_extra(&self)  -> usize { self.map_header.west_connection.is_some()  as usize }

    pub fn meta_tiles(&self) -> Vec<MetaTile> {
        let base_width  = self.meta_width();
        let base_height = self.meta_height();

        // Connection strips expand the map by one meta-tile row/column per direction.
        let x_off      = self.west_extra();
        let y_off      = self.north_extra();
        let exp_width  = base_width  + x_off + self.east_extra();
        let exp_height = base_height + y_off + self.south_extra();

        let mut result = vec![MetaTile::Obstacle; exp_width * exp_height];

        // Fill current map tiles, shifted by the connection offsets.
        // Priority: Water > Empty > Counter > Obstacle (default).
        // Water beats walkable: if any sub-tile of a 2×2 meta-tile is water/shore, the whole
        // meta-tile is Water, even if other sub-tiles are walkable (e.g. north-shore + water).
        // Counter: non-walkable tile listed in wTilesetTalkingOverTiles; marks desks/counters
        // that sprites can be talked to through.
        let width_tiles  = self.map_header.width  as usize * Self::BLOCK_TILE_WIDTH;
        let height_tiles = self.map_header.height as usize * Self::BLOCK_TILE_WIDTH;
        for tile_y in 0..height_tiles {
            let my = tile_y / Self::TILES_PER_META + y_off;
            for tile_x in 0..width_tiles {
                let mx    = tile_x / Self::TILES_PER_META + x_off;
                let index = mx + my * exp_width;
                if result[index] != MetaTile::Water {
                    let tile_id = self.tile_id(tile_x, tile_y);
                    if self.is_water(tile_x, tile_y) {
                        result[index] = MetaTile::Water;
                    } else if self.map_header.tileset.cut_tree_tile_id() == Some(tile_id) {
                        result[index] = MetaTile::CutTree;
                    } else if let Some(&dir) = self.ledge_tiles.get(&tile_id) {
                        // Ledge tiles are detected by tile ID alone (not walkability).
                        // The bottom graphical row of a ledge block is non-walkable (not in
                        // collision_tiles) but must still override the walkable rows above it.
                        result[index] = MetaTile::Jump(dir);
                    } else if result[index] == MetaTile::Obstacle && self.is_empty(tile_x, tile_y) {
                        result[index] = MetaTile::Empty;
                    } else if result[index] == MetaTile::Obstacle && self.talking_over_tiles.contains(&tile_id) {
                        result[index] = MetaTile::Counter;
                    }
                }
            }
        }

        for sprite in self.sprites.iter().filter(|s| !s.hidden) {
            let mx = sprite.position.x as usize + x_off;
            let my = sprite.position.y as usize + y_off;
            if mx < exp_width && my < exp_height {
                result[mx + my * exp_width] = MetaTile::Sprite(sprite.name);
            }
        }

        for warp in &self.warp_events {
            let mx = warp.position.x as usize + x_off;
            let my = warp.position.y as usize + y_off;
            if mx < exp_width && my < exp_height {
                result[mx + my * exp_width] = MetaTile::Warp(warp.map_id);
            }
        }

        // Fill the extra border rows/columns from connected map strips.
        for strip in &self.connected_strips {
            let strip_meta = strip.strip_length as usize * Self::TILES_PER_META;
            match strip.direction {
                MapConnectionDirection::North | MapConnectionDirection::South => {
                    let x_start = strip.meta_align_offset + x_off;
                    let row = if strip.direction == MapConnectionDirection::North { 0 } else { y_off + base_height };
                    for i in 0..strip_meta {
                        let mx = x_start + i;
                        if mx >= exp_width { break; }
                        result[mx + row * exp_width] = strip.meta_tile_at(i);
                    }
                }
                MapConnectionDirection::West | MapConnectionDirection::East => {
                    let y_start = strip.meta_align_offset + y_off;
                    let col = if strip.direction == MapConnectionDirection::West { 0 } else { x_off + base_width };
                    for i in 0..strip_meta {
                        let my = y_start + i;
                        if my >= exp_height { break; }
                        result[col + my * exp_width] = strip.meta_tile_at(i);
                    }
                }
            }
        }

        result
    }
}

pub struct PokemonBlockAddresses {
    pub pokemon: DmgPointer,
    pub trainer_name: DmgPointer,
    pub nickname: DmgPointer,
}

impl PokemonBlockAddresses {
    pub const PARTY_MAX: u16 = 6;
    pub const POKEMON_BLOCK_SIZE: u16 = 0x2C;
    pub const NAME_LENGTH: u16 = 0xB;

    fn of_indexed(party_base_pointer: DmgPointer, index: u16) -> Self {
        Self {
            pokemon: party_base_pointer + index * Self::POKEMON_BLOCK_SIZE,
            trainer_name: party_base_pointer + Self::PARTY_MAX * Self::POKEMON_BLOCK_SIZE + index * Self::NAME_LENGTH,
            nickname: party_base_pointer + Self::PARTY_MAX * Self::POKEMON_BLOCK_SIZE + Self::PARTY_MAX * Self::NAME_LENGTH + index * Self::NAME_LENGTH,
        }
    }
}

pub fn reverse_bcd(mut value: u32) -> u32 {
    let mut result = 0u32;
    let mut multiplier = 1u32;
    while value > 0 {
        let digit = value & 0xF;
        result += digit * multiplier;
        multiplier *= 10;
        value >>= 4;
    }
    result
}

#[cfg(test)]
mod tests {
    use crate::roms::blargg_cpu::ROM;
    use crate::pokemon::*;
    use crate::pokemon::encoding::reverse_bcd;

    #[test]
    fn test_reverse_bcd() {
        assert_eq!(reverse_bcd(0x3000), 3000);
        assert_eq!(reverse_bcd(0x1234), 1234);
        assert_eq!(reverse_bcd(0x0000), 0);
        assert_eq!(reverse_bcd(0x9999), 9999);
        assert_eq!(reverse_bcd(0x0001), 1);
        assert_eq!(reverse_bcd(0x0012), 12);
        assert_eq!(reverse_bcd(0x0100), 100);
    }

    #[test]
    fn test_full_pokemon_encoding() -> Result<(), String> {
        let mut mmu = MMU::from_rom(ROM)?;

        let mut party = PokemonParty::default();
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Charizard,
                "CHARIZARD",
                [
                    PokemonMoveName::Flamethrower,
                    PokemonMoveName::FireBlast,
                    PokemonMoveName::Fly,
                    PokemonMoveName::Slash,
                ],
                "TRAINER1",
                11111,
            )
        )?;
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Mewtwo,
                "MEWTWO",
                [
                    PokemonMoveName::Psychic,
                    PokemonMoveName::IceBeam,
                    PokemonMoveName::Thunderbolt,
                    PokemonMoveName::Recover,
                ],
                "TRAINER2",
                22222,
            )
        )?;
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Snorlax,
                "SNORLAX",
                [
                    PokemonMoveName::BodySlam,
                    PokemonMoveName::Rest,
                    PokemonMoveName::Bite,
                    PokemonMoveName::Earthquake,
                ],
                "TRAINER3",
                33333,
            )
        )?;
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Gyarados,
                "GYARADOS",
                [
                    PokemonMoveName::HydroPump,
                    PokemonMoveName::DragonRage,
                    PokemonMoveName::Bite,
                    PokemonMoveName::Surf,
                ],
                "TRAINER4",
                44444,
            )
        )?;
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Alakazam,
                "ALAKAZAM",
                [
                    PokemonMoveName::Psychic,
                    PokemonMoveName::Recover,
                    PokemonMoveName::Psybeam,
                    PokemonMoveName::Reflect,
                ],
                "TRAINER5",
                55555,
            )
        )?;
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Dragonite,
                "DRAGONITE",
                [
                    PokemonMoveName::HyperBeam,
                    PokemonMoveName::Fly,
                    PokemonMoveName::Thunderbolt,
                    PokemonMoveName::Surf,
                ],
                "TRAINER6",
                65535,
            )
        )?;

        mmu.write_player_pokemon_party(&party)?;

        let result = mmu.read_player_pokemon_party()?;

        assert_eq!(party, result);
        Ok(())
    }

    #[test]
    fn test_partial_pokemon_encoding() -> Result<(), String> {
        let mut mmu = MMU::from_rom(ROM)?;

        let mut party = PokemonParty::default();
        party.push(
            Pokemon::maxed(
                PokemonSpecies::Charizard,
                "CHARIZARD",
                [
                    PokemonMoveName::Flamethrower,
                    PokemonMoveName::FireBlast,
                    PokemonMoveName::Fly,
                    PokemonMoveName::Slash,
                ],
                "TRAINER1",
                11111,
            )
        )?;

        mmu.write_player_pokemon_party(&party)?;

        let result = mmu.read_player_pokemon_party()?;

        assert_eq!(party, result);
        Ok(())
    }
}