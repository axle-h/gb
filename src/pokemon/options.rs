use crate::mmu::MMU;
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleStyle { Set, Shift }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSpeed { Slow, Medium, Fast }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOptions {
    pub battle_animations_on: bool,
    pub battle_style: BattleStyle,
    pub text_speed: TextSpeed,
}

impl Default for GameOptions {
    fn default() -> Self {
        Self {
            battle_animations_on: true,
            battle_style: BattleStyle::Set,
            text_speed: TextSpeed::Fast,
        }
    }
}

pub trait GameOptionsReader {
    fn read_game_options(&self) -> Result<GameOptions, String>;
}

impl GameOptionsReader for MMU {
    fn read_game_options(&self) -> Result<GameOptions, String> {
        /*
        Bit 7 = Battle Animation (1 = Off, 0 = On)
        Bit 6 = Battle Style (1 = Set, 0 = Shift)
        Bit 5-4 = probably unused
        Low nibble = Text Speed (0x0 = fastest, 0xF = slowest)
            Fast = 1
            Medium = 3
            Slow = 5
         */
        let byte = self.read_pointer(&pokered_symbols::wOptions);

        let battle_animations_on = (byte & 0x80) == 0;
        let battle_style = if (byte & 0x40) != 0 { BattleStyle::Set } else { BattleStyle::Shift };
        let text_speed = match byte & 0x0F {
            1 => TextSpeed::Fast,
            3 => TextSpeed::Medium,
            5 => TextSpeed::Slow,
            n => return Err(format!("Unknown text speed: {}", n)),
        };

        Ok(GameOptions { battle_animations_on, battle_style, text_speed })
    }
}

pub trait GameOptionsWriter {
    fn write_game_options(&mut self, options: &GameOptions) -> Result<(), String>;
}

impl GameOptionsWriter for MMU {
    fn write_game_options(&mut self, options: &GameOptions) -> Result<(), String> {
        let options_byte =
            (if options.battle_animations_on { 0 } else { 0x80 }) |
            (if options.battle_style == BattleStyle::Set { 0x40 } else { 0 }) |
            match options.text_speed {
                TextSpeed::Fast => 1,
                TextSpeed::Medium => 3,
                TextSpeed::Slow => 5,
            };

        self.write_pointer(&pokered_symbols::wOptions, options_byte)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mmu::MMU;
    use crate::roms::blargg_cpu::ROM;

    #[test]
    fn test_round_trip_all_combinations() {
        for battle_animations_on in [true, false] {
            for battle_style in [BattleStyle::Set, BattleStyle::Shift] {
                for text_speed in [TextSpeed::Fast, TextSpeed::Medium, TextSpeed::Slow] {
                    let options = GameOptions { battle_animations_on, battle_style, text_speed };
                    let mut mmu = MMU::from_rom(ROM).unwrap();
                    mmu.write_game_options(&options).unwrap();
                    assert_eq!(mmu.read_game_options().unwrap(), options);
                }
            }
        }
    }
}
