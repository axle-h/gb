use std::collections::HashSet;
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::mmu::MMU;
use crate::pokemon::font::FontAware;
use crate::pokemon::map::Map;
use crate::pokemon::map_header::{MapHeader, MapHeaderReader};
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

    fn read_warp_events(&self) -> Result<Vec<WarpEvent>, String>;

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

    fn read_warp_events(&self) -> Result<Vec<WarpEvent>, String> {
        let warp_count = self.read(0xD3AE) as u16;
        let mut result = vec![];
        let last_map_id = self.read(0xD73C);
        for index in 0..warp_count {
            let address = 0xD3AF + index * 4;
            let map_id = self.read(address + 3);
            let warp = WarpEvent {
                position: Point8 { y: self.read(address), x: self.read(address + 1) },
                // warp_id: self.read(address + 2),
                map_id: Map::from_repr(if map_id == 0xFF {
                    last_map_id
                } else {
                    map_id
                }).ok_or_else(|| format!("Invalid map number {}", map_id))?,
            };
            result.push(warp);
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
        let mut collision_tiles = HashSet::new();
        for index in 0..20 {
            let collision_byte = self.read(collision_address + index);
            if collision_byte == 0xff {
                break;
            }
            collision_tiles.insert(collision_byte);
        }

        let map_header_pointer = map.header_pointer().ok_or_else(|| format!("Map has no header pointer: {}", map))?;
        let map_header = self.read_map_header(map_header_pointer).ok_or_else(|| "Invalid map header".to_string())?;

        let map_data_address = self.read_pointer_u16_le(&pokered_symbols::wCurMapDataPtr);
        let map_data = self.rom_data_from_pointer(map_bank, map_data_address, map_header.height as usize * map_header.width as usize).to_vec();

        let max_block_id = *map_data.iter().max().unwrap() as usize;
        let block_data = self.rom_data_from_pointer(tileset_bank, map_header.blocks_address, (max_block_id + 1) * CurrentMap::BLOCK_TILES).to_vec();

        let warp_events = self.read_warp_events()?;

        let sprites = self.read_sprites()?;
        let player_position = Point8 {
            x: self.read_pointer(&pokered_symbols::wXCoord),
            y: self.read_pointer(&pokered_symbols::wYCoord),
        };

        let player_direction_raw = self.read_pointer(&pokered_symbols::wPlayerDirection);
        let player_direction = PlayerFacingDirection::from_repr(player_direction_raw)
            .ok_or_else(|| format!("Invalid player facing direction {}", player_direction_raw))?;

        Ok(CurrentMap {
            map,
            map_header,
            player_position,
            player_direction,
            map_data,
            block_data,
            collision_tiles,
            warp_events,
            sprites,
        })
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
    Sprite(&'static str),
    Warp(Map),
    // TODO map connection
    // TODO signs
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum PlayerFacingDirection {
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

pub struct CurrentMap {
    pub map: Map,
    pub map_header: MapHeader,
    pub player_position: Point8,
    pub player_direction: PlayerFacingDirection,
    pub map_data: Vec<u8>,
    pub block_data: Vec<u8>,
    pub collision_tiles: HashSet<u8>,
    pub warp_events: Vec<WarpEvent>,
    pub sprites: Vec<Sprite>,
}

impl CurrentMap {
    pub const BLOCK_TILE_WIDTH: usize = 4; // a block is 4x4 tiles
    pub const BLOCK_TILES: usize = Self::BLOCK_TILE_WIDTH * Self::BLOCK_TILE_WIDTH;
    pub const TILES_PER_META: usize = 2; // a meta tile on the map is 2x2 graphical tiles

    pub fn is_empty(&self, tile_x: usize, tile_y: usize) -> bool {
        let block_x = tile_x / Self::BLOCK_TILE_WIDTH;
        let block_y = tile_y / Self::BLOCK_TILE_WIDTH;
        let block_index = self.map_data[block_x + block_y * self.map_header.width as usize] as usize;
        let block_offset = block_index * Self::BLOCK_TILES;
        let tile_offset = (tile_x % Self::BLOCK_TILE_WIDTH) + (tile_y % Self::BLOCK_TILE_WIDTH) * Self::BLOCK_TILE_WIDTH;
        let tile_index = self.block_data[block_offset + tile_offset];
        self.collision_tiles.contains(&tile_index)
    }

    pub fn meta_width(&self) -> usize {
        self.map_header.width as usize * Self::TILES_PER_META
    }

    pub fn meta_height(&self) -> usize {
        self.map_header.height as usize * Self::TILES_PER_META
    }

    pub fn meta_tiles(&self) -> Vec<MetaTile> {
        let width = self.meta_width();
        let height = self.meta_height();

        // start off assuming all tiles are obstacles
        let mut result = vec![MetaTile::Obstacle; width * height];

        let width_tiles = self.map_header.width as usize * Self::BLOCK_TILE_WIDTH;
        let height_tiles = self.map_header.height as usize * Self::BLOCK_TILE_WIDTH;
        for tile_y in 0..height_tiles {
            let y = tile_y / Self::TILES_PER_META;
            for tile_x in 0..width_tiles {
                let x = tile_x / Self::TILES_PER_META;
                let index = x + y * width;
                // if you can walk over any tile in a map position (2x2 tiles) then you can walk over the whole meta tile
                if result[index] == MetaTile::Obstacle && self.is_empty(tile_x, tile_y) {
                    result[index] = MetaTile::Empty;
                }
            }
        }

        // now check for sprites
        for sprite in self.sprites.iter().filter(|sprite| !sprite.hidden) {
            let index = sprite.position.x as usize + sprite.position.y as usize * width;
            result[index] = MetaTile::Sprite(sprite.name);
        }

        // now check for warp events
        for warp_event in &self.warp_events {
            let index = warp_event.position.x as usize + warp_event.position.y as usize * width;
            result[index] = MetaTile::Warp(warp_event.map_id);
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