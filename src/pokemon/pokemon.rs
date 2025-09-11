use crate::pokemon::move_name::PokemonMove;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::status::PokemonStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pokemon {
    pub nickname: String,
    pub species: PokemonSpecies,
    pub current_hp: u16,
    pub status: PokemonStatus,
    pub types: [PokemonType; 2], // TODO revalidate types against pokedex on write
    pub moves: [Option<PokemonMove>; 4],
    pub trainer_name: String,
    pub trainer_id: u16,
    pub experience: u32,
    pub effort_values: PokemonStats,
    pub individual_values: PokemonStats,
    pub level: u8,
    pub stats: PokemonStats,
}

impl Pokemon {
    pub fn recalculate(&mut self) {
        self.experience &= 0xFFFFFF;
        self.individual_values = self.individual_values.truncated_to_iv();
        self.level = self.species.experience_group().level_from_experience(self.experience);
        self.stats = self.recalculated_stats();
        self.current_hp = self.current_hp.min(self.stats.hp);
    }

    pub fn recalculated_stats(&self) -> PokemonStats {
        let base = self.species.base_stats();
        PokemonStats {
            hp: self.stat0(base.hp, self.individual_values.hp, self.effort_values.hp) + self.level as u16 + 10,
            attack: self.stat(base.attack, self.individual_values.attack, self.effort_values.attack),
            defense: self.stat(base.defense, self.individual_values.defense, self.effort_values.defense),
            speed: self.stat(base.speed, self.individual_values.speed, self.effort_values.speed),
            special: self.stat(base.special, self.individual_values.special, self.effort_values.special),
        }
    }

    fn stat0(&self, base_stat: u16, iv: u16, ev: u16) -> u16 {
        //floor((((B + I) × 2 + floor(ceil(sqrt(E)) ÷ 4)) × L) ÷ 100)
        ((2 * (base_stat + iv) + (ev as f64).sqrt().ceil() as u16 / 4) * self.level as u16) / 100
    }

    fn stat(&self, base_stat: u16, iv: u16, ev: u16) -> u16 {
        self.stat0(base_stat, iv, ev) + 5
    }

}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PokemonStats {
    pub attack: u16,
    pub defense: u16,
    pub speed: u16,
    pub special: u16,
    pub hp: u16,
}

impl PokemonStats {

    pub fn truncated_to_iv(self) -> Self {
        let mut result = Self {
            attack: self.attack & 0xF,
            defense: self.defense & 0xF,
            speed: self.speed & 0xF,
            special: self.special & 0xF,
            hp: 0,
        };
        result.recalculate_hp_iv();
        result
    }

    pub fn from_iv_bytes(attack_defense: u8, speed_special: u8) -> Self {
        let attack = (attack_defense >> 4) & 0xF;
        let defense = attack_defense & 0xF;
        let speed = (speed_special >> 4) & 0xF;
        let special = speed_special & 0xF;

        let mut result = Self {
            attack: attack as u16,
            defense: defense as u16,
            speed: speed as u16,
            special: special as u16,
            hp: 0,
        };
        result.recalculate_hp_iv();
        result
    }

    pub fn into_iv_bytes(self) -> (u8, u8) {
        let attack = (self.attack & 0xF) as u8;
        let defense = (self.defense & 0xF) as u8;
        let speed = (self.speed & 0xF) as u8;
        let special = (self.special & 0xF) as u8;

        let attack_defense = (attack << 4) | defense;
        let speed_special = (speed << 4) | special;

        (attack_defense, speed_special)
    }

    fn recalculate_hp_iv(&mut self) {
        // The HP IV is calculated by taking the least significant bit (the final binary digit) of the Attack, Defense, Speed, and Special IVs,
        // then creating a binary string by placing them in that order.
        self.hp = ((self.attack & 0x1) << 3) | ((self.defense & 0x1) << 2) | ((self.speed & 0x1) << 1) | (self.special & 0x1);
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug, strum_macros::Display, strum_macros::FromRepr)]
#[repr(u8)]
pub enum PokemonType {
    Normal = 0,
    Fighting,
    Flying,
    Poison,
    Ground,
    Rock,
    Bird,
    Bug,
    Ghost,
    Fire = 20,
    Water,
    Grass,
    Electric,
    Psychic,
    Ice,
    Dragon,
}