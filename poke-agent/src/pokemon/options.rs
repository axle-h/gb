use gb::mmu::MMU;
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

/// What a served run plays on, the web UI and the SDL window: fast text and no switch prompt,
/// with the animations kept because somebody is watching.
pub const SERVED_OPTIONS: GameOptions = GameOptions {
    battle_animations_on: true,
    battle_style: BattleStyle::Set,
    text_speed: TextSpeed::Fast,
};

/// What a headless run plays on, every test fixture: [`SERVED_OPTIONS`] without the animations.
pub const HEADLESS_OPTIONS: GameOptions = GameOptions {
    battle_animations_on: false,
    ..SERVED_OPTIONS
};

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

/// Write `options` if `wOptions` says anything else, answering whether it did. Cheap enough to
/// call every tick, which a host has to: Continue restores the options the save was written with.
pub fn keep_game_options(mmu: &mut MMU, options: &GameOptions) -> bool {
    // An unreadable byte, which a fresh boot leaves, counts as drifted.
    let drifted = mmu.read_game_options().map_or(true, |live| live != *options);
    if drifted {
        mmu.write_game_options(options).ok();
    }
    drifted
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb::mmu::MMU;
    use gb::roms::blargg_cpu::ROM;

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
