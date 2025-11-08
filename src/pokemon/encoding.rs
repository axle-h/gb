use std::collections::HashSet;
use unicode_segmentation::UnicodeSegmentation;
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::mmu::MMU;
use crate::pokemon::map::Map;
use crate::pokemon::memory_map::PokemonMemoryMap;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::party::PokemonParty;
use crate::pokemon::pokemon::{Pokemon, PokemonStats, PokemonType};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::sprite::{PictureId, Sprite};
use crate::pokemon::strings::PokemonString;

pub trait PokemonEncoding {
    fn read_pokemon_string(&self, address: u16) -> PokemonString;

    fn write_pokemon_string(&mut self, address: u16, string: &PokemonString);

    fn read_pokemon_party(&self, base_address: u16) -> Result<PokemonParty, String>;
    
    fn read_player_pokemon_party(&self) -> Result<PokemonParty, String> {
        self.read_pokemon_party(PokemonMemoryMap::address("wPartyDataStart"))
    }

    fn read_pokemon(&self, base_address: u16, index: u16) -> Result<Pokemon, String>;

    fn write_pokemon_party(&mut self, base_address: u16, party: PokemonParty);
    
    fn write_player_pokemon_party(&mut self, party: PokemonParty) {
        self.write_pokemon_party(PokemonMemoryMap::address("wPartyDataStart"), party);
    }

    fn write_pokemon(&mut self, base_address: u16, index: u16, pokemon: &Pokemon);

    fn read_sprites(&self) -> Result<Vec<Sprite>, String>;

    fn read_warp_events(&self) -> Result<Vec<WarpEvent>, String>;

    fn read_game_mode(&self) -> GameMode;

    fn read_current_map(&self) -> Result<CurrentMap, String>;
}

impl PokemonEncoding for MMU {
    fn read_pokemon_string(&self, address: u16) -> PokemonString {
        let mut bytes = vec![];
        for i in 0..u16::MAX {
            let byte = self.read(address + i);
            bytes.push(byte);
            if byte == PokemonString::TERMINATOR {
                break;
            }
        }
        PokemonString(bytes)
    }

    fn write_pokemon_string(&mut self, address: u16, string: &PokemonString) {
        for (index, byte) in string.0.iter().enumerate() {
            self.write(address + index as u16, *byte);
        }
    }

    fn read_pokemon_party(&self, base_address: u16) -> Result<PokemonParty, String> {
        let count = self.read(base_address);
        let mut party = PokemonParty::default();
        for i in 0..count {
            let pokemon = self.read_pokemon(base_address + 8, i as u16)?;
            party.push(pokemon)?;
        }
        Ok(party)
    }

    fn read_pokemon(&self, base_address: u16, index: u16) -> Result<Pokemon, String> {
        let addresses = PokemonBlockAddresses::of_indexed(base_address, index);

        fn parse_type(mmu: &MMU, pkmn_base: u16, offset: u16) -> Result<PokemonType, String> {
            PokemonType::from_repr(mmu.read(pkmn_base + 5 + offset))
                .ok_or_else(|| format!("Invalid Pokemon type {}", offset + 1))
        }

        fn parse_move(mmu: &MMU, pkmn_base: u16, offset: u16) -> Option<PokemonMove> {
            if let Some(name) = PokemonMoveName::from_repr(mmu.read(pkmn_base + 8 + offset)) {
                Some(
                    PokemonMove {
                        name,
                        pp: mmu.read(pkmn_base + 29 + offset)
                    }
                )
            } else {
                None
            }
        }

        fn read_stats(mmu: &MMU, pkmn_base: u16, offset: u16) -> PokemonStats {
            PokemonStats {
                hp: mmu.read_u16_be(pkmn_base + offset),
                attack: mmu.read_u16_be(pkmn_base + offset + 2),
                defense: mmu.read_u16_be(pkmn_base + offset + 4),
                speed: mmu.read_u16_be(pkmn_base + offset + 6),
                special: mmu.read_u16_be(pkmn_base + offset + 8),
            }
        }

        Ok(Pokemon {
            nickname: self.read_pokemon_string(addresses.nickname),
            trainer_name: self.read_pokemon_string(addresses.trainer_name),
            species: PokemonSpecies::from_repr(self.read(addresses.pokemon)).ok_or_else(|| "Invalid Pokemon species".to_string())?,
            current_hp: self.read_u16_be(addresses.pokemon + 1),
            status: self.read(addresses.pokemon + 4).into(),
            types: [
                parse_type(self, addresses.pokemon, 0)?,
                parse_type(self, addresses.pokemon, 1)?,
            ],
            moves: std::array::from_fn(|i| parse_move(self, addresses.pokemon, i as u16)),
            trainer_id: self.read_u16_be(addresses.pokemon + 12),
            experience: self.read_u32_be(addresses.pokemon + 13) & 0xFFFFFF, // 3 bytes so read as u32 offset -1 and trim top byte
            effort_values: read_stats(self, addresses.pokemon, 17),
            individual_values: PokemonStats::from_iv_bytes(
                self.read(addresses.pokemon + 27),
                self.read(addresses.pokemon + 28)
            ),
            level: self.read(addresses.pokemon + 33),
            stats: read_stats(self, addresses.pokemon, 34),
        })
    }

