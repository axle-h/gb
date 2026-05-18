use std::fmt::{Display, Formatter};
use crate::pokemon::battle::{BattleAction};
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::status::PokemonStatus;
use crate::pokemon::strings::PokemonString;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pokemon {
    pub nickname: PokemonString,
    pub species: PokemonSpecies,
    pub current_hp: u16,
    pub status: PokemonStatus,
    pub types: [PokemonType; 2],
    pub moves: [Option<PokemonMove>; 4],
    pub trainer_name: PokemonString,
    pub trainer_id: u16,
    pub experience: u32,
    pub effort_values: PokemonStats,
    pub individual_values: PokemonStats,
    pub level: u8,
    pub stats: PokemonStats,
}

impl Pokemon {
    pub fn maxed<NN: Into<PokemonString>, TN : Into<PokemonString>>(species: PokemonSpecies, nickname: NN, moves: [PokemonMoveName; 4], trainer_name: TN, trainer_id: u16) -> Self {
        let metadata = species.metadata();
        let mut result = Self {
            nickname: nickname.into(),
            species,
            current_hp: u16::MAX, // temporary, will be recalculated
            status: PokemonStatus::default(),
            types: [metadata.type1, metadata.type2.unwrap_or(metadata.type1)],
            moves: moves.map(|move_name| Some(PokemonMove::with_max_pp(move_name))),
            trainer_name: trainer_name.into(),
            trainer_id,
            experience: metadata.experience_group.experience_for_level(100),
            effort_values: PokemonStats::MAX_EV,
            individual_values: PokemonStats::MAX_IV,
            level: 100,
            stats: PokemonStats::ZERO, // temporary, will be recalculated
        };
        result.recalculate();
        result
    }
    
    pub fn recalculate(&mut self) {
        let metadata = self.species.metadata();
        
        self.experience &= 0xFFFFFF;
        self.individual_values = self.individual_values.truncated_to_iv();
        self.level = metadata.experience_group.level_from_experience(self.experience);
        self.stats = self.recalculated_stats();
        self.current_hp = self.current_hp.min(self.stats.hp);

        // Ensure all moves don't exceed their maximum PP
        for move_slot in &mut self.moves {
            if let Some(pokemon_move) = move_slot {
                let max_pp = pokemon_move.name.metadata().pp;
                pokemon_move.pp = pokemon_move.pp.min(max_pp);
            }
        }

        // revalidate types against pokedex
        self.types[0] = metadata.type1;
        self.types[1] = metadata.type2.unwrap_or(metadata.type1);
    }

    pub fn recalculated_stats(&self) -> PokemonStats {
        let base = self.species.metadata().base_stats;
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

    pub fn summary(&self) -> PokemonSummary {
        PokemonSummary {
            species: self.species,
            current_hp: self.current_hp,
            status: self.status,
            types: self.types,
            level: self.level,
            moves: self.moves,
            stats: self.stats,
        }
    }

}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PokemonSummary {
    pub species: PokemonSpecies,
    pub current_hp: u16,
    pub status: PokemonStatus,
    pub types: [PokemonType; 2],
    pub level: u8,
    pub moves: [Option<PokemonMove>; 4],
    pub stats: PokemonStats,
}

impl PokemonSummary {
    pub fn available_battle_moves(&self) -> Vec<BattleAction> {
        self.moves.iter().enumerate()
            .filter_map(|(i, m)|
                m.filter(|m| m.pp > 0)
                    .map(|m| BattleAction::Fight {
                        slot: i as u8,
                        battle_move: m
                    })
            )
            .collect()
    }
}

impl Display for PokemonSummary {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} Lv.{} HP {}/{}", self.species, self.level, self.current_hp, self.stats.hp)?;
        if self.status != PokemonStatus::None {
            write!(f, "{}", self.status)?;
        }
        Ok(())
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
    pub const MAX_IV: Self = Self { attack: 15, defense: 15, speed: 15, special: 15, hp: 15 };
    pub const MAX_EV: Self = Self { attack: u16::MAX, defense: u16::MAX, speed: u16::MAX, special: u16::MAX, hp: u16::MAX };
    
    pub const ZERO: Self = Self { attack: 0, defense: 0, speed: 0, special: 0, hp: 0 };
    
