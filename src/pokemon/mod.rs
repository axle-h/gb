use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::ops::{Deref, DerefMut, Index, IndexMut};
use strum::IntoEnumIterator;
use badge::Badge;
use map::Map;
use species::PokemonSpecies;
use unicode_segmentation::UnicodeSegmentation;
use actions::OverworldAction;
use party::PokemonParty;
use crate::game_boy::GameBoy;
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::mmu::MMU;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::pokemon::{Pokemon, PokemonStats, PokemonType};
use crate::pokemon::sprite::{PictureId, Sprite};

pub mod badge;
pub mod map;
pub mod pokemon;
pub mod status;
pub mod species;
pub mod move_name;
pub mod sprite;
pub mod party;
pub mod agent;
pub mod actions;

#[derive(Debug)]
pub struct PokemonApi<'a> {
    game_boy: &'a mut GameBoy
}

impl<'a> PokemonApi<'a> {
    pub fn new(game_boy: &'a mut GameBoy) -> Self {
        Self { game_boy }
    }

    fn mmu(&self) -> &MMU {
        self.game_boy.core().mmu()
    }

    fn mmu_mut(&mut self) -> &mut MMU {
        self.game_boy.core_mut().mmu_mut()
    }

    pub fn release_all_buttons(&mut self) {
        let joypad = self.mmu_mut().joypad_mut();
        for button in JoypadButton::iter() {
            joypad.release_button(button);
        }
    }

    pub fn press_button(&mut self, button: JoypadButton) {
        let joypad = self.mmu_mut().joypad_mut();
        joypad.press_button(button);
    }

    pub fn game_mode(&self) -> GameMode {
        let mmu = self.mmu();

        let menu_y = mmu.read(0xcc24);
        let menu_x = mmu.read(0xcc25);
        let wTextBoxID = mmu.read(0xd125);
        println!("menu position: {}, {}: {}", menu_x, menu_y, wTextBoxID);

        // ; lost battle, this is -1
        // ; no battle, this is 0
        // ; wild battle, this is 1
        // ; trainer battle, this is 2
        match mmu.read(0xD057) {
            1 => GameMode::WildBattle,
            2 => GameMode::TrainerBattle,
            _ => GameMode::Overworld // TODO menu
        }
    }

    pub fn player_state(&self) -> Result<PlayerState, String> {
        println!("{:x}, {:x}, {:x}", self.mmu().read(0xD347), self.mmu().read(0xD348), self.mmu().read(0xD349));
        Ok(PlayerState {
            player_id: self.mmu().read(0xD359) as u16 * 256 + self.mmu().read(0xD35A) as u16,
            name: self.mmu().read_pokemon_string(0xD158, PokemonBlockAddresses::NAME_LENGTH)?,
            rival_name: self.mmu().read_pokemon_string(0xD34A, 0x8)?,
            badges: Badge::parse_flags(self.mmu().read(0xD356)),
            money: reverse_bcd(self.mmu().read_u32_be(0xD346) & 0xFFFFFF),
        })
    }

    pub fn pokemon_party(&self) -> Result<PokemonParty, String> {
        let mmu = self.mmu();
        let count = mmu.read(0xD163);
        let mut party = PokemonParty::default();
        for i in 0..count {
            let pokemon = mmu.read_pokemon(0xD16B, i as u16)?;
            party.push(pokemon)?;
        }
        Ok(party)
    }

    pub fn write_pokemon_party(&mut self, party: PokemonParty) {
        let mmu = self.mmu_mut();
        mmu.write(0xD163, party.len() as u8); // length
        mmu.write(0xD164 + party.len() as u16, 0xFF); // list end
        for (index, pokemon) in party.into_iter().enumerate() {
            mmu.write_pokemon(0xD16B, index as u16, &pokemon);
            mmu.write(0xD164 + index as u16, pokemon.species as u8);
        }
    }