    fn write_pokemon_party(&mut self, base_address: u16, party: PokemonParty) {
        self.write(base_address, party.len() as u8); // length
        self.write(base_address + 1 + party.len() as u16, 0xFF); // list end
        for (index, pokemon) in party.into_iter().enumerate() {
            self.write_pokemon(base_address + 8, index as u16, &pokemon);
            self.write(base_address + 1 + index as u16, pokemon.species as u8);
        }
    }

    fn write_pokemon(&mut self, base_address: u16, index: u16, pokemon: &Pokemon) {
        let addresses = PokemonBlockAddresses::of_indexed(base_address, index);

        fn write_move(mmu: &mut MMU, pkmn_base: u16, offset: u16, move_: Option<PokemonMove>) {
            if let Some(move_) = move_ {
                mmu.write(pkmn_base + 8 + offset, move_.name as u8);
                mmu.write(pkmn_base + 29 + offset, move_.pp);
            } else {
                mmu.write(pkmn_base + 8 + offset, 0x00);
                mmu.write(pkmn_base + 29 + offset, 0x00);
            }
        }

        fn write_stats(mmu: &mut MMU, pkmn_base: u16, offset: u16, stats: PokemonStats) {
            mmu.write_u16_be(pkmn_base + offset, stats.hp);
            mmu.write_u16_be(pkmn_base + offset + 2, stats.attack);
            mmu.write_u16_be(pkmn_base + offset + 4, stats.defense);
            mmu.write_u16_be(pkmn_base + offset + 6, stats.speed);
            mmu.write_u16_be(pkmn_base + offset + 8, stats.special);
        }

        self.write_pokemon_string(addresses.nickname, &pokemon.nickname);
        self.write_pokemon_string(addresses.trainer_name, &pokemon.trainer_name);
        self.write(addresses.pokemon, pokemon.species as u8);
        self.write_u16_be(addresses.pokemon + 1, pokemon.current_hp);
        self.write(addresses.pokemon + 4, pokemon.status.into());
        self.write(addresses.pokemon + 5, pokemon.types[0] as u8);
        self.write(addresses.pokemon + 6, pokemon.types[1] as u8);
        for i in 0..4 {
            write_move(self, addresses.pokemon, i as u16, pokemon.moves[i]);
        }
        self.write_u32_be(addresses.pokemon + 13, pokemon.experience & 0xFFFFFF);
        self.write_u16_be(addresses.pokemon + 12, pokemon.trainer_id);
        write_stats(self, addresses.pokemon, 17, pokemon.effort_values);

        let (attack_defense, speed_special) = pokemon.individual_values.into_iv_bytes();
        self.write(addresses.pokemon + 27, attack_defense);
        self.write(addresses.pokemon + 28, speed_special);
        self.write(addresses.pokemon + 33, pokemon.level);
        write_stats(self, addresses.pokemon, 34, pokemon.stats);
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
        match self.read(0xD057) {
            1 => GameMode::WildBattle,
            2 => GameMode::TrainerBattle,
            _ => {
                // wFontLoaded infers a text box is open
                // it is set in DisplayTextIDInit and reset in ReloadMapSpriteTilePatterns
                let font_loaded = self.read(0xcfc4) & 0x01 == 1;
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
        let map = Map::from_repr(self.read(0xD35E)).ok_or_else(|| "Invalid map number".to_string())?;
        let map_bank = self.rom_data(3, 0x023D, Map::COUNT)[map as usize] as usize;
        let map_height_blocks = self.read(0xD368) as usize;
        let map_width_blocks = self.read(0xD369) as usize;
        let tileset_bank = self.read(0xD52B) as usize;

        // collision data is always in bank 0
        let collision_address = self.read_u16_le(0xD530);
        let mut collision_tiles = HashSet::new();
        for index in 0..20 {
            let collision_byte = self.read(collision_address + index);
            if collision_byte == 0xff {
                break;
            }
            collision_tiles.insert(collision_byte);
        }

        let map_data_address = self.read_u16_le(0xD36A);
        let map_data = self.rom_data_from_pointer(map_bank, map_data_address, map_height_blocks * map_width_blocks).to_vec();

        let max_block_id = *map_data.iter().max().unwrap() as usize;
        let block_data_address = self.read_u16_le(0xD52C);
        let block_data = self.rom_data_from_pointer(tileset_bank, block_data_address, (max_block_id + 1) * CurrentMap::BLOCK_TILES).to_vec();

        let warp_events = self.read_warp_events()?;

        let sprites = self.read_sprites()?;
        let player_position = Point8 { x: self.read(0xD362), y: self.read(0xD361) };

        let player_direction = PlayerFacingDirection::from_repr(self.read(0xD52A))
            .ok_or_else(|| format!("Invalid player facing direction {}", self.read(0xD52A)))?;

        // read current text
        // read wSpriteIndex from CF13 to resolve the current sprite
        let current_sprite_index = self.read(0xCF13) as usize;
        let current_text = if current_sprite_index == 0xFF || current_sprite_index == 0 {
            None
        } else {
            println!("current_sprite_index: {}", current_sprite_index);

            let text_address = self.read_u16_le(0xD36C);
            println!("text_table_pointer: {:04X}", text_address);
            let text_pointers = self.rom_data_from_pointer(map_bank, text_address, 0xFF * 2);
            // read text pointer via current_sprite_index
            let table_offset = (current_sprite_index - 1) * 2;
            let text_pointer = u16::from_le_bytes([
                text_pointers[table_offset],
                text_pointers[table_offset + 1]
            ]);


            println!("text_pointer: {:04X}", text_pointer);
            let text_bytes = self.rom_data_from_pointer(map_bank, text_pointer, None);

            // TODO this is a dead end... text_pointer is actually a pointer to the script
            // to truly resolve the text I will have to run the script on a forked GB with a BP on the call to PrintText

            println!("TEXT: {}", PokemonString::from_slice(text_bytes));
            Some(())
        };

        Ok(CurrentMap {
            map,
            player_position,
            player_direction,
            map_width_blocks,
            map_height_blocks,
            map_data,
            block_data,
            collision_tiles,
            warp_events,
            sprites,
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, strum_macros::Display)]
pub enum GameMode {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum_macros::Display)]
pub enum MetaTile {
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
    pub player_position: Point8,
    pub player_direction: PlayerFacingDirection,
    pub map_width_blocks: usize,
    pub map_height_blocks: usize,
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
        let block_index = self.map_data[block_x + block_y * self.map_width_blocks] as usize;
        let block_offset = block_index * Self::BLOCK_TILES;
        let tile_offset = (tile_x % Self::BLOCK_TILE_WIDTH) + (tile_y % Self::BLOCK_TILE_WIDTH) * Self::BLOCK_TILE_WIDTH;
        let tile_index = self.block_data[block_offset + tile_offset];
        self.collision_tiles.contains(&tile_index)
    }

    pub fn meta_width(&self) -> usize {
        self.map_width_blocks * Self::TILES_PER_META
    }

    pub fn meta_height(&self) -> usize {
        self.map_height_blocks * Self::TILES_PER_META
    }

    pub fn meta_tiles(&self) -> Vec<MetaTile> {
        let width = self.meta_width();
        let height = self.meta_height();

        // start off assuming all tiles are obstacles
        let mut result = vec![MetaTile::Obstacle; width * height];

        let width_tiles = self.map_width_blocks * Self::BLOCK_TILE_WIDTH;
        let height_tiles = self.map_height_blocks * Self::BLOCK_TILE_WIDTH;
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
    pub pokemon: u16,
    pub trainer_name: u16,
    pub nickname: u16,
}

impl PokemonBlockAddresses {
    pub const PARTY_MAX: u16 = 6;
    pub const POKEMON_BLOCK_SIZE: u16 = 0x2C;
    pub const NAME_LENGTH: u16 = 0xB;

    fn of_indexed(base_address: u16, index: u16) -> Self {
        Self {
            pokemon: base_address + index * Self::POKEMON_BLOCK_SIZE,
            trainer_name: base_address + Self::PARTY_MAX * Self::POKEMON_BLOCK_SIZE + index * Self::NAME_LENGTH,
            nickname: base_address + Self::PARTY_MAX * Self::POKEMON_BLOCK_SIZE + Self::PARTY_MAX * Self::NAME_LENGTH + index * Self::NAME_LENGTH,
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
    use crate::pokemon::status::PokemonStatus;
    use crate::roms::blargg_cpu::ROM;
    use crate::pokemon::*;
    use crate::pokemon::encoding::reverse_bcd;
    use crate::pokemon::strings::PokemonString;

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
    fn test_pokemon_encoding() {
        let mut mmu = MMU::from_rom(ROM).unwrap();
        let mut charizard = Pokemon {
            nickname: PokemonString::from_string("BACON"),
            species: PokemonSpecies::Charizard,
            current_hp: 65,
            status: PokemonStatus::None,
            types: [PokemonType::Fire, PokemonType::Flying],
            moves: [
                Some(PokemonMove {
                    name: PokemonMoveName::Flamethrower,
                    pp: 10
                }),
                Some(PokemonMove {
                    name: PokemonMoveName::FireBlast,
                    pp: 5
                }),
                Some(PokemonMove {
                    name: PokemonMoveName::Fly,
                    pp: 6
                }),
                None,
            ],
            trainer_name: PokemonString::from_string("LLM"),
            trainer_id: 57937,
            experience: 6457,
            effort_values: PokemonStats { attack: 100, defense: 200, speed: 300, special: 400, hp: 500 },
            individual_values: PokemonStats { attack: 5, defense: 10, speed: 15, special: 10, hp: 15 },
            level: 20,
            stats: PokemonStats { attack: 41, defense: 40, speed: 51, special: 44, hp: 66 },
        };

        charizard.recalculate();

        mmu.write_pokemon(0xD16B, 0, &charizard);
        assert_eq!(charizard, mmu.read_pokemon(0xD16B, 0).unwrap());
    }
}