    pub const fn new(hp: u16, attack: u16, defense: u16, speed: u16, special: u16) -> Self {
        Self { attack, defense, speed, special, hp }
    }

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

pub enum MoveEffectiveness {
    /// The move does base damage.
    Base,
    /// The move is 'not very effective' (0.5x damage)
    Half,
    /// The move is 'super effective' (2x damage)
    Double,
    /// The move has no effect (0 damage)
    None,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug, strum_macros::Display)]
pub enum PokemonTypeCategory {
    Physical,
    Special,
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
    /// unused https://bulbapedia.bulbagarden.net/wiki/Bird_(type)
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


impl PokemonType {
    pub fn category(&self) -> PokemonTypeCategory {
        use PokemonType::*;
        match self {
            Bug | Fighting | Flying | Ghost | Ground | Normal | Poison | Rock | Bird => PokemonTypeCategory::Physical,
            Dragon | Electric | Fire | Grass | Ice | Psychic | Water => PokemonTypeCategory::Special,
        }
    }

    pub fn attack_effectiveness(&self, defending: PokemonType) -> MoveEffectiveness {
        // https://pokemondb.net/type/old
        use PokemonType::*;
        match self {
            Normal => match defending {
                Rock => MoveEffectiveness::Half,
                Ghost => MoveEffectiveness::None,
                _ => MoveEffectiveness::Base,
            },
            Fighting => match defending {
                Normal | Ice | Rock => MoveEffectiveness::Double,
                Poison | Flying | Psychic | Bug => MoveEffectiveness::Half,
                Ghost => MoveEffectiveness::None,
                _ => MoveEffectiveness::Base,
            }
            Flying => match defending {
                Grass | Fighting | Bug => MoveEffectiveness::Double,
                Rock | Electric => MoveEffectiveness::Half,
                _ => MoveEffectiveness::Base,
            }
            Poison => match defending {
                Grass | Bug => MoveEffectiveness::Double,
                Poison | Ground | Rock | Ghost => MoveEffectiveness::Half,
                _ => MoveEffectiveness::Base,
            }
            Ground => match defending {
                Fire | Electric | Poison | Rock => MoveEffectiveness::Double,
                Grass | Bug => MoveEffectiveness::Half,
                Flying => MoveEffectiveness::None,
                _ => MoveEffectiveness::Base,
            }
            Rock => match defending {
                Fire | Ice | Flying | Bug => MoveEffectiveness::Double,
                Fighting | Ground => MoveEffectiveness::Half,
                _ => MoveEffectiveness::Base,
            }
            Bird => MoveEffectiveness::Base,
            Bug => match defending {
                Grass | Poison | Psychic => MoveEffectiveness::Double,
                Fire | Fighting | Flying | Ghost => MoveEffectiveness::Half,
                _ => MoveEffectiveness::Base,
            }
            Ghost => match defending {
                Ghost => MoveEffectiveness::Double,
                Normal | Psychic => MoveEffectiveness::None,
                _ => MoveEffectiveness::Base,
            }
            Fire => match defending {
                Grass | Ice | Bug => MoveEffectiveness::Double,
                Fire | Water | Rock | Dragon => MoveEffectiveness::Half,
                _ => MoveEffectiveness::Base,
            }
            Water => match defending {
                Fire | Ground | Rock => MoveEffectiveness::Double,
                Water | Grass | Dragon => MoveEffectiveness::Half,
                _ => MoveEffectiveness::Base,
            }
            Grass => match defending {
                Water | Ground | Rock => MoveEffectiveness::Double,
                Fire | Grass | Poison | Flying | Bug | Dragon => MoveEffectiveness::Half,
                _ => MoveEffectiveness::Base,
            }
            Electric => match defending {
                Water | Flying => MoveEffectiveness::Double,
                Electric | Grass | Dragon => MoveEffectiveness::Half,
                Ground => MoveEffectiveness::None,
                _ => MoveEffectiveness::Base,
            }
            Psychic => match defending {
                Fighting | Poison => MoveEffectiveness::Double,
                Psychic => MoveEffectiveness::Half,
                _ => MoveEffectiveness::Base,
            }
            Ice => match defending {
                Grass | Ground | Flying | Dragon => MoveEffectiveness::Double,
                Water | Ice => MoveEffectiveness::Half,
                _ => MoveEffectiveness::Base,
            }
            Dragon => match defending {
                Dragon => MoveEffectiveness::Double,
                _ => MoveEffectiveness::Base,
            }
        }

    }
}