    pub fn map_state(&self) -> Result<MapState, String> {
        let mmu = self.mmu();
        let meta_tile_map = MetaTileMap::new(&CurrentMap::from_mmu(mmu)?);
        // println!("{}", meta_tile_map);
        let actions = meta_tile_map.actions();
        // for action in actions.iter() {
        //     println!("{:?}", action);
        // }

        Ok(MapState {
            map: meta_tile_map.map,
            player_position: meta_tile_map.player_position,
            player_tile: meta_tile_map.player_tile(),
            sprites: meta_tile_map.sprites,
            actions,
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
    Dialogue,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PlayerState {
    pub player_id: u16,
    pub name: String,
    pub rival_name: String,
    pub badges: Vec<Badge>,
    pub money: u32,
}


#[derive(Debug, Clone)]
pub struct MapState {
    pub map: Map,
    pub player_position: Point8,
    pub player_tile: MetaTile,
    pub sprites: Vec<Sprite>,
    pub actions: Vec<OverworldAction>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct WarpEvent {
    pub position: Point8,
    pub map_id: Map,
}

trait PokemonEncoding {
    fn read_pokemon_string(&self, address: u16, max_length: u16) -> Result<String, String>;

    fn write_pokemon_string(&mut self, address: u16, string: &str, max_length: u16);

    fn read_pokemon(&self, base_address: u16, index: u16) -> Result<Pokemon, String>;

    fn write_pokemon(&mut self, base_address: u16, index: u16, pokemon: &Pokemon);

    fn read_sprites(&self) -> Result<Vec<Sprite>, String>;

    fn read_warp_events(&self) -> Result<Vec<WarpEvent>, String>;
}

impl PokemonEncoding for MMU {
    fn read_pokemon_string(&self, address: u16, max_length: u16) -> Result<String, String> {
        // https://bulbapedia.bulbagarden.net/wiki/Character_encoding_(Generation_I)
        let mut utf8 = vec![];
        for i in 0..max_length {
            let byte = self.read(address + i);

            match byte {
                0x00 => utf8.push(b'\0'), // null
                0x50 => break, // end: marks the end of a string
                0x80..=0x99 => utf8.push(byte - 0x80 + b'A'), // A-Z
                0x9A => utf8.push(b'('),
                0x9B => utf8.push(b')'),
                0x9C => utf8.push(b':'),
                0x9D => utf8.push(b';'),
                0x9E => utf8.push(b'['),
                0x9F => utf8.push(b']'),
                0xA0..=0xB9 => utf8.push(byte - 0xA0 + b'a'), // a-z
                0xBA => utf8.push(b'e'),
                0xBB => utf8.push(b'd'),
                0xBC => utf8.push(b'l'),
                0xBD => utf8.push(b's'),
                0xBE => utf8.push(b't'),
                0xBF => utf8.push(b'v'),
                0xE0 => utf8.push(b'\''),
                0xE1 => utf8.push(b'P'), // pk character
                0xE2 => utf8.push(b'M'), // mn character
                0xE3 => utf8.push(b'-'),
                0xE4 => utf8.push(b'r'),
                0xE5 => utf8.push(b'm'),
                0xE6 => utf8.push(b'?'),
                0xE7 => utf8.push(b'!'),
                0xE8 => utf8.push(b'.'),
                0xE9 => utf8.extend_from_slice("ァ".as_bytes()),
                0xEA => utf8.extend_from_slice("ゥ".as_bytes()),
                0xEB => utf8.extend_from_slice("ェ".as_bytes()),
                0xEC => utf8.extend_from_slice("▷".as_bytes()),
                0xED => utf8.extend_from_slice("▶".as_bytes()),
                0xEE => utf8.extend_from_slice("▼".as_bytes()),
                0xEF => utf8.extend_from_slice("♂".as_bytes()),
                0xF1 => utf8.extend_from_slice("×".as_bytes()),
                0xF2 => utf8.push(b'.'),
                0xF3 => utf8.push(b'/'),
                0xF4 => utf8.push(b','),
                0xF5 => utf8.extend_from_slice("♀".as_bytes()),
                0xF6..=0xFF => utf8.push(byte - 0xF6 + b'0'), // 0-9
                _ => utf8.push(b' ') // Undefined characters simply print as spaces.
            };
        }
        std::str::from_utf8(&utf8)
            .map_err(|_| "Invalid UTF-8 in string".to_string())
            .map(|s| s.to_string())
    }

    fn write_pokemon_string(&mut self, address: u16, string: &str, max_length: u16) {
        // https://bulbapedia.bulbagarden.net/wiki/Character_encoding_(Generation_I)
        let graphemes = string.graphemes(true)
            .take(max_length as usize - 1); // -1 for terminator byte
        for (index, grapheme) in graphemes.enumerate() {
            let byte = if grapheme.bytes().count() > 1 {
                // unicode
                match grapheme {
                    "ァ" => 0xE9,
                    "ゥ" => 0xEA,
                    "ェ" => 0xEB,
                    "▷" => 0xEC,
                    "▶" => 0xED,
                    "▼" => 0xEE,
                    "♂" => 0xEF,
                    "×" => 0xF1,
                    "♀" => 0xF5,
                    _ => 0x00
                }
            } else {
                // ascii
                let char = grapheme.bytes().next().unwrap();
                match char {
                    b'A'..=b'Z' => (char - b'A') + 0x80,
                    b'a'..=b'z' => (char - b'a') + 0xA0,
                    b'0'..=b'9' => (char - b'0') + 0xF6,
                    b'(' => 0x9A,
                    b')' => 0x9B,
                    b':' => 0x9C,
                    b';' => 0x9D,
                    b'[' => 0x9E,
                    b']' => 0x9F,
                    b'\'' => 0xE0,
                    b'-' => 0xE3,
                    b'?' => 0xE6,
                    b'!' => 0xE7,
                    b'.' => 0xE8,
                    b'/' => 0xF3,
                    b',' => 0xF4,
                    b' ' => 0x7F,
                    _ => 0x00
                }
            };
            self.write(address + index as u16, byte);
        }
        self.write(address + string.len() as u16, 0x50);
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
            nickname: self.read_pokemon_string(addresses.nickname, PokemonBlockAddresses::NAME_LENGTH)?,
            trainer_name: self.read_pokemon_string(addresses.trainer_name, PokemonBlockAddresses::NAME_LENGTH)?,
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

        self.write_pokemon_string(addresses.nickname, &pokemon.nickname, PokemonBlockAddresses::NAME_LENGTH);
        self.write_pokemon_string(addresses.trainer_name, &pokemon.trainer_name, PokemonBlockAddresses::NAME_LENGTH);
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
    map: Map,
    player_position: Point8,
    player_direction: PlayerFacingDirection,
    map_width_blocks: usize,
    map_height_blocks: usize,
    map_data: Vec<u8>,
    block_data: Vec<u8>,
    collision_tiles: HashSet<u8>,
    warp_events: Vec<WarpEvent>,
    sprites: Vec<Sprite>,
}

impl CurrentMap {

    pub fn from_mmu(mmu: &MMU) -> Result<Self, String> {
        // any rom data we read must be directly from the rom banks as the game is not guaranteed to have the correct bank loaded
        let map = Map::from_repr(mmu.read(0xD35E)).ok_or_else(|| "Invalid map number".to_string())?;
        let map_bank = mmu.rom_data(3, 0x023D, Map::COUNT)[map as usize] as usize;
        let map_height_blocks = mmu.read(0xD368) as usize;
        let map_width_blocks = mmu.read(0xD369) as usize;
        let tileset_bank = mmu.read(0xD52B) as usize;

        // collision data is always in bank 0
        let collision_address = mmu.read_u16_le(0xD530);
        let mut collision_tiles = HashSet::new();
        for index in 0..20 {
            let collision_byte = mmu.read(collision_address + index);
            if collision_byte == 0xff {
                break;
            }
            collision_tiles.insert(collision_byte);
        }

        let map_data_address = mmu.read_u16_le(0xD36A);
        let map_data = mmu.rom_data_from_pointer(map_bank, map_data_address, map_height_blocks * map_width_blocks).to_vec();

        let max_block_id = *map_data.iter().max().unwrap() as usize;
        let block_data_address = mmu.read_u16_le(0xD52C);
        let block_data = mmu.rom_data_from_pointer(tileset_bank, block_data_address, (max_block_id + 1) * Self::BLOCK_TILES).to_vec();

        let warp_events = mmu.read_warp_events()?;

        let sprites = mmu.read_sprites()?;
        let player_position = Point8 { x: mmu.read(0xD362), y: mmu.read(0xD361) };

        let player_direction = PlayerFacingDirection::from_repr(mmu.read(0xD52A))
            .ok_or_else(|| format!("Invalid player facing direction {}", mmu.read(0xD52A)))?;

        Ok(Self {
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

pub struct MetaTileMap {
    player_position: Point8,
    player_direction: PlayerFacingDirection,
    map: Map,
    width: usize,
    height: usize,
    meta_tiles: Vec<MetaTile>,
    sprites: Vec<Sprite>,
    warp_targets: HashSet<Map>,
}

impl MetaTileMap {
    pub fn new(map: &CurrentMap) -> Self {
        Self {
            player_position: map.player_position,
            player_direction: map.player_direction,
            map: map.map,
            width: map.meta_width(),
            height: map.meta_height(),
            meta_tiles: map.meta_tiles(),
            sprites: map.sprites.clone(),
            warp_targets: map.warp_events.iter()
                .map(|warp_event| warp_event.map_id)
                .collect()
        }
    }

    pub fn player_tile(&self) -> MetaTile {
        self.meta_tiles[self.player_position.x as usize + self.player_position.y as usize * self.width]
    }

    fn player_route(&self, destination: Point8) -> Option<OverworldAction> {
        use std::collections::{BinaryHeap, HashMap};
        use std::cmp::Ordering;

        #[derive(Clone, Eq, PartialEq)]
        struct Node {
            position: Point8,
            cost: u32,
            heuristic: u32,
        }

        impl Ord for Node {
            fn cmp(&self, other: &Self) -> Ordering {
                (other.cost + other.heuristic).cmp(&(self.cost + self.heuristic))
            }
        }

        impl PartialOrd for Node {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        let heuristic = |pos: Point8| -> u32 {
            (pos.x.abs_diff(destination.x) + pos.y.abs_diff(destination.y)) as u32
        };

        let mut open_set = BinaryHeap::new();
        let mut came_from: HashMap<Point8, (Point8, JoypadButton)> = HashMap::new();
        let mut g_score: HashMap<Point8, u32> = HashMap::new();

        let origin = self.player_position;
        open_set.push(Node { position: origin, cost: 0, heuristic: heuristic(origin) });
        g_score.insert(origin, 0);

        while let Some(current) = open_set.pop() {
            if current.position == destination {
                let mut route = vec![];
                let mut pos = destination;
                while pos != origin {
                    if let Some((prev, dir)) = came_from.get(&pos) {
                        route.push(*dir);
                        pos = *prev;
                    }
                }
                route.reverse();
                return Some(
                    OverworldAction {
                        map: self.map,
                        origin,
                        destination,
                        tile: self.meta_tiles[destination.x as usize + destination.y as usize * self.width].clone(),
                        route,
                    }
                );
            }

            let neighbors = [
                (JoypadButton::Up, Point8 { x: current.position.x, y: current.position.y.saturating_sub(1) }),
                (JoypadButton::Down, Point8 { x: current.position.x, y: current.position.y + 1 }),
                (JoypadButton::Left, Point8 { x: current.position.x.saturating_sub(1), y: current.position.y }),
                (JoypadButton::Right, Point8 { x: current.position.x + 1, y: current.position.y }),
            ];

            for (dir, neighbor) in neighbors {
                if neighbor.x as usize >= self.width || neighbor.y as usize >= self.height {
                    continue;
                }

                let tile = &self.meta_tiles[neighbor.x as usize + neighbor.y as usize * self.width];
                if matches!(tile, MetaTile::Obstacle | MetaTile::Sprite(_)) && neighbor != destination {
                    continue;
                }

                let tentative_g = g_score.get(&current.position).unwrap_or(&u32::MAX) + 1;
                if tentative_g < *g_score.get(&neighbor).unwrap_or(&u32::MAX) {
                    came_from.insert(neighbor, (current.position, dir));
                    g_score.insert(neighbor, tentative_g);
                    open_set.push(Node {
                        position: neighbor,
                        cost: tentative_g,
                        heuristic: heuristic(neighbor),
                    });
                }
            }
        }
        None
    }

    pub fn actions(&self) -> Vec<OverworldAction> {
        // 1. routes to warps
        let mut routes = vec![];
        for to_map in &self.warp_targets {
            let target_tile = MetaTile::Warp(*to_map);
            let shortest_route = self.meta_tiles
                .iter()
                .enumerate()
                .filter(|(_, tile)| tile == &&target_tile)
                .map(|(index, _)| Point8 { x: (index % self.width) as u8, y: (index / self.width) as u8 })
                .filter_map(|to| self.player_route(to))
                .min_by(|a, b| a.route.len().cmp(&b.route.len()));

            if shortest_route.is_none() {
                continue;
            }

            let mut shortest_route = shortest_route.unwrap();

            if shortest_route.destination.x == 0 {
                shortest_route.route.push(JoypadButton::Left);
            } else if shortest_route.destination.x == (self.width - 1) as u8 {
                shortest_route.route.push(JoypadButton::Right);
            } else if shortest_route.destination.y == 0 {
                shortest_route.route.push(JoypadButton::Up);
            } else if shortest_route.destination.y == (self.height - 1) as u8 {
                shortest_route.route.push(JoypadButton::Down);
            }

            routes.push(shortest_route);
        }

        // 2. routes to sprites - keep shortest route per sprite
        for sprite in &self.sprites {
            if sprite.hidden {
                continue;
            }

            let sprite_pos = sprite.position;
            let adjacent_positions = [
                (PlayerFacingDirection::Down, Point8 { x: sprite_pos.x, y: sprite_pos.y.saturating_sub(1) }),
                (PlayerFacingDirection::Up, Point8 { x: sprite_pos.x, y: sprite_pos.y + 1 }),
                (PlayerFacingDirection::Right, Point8 { x: sprite_pos.x.saturating_sub(1), y: sprite_pos.y }),
                (PlayerFacingDirection::Left, Point8 { x: sprite_pos.x + 1, y: sprite_pos.y }),
            ];
            let shortest_route = adjacent_positions
                .into_iter()
                .filter(|(_, pos)| {
                    pos.x < self.width as u8 && pos.y < self.height as u8 &&
                        matches!(self.meta_tiles[pos.x as usize + pos.y as usize * self.width], MetaTile::Empty)
                })
                .filter_map(|(dir, to)| {
                    self.player_route(to).map(|route| (dir, route))
                })
                .min_by(|(_, a), (_, b)| a.route.len().cmp(&b.route.len()));

            if shortest_route.is_none() {
                continue;
            }
            let (sprite_direction, mut shortest_route) = shortest_route.unwrap();
            shortest_route.tile = MetaTile::Sprite(sprite.name);
            if shortest_route.route.is_empty() {
                if sprite_direction != self.player_direction {
                    // no more movement needed, just turn to face sprite
                    shortest_route.route.push(sprite_direction.into());
                }
            } else {
                // ensure we face the sprite before interacting
                let last_move = shortest_route.route.last().unwrap();
                let expected_button: JoypadButton = sprite_direction.into();
                if *last_move != expected_button {
                    shortest_route.route.push(expected_button);
                }
            }
            shortest_route.route.push(JoypadButton::A);
            routes.push(shortest_route);
        }
        routes
    }
}

impl Display for MetaTileMap {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for y in 0..self.height {
            for x in 0..self.width {
                match self.meta_tiles[x + y * self.width] {
                    MetaTile::Empty => write!(f, " ")?,
                    MetaTile::Obstacle => write!(f, "O")?,
                    MetaTile::Sprite(_) => write!(f, "S")?,
                    MetaTile::Warp(_) => write!(f, "W")?,
                }
            }
            writeln!(f)?;
        }
        writeln!(f)
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

fn reverse_bcd(mut value: u32) -> u32 {
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
    use super::*;

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
            nickname: "BACON".to_string(),
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
            trainer_name: "LLM".to_string(),
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