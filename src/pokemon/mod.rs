use std::ops::{Deref, DerefMut, Index, IndexMut};
use badge::Badge;
use map::Map;
use species::PokemonSpecies;
use unicode_segmentation::UnicodeSegmentation;
use crate::game_boy::GameBoy;
use crate::geometry::Point8;
use crate::mmu::MMU;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::pokemon::{Pokemon, PokemonStats, PokemonType};

pub mod badge;
pub mod map;
pub mod pokemon;
pub mod status;
pub mod species;
pub mod move_name;

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
        Ok(MapState {
            map_number: Map::from_repr(self.mmu().read(0xD35E)).ok_or_else(|| "Invalid map number".to_string())?,
            position: Point8 { x: self.mmu().read(0xD362), y: self.mmu().read(0xD361) },
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PlayerState {
    player_id: u16,
    name: String,
    rival_name: String,
    badges: Vec<Badge>,
    money: u32,
}

#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct PokemonParty(Vec<Pokemon>);

impl PokemonParty {
    pub fn push(&mut self, pokemon: Pokemon) -> Result<(), String> {
        if self.0.len() >= PokemonBlockAddresses::PARTY_MAX as usize {
            Err("Party is full".to_string())
        } else {
            self.0.push(pokemon);
            Ok(())
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl Index<usize> for PokemonParty {
    type Output = Pokemon;

    fn index(&self, index: usize) -> &Self::Output {
        self.0.index(index)
    }
}

impl IndexMut<usize> for PokemonParty {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.0.index_mut(index)
    }
}

impl IntoIterator for PokemonParty {
    type Item = Pokemon;
    type IntoIter = std::vec::IntoIter<Pokemon>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}


#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct MapState {
    map_number: Map,
    position: Point8,
}

trait PokemonEncoding {
    fn read_pokemon_string(&self, address: u16, max_length: u16) -> Result<String, String>;

    fn write_pokemon_string(&mut self, address: u16, string: &str, max_length: u16);

    fn read_pokemon(&self, base_address: u16, index: u16) -> Result<Pokemon, String>;

    fn write_pokemon(&mut self, base_address: u16, index: u16, pokemon: &Pokemon);
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