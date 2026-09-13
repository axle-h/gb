//! `TextCommandProcessor`'s bytecode, decoded from the cartridge into a typed list.
//!
//! A script is read straight out of the ROM: `TX_FAR` is followed and flattened, since it is a
//! static jump the assembler resolved, and the script ends where the cartridge's would. What a
//! command reads from RAM is a named buffer rather than an address, because the WRAM layout is the
//! program's and not the game's; the union at `$CF4B` is why the lookup is per command kind.

use serde::{Deserialize, Serialize};
use crate::pointer::{DmgBank, DmgPointer};
use crate::rom_gfx::rom_slice;
use crate::symbols::pokered_symbols as sym;

/// `@`, which ends a string, and `TX_END`, which ends a script: the cartridge spells both `$50`.
const TERMINATOR: u8 = 0x50;
/// `<DONE>` and `<PROMPT>`, which end the whole script from inside a string.
const DONE: u8 = 0x57;
const PROMPT: u8 = 0x58;

const TX_START: u8 = 0x00;
const TX_RAM: u8 = 0x01;
const TX_BCD: u8 = 0x02;
const TX_MOVE: u8 = 0x03;
const TX_BOX: u8 = 0x04;
const TX_LOW: u8 = 0x05;
const TX_PROMPT_BUTTON: u8 = 0x06;
const TX_SCROLL: u8 = 0x07;
const TX_START_ASM: u8 = 0x08;
const TX_NUM: u8 = 0x09;
const TX_PAUSE: u8 = 0x0A;
const TX_SOUND_GET_ITEM_1: u8 = 0x0B;
const TX_DOTS: u8 = 0x0C;
const TX_WAIT_BUTTON: u8 = 0x0D;
/// `NextTextCommand` sends this and every byte up to `TX_FAR` to `TextCommand_SOUND`.
const TX_SOUND_POKEDEX_RATING: u8 = 0x0E;
const TX_FAR: u8 = 0x17;

const MONEY_SIGN: u8 = 1 << 5;
const LEFT_ALIGN: u8 = 1 << 6;
const LEADING_ZEROES: u8 = 1 << 7;

/// One entry of `TextCommandJumpTable`. `TX_END` is the list running out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextCommand {
    /// `TX_START`: charmap bytes up to the `@`, printed a letter at a time.
    Text(Vec<u8>),
    /// `TX_RAM`: a string the caller left in a buffer.
    Buffer(TextBuffer),
    /// `TX_NUM`: `bytes` of a number, big-endian, in `digits` digits and always left-aligned.
    Number { source: TextNumber, bytes: u8, digits: u8 },
    /// `TX_BCD`: `bytes` packed two digits each.
    Bcd { source: TextMoney, bytes: u8, skip_leading_zeroes: bool, left_align: bool, money_sign: bool },
    /// `TX_LOW`: print on the box's second line.
    Low,
    /// `TX_PROMPT_BUTTON`: the `▼`, then a press.
    PromptButton,
    /// `TX_WAIT_BUTTON`: a press, with no `▼`.
    WaitButton,
    /// `TX_PAUSE`: a press, or thirty frames.
    Pause,
    Sound(TextSound),
    /// `TX_START_ASM`: the routine that takes the printing over, named by where it is. The chunk
    /// that recreates that routine is the one that gives it a step.
    Asm(DmgPointer),
    /// `TX_MOVE`: print from this tile on.
    Move(u16),
    /// `TX_BOX`: a border at `at`, around `width` × `height` tiles.
    Box { at: u16, width: u8, height: u8 },
    /// `TX_DOTS`: that many `…`, a press or ten frames apart.
    Dots(u8),
    /// `TX_SCROLL`: up two lines, with no `▼` and no wait.
    Scroll,
}

/// The scratch buffers `TX_RAM` prints from, which the caller fills before it prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TextBuffer {
    StringBuffer,
    NameBuffer,
    EnemyMonNick,
    BattleMonNick,
    TrainerName,
    OaksAideRewardItemName,
    LearnMoveMonName,
    GymLeaderName,
    GymCityName,
    DayCareMonName,
    BoxMonNicks,
    InGameTradeGiveMonName,
    InGameTradeReceiveMonName,
    NameOfPlayerMonToBeTraded,
    LinkEnemyTrainerName,
    BoxNumString,
    Buffer,
}

/// What `TX_NUM` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TextNumber {
    OaksAideRequirement,
    OaksAideNumMonsOwned,
    CurEnemyLevel,
    PlayerNumHits,
    EnemyNumHits,
    HpBarHpDifference,
    ExpAmountGained,
    DexRatingNumMonsSeen,
    DexRatingNumMonsOwned,
    DexRatingNumMonsSeenH,
    DexRatingNumMonsOwnedH,
    DayCareNumLevelsGrown,
    TextId,
}

/// What `TX_BCD` reads: money and coins, which the cartridge keeps as BCD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TextMoney {
    Money,
    Coins,
    PlayerCoins,
    TotalPayDayMoney,
    AmountMoneyWon,
    DayCareTotalCost,
}

/// `TextCommandSounds`. The three cries are the only species the table names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TextSound {
    GetItem1,
    GetItem2,
    GetKeyItem,
    CaughtMon,
    DexPageAdded,
    PokedexRating,
    GetItem1Duplicate,
    CryNidorina,
    CryPidgeot,
    CryDewgong,
}

macro_rules! by_address {
    ($name:ident, $($symbol:ident => $variant:expr),+ $(,)?) => {
        impl $name {
            fn of(address: u16) -> Option<Self> {
                match address {
                    $(a if a == sym::$symbol.address => Some($variant),)+
                    _ => None,
                }
            }
        }
    };
}

by_address!(TextBuffer,
    wStringBuffer => Self::StringBuffer,
    wNameBuffer => Self::NameBuffer,
    wEnemyMonNick => Self::EnemyMonNick,
    wBattleMonNick => Self::BattleMonNick,
    wTrainerName => Self::TrainerName,
    wOaksAideRewardItemName => Self::OaksAideRewardItemName,
    wLearnMoveMonName => Self::LearnMoveMonName,
    wGymLeaderName => Self::GymLeaderName,
    wGymCityName => Self::GymCityName,
    wDayCareMonName => Self::DayCareMonName,
    wBoxMonNicks => Self::BoxMonNicks,
    wInGameTradeGiveMonName => Self::InGameTradeGiveMonName,
    wInGameTradeReceiveMonName => Self::InGameTradeReceiveMonName,
    wNameOfPlayerMonToBeTraded => Self::NameOfPlayerMonToBeTraded,
    wLinkEnemyTrainerName => Self::LinkEnemyTrainerName,
    wBoxNumString => Self::BoxNumString,
    wBuffer => Self::Buffer,
);

by_address!(TextNumber,
    hOaksAideRequirement => Self::OaksAideRequirement,
    hOaksAideNumMonsOwned => Self::OaksAideNumMonsOwned,
    wCurEnemyLevel => Self::CurEnemyLevel,
    wPlayerNumHits => Self::PlayerNumHits,
    wEnemyNumHits => Self::EnemyNumHits,
    wHPBarHPDifference => Self::HpBarHpDifference,
    wExpAmountGained => Self::ExpAmountGained,
    wDexRatingNumMonsSeen => Self::DexRatingNumMonsSeen,
    wDexRatingNumMonsOwned => Self::DexRatingNumMonsOwned,
    hDexRatingNumMonsSeen => Self::DexRatingNumMonsSeenH,
    hDexRatingNumMonsOwned => Self::DexRatingNumMonsOwnedH,
    wDayCareNumLevelsGrown => Self::DayCareNumLevelsGrown,
    hTextID => Self::TextId,
);

by_address!(TextMoney,
    hMoney => Self::Money,
    hCoins => Self::Coins,
    wPlayerCoins => Self::PlayerCoins,
    wTotalPayDayMoney => Self::TotalPayDayMoney,
    wAmountMoneyWon => Self::AmountMoneyWon,
    wDayCareTotalCost => Self::DayCareTotalCost,
);

impl TextSound {
    fn of(command: u8) -> Option<Self> {
        Some(match command {
            TX_SOUND_GET_ITEM_1 => Self::GetItem1,
            TX_SOUND_POKEDEX_RATING => Self::PokedexRating,
            0x0F => Self::GetItem1Duplicate,
            0x10 => Self::GetItem2,
            0x11 => Self::GetKeyItem,
            0x12 => Self::CaughtMon,
            0x13 => Self::DexPageAdded,
            0x14 => Self::CryNidorina,
            0x15 => Self::CryPidgeot,
            0x16 => Self::CryDewgong,
            _ => return None,
        })
    }
}

/// A `TX_FAR` inside a `TX_FAR`: the cartridge nests one deep, and this is room to spare.
const MAX_DEPTH: usize = 8;

/// The script at `at`, with every `TX_FAR` followed and flattened.
/// The script a `text_far` label names. A caller that knows a text by name rather than by address
/// goes through here, since the address is the build's rather than the game's.
pub fn far_text(label: &str) -> Result<Vec<TextCommand>, String> {
    let (_, at) = crate::symbols::pokered_symbols::TEXT_LABELS
        .iter()
        .find(|(name, _)| *name == label)
        .ok_or_else(|| format!("no text is labelled {label}"))?;
    decode(*at)
}

pub fn decode(at: DmgPointer) -> Result<Vec<TextCommand>, String> {
    let mut commands = Vec::new();
    decode_into(rom_slice(at), at, &mut commands, 0)?;
    Ok(commands)
}

/// A script that is not in the ROM: one the caller built, or one the cartridge left in RAM. `at` is
/// only where it claims to be, for a `TX_START_ASM` to name and an error to report.
pub fn decode_slice(bytes: &[u8], at: DmgPointer) -> Result<Vec<TextCommand>, String> {
    let mut commands = Vec::new();
    decode_into(bytes, at, &mut commands, 0)?;
    Ok(commands)
}

fn decode_into(bytes: &[u8], at: DmgPointer, commands: &mut Vec<TextCommand>, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{at} nests text more than {MAX_DEPTH} deep"));
    }
    let mut i = 0;
    let word = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    loop {
        let command = *bytes.get(i).ok_or_else(|| format!("{at} runs off the end of its bank"))?;
        i += 1;
        match command {
            TERMINATOR => return Ok(()),
            TX_START => {
                let start = i;
                while !matches!(bytes[i], TERMINATOR | DONE | PROMPT) {
                    i += 1;
                }
                let ends_here = bytes[i] != TERMINATOR;
                // `<DONE>` and `<PROMPT>` are the printer's, so they stay in the run it prints.
                if ends_here {
                    i += 1;
                }
                commands.push(TextCommand::Text(bytes[start..i].to_vec()));
                if ends_here {
                    return Ok(());
                }
                i += 1;
            }
            TX_RAM => {
                let address = word(i);
                let source = TextBuffer::of(address)
                    .ok_or_else(|| format!("{at}: TX_RAM reads ${address:04X}, which has no name"))?;
                commands.push(TextCommand::Buffer(source));
                i += 2;
            }
            TX_NUM => {
                let address = word(i);
                let source = TextNumber::of(address)
                    .ok_or_else(|| format!("{at}: TX_NUM reads ${address:04X}, which has no name"))?;
                let nybbles = bytes[i + 2];
                commands.push(TextCommand::Number { source, bytes: nybbles >> 4, digits: nybbles & 0x0F });
                i += 3;
            }
            TX_BCD => {
                let address = word(i);
                let source = TextMoney::of(address)
                    .ok_or_else(|| format!("{at}: TX_BCD reads ${address:04X}, which has no name"))?;
                let flags = bytes[i + 2];
                commands.push(TextCommand::Bcd {
                    source,
                    bytes: flags & !(MONEY_SIGN | LEFT_ALIGN | LEADING_ZEROES),
                    skip_leading_zeroes: flags & LEADING_ZEROES != 0,
                    left_align: flags & LEFT_ALIGN != 0,
                    money_sign: flags & MONEY_SIGN != 0,
                });
                i += 3;
            }
            TX_MOVE => {
                commands.push(TextCommand::Move(word(i)));
                i += 2;
            }
            TX_BOX => {
                commands.push(TextCommand::Box { at: word(i), height: bytes[i + 2], width: bytes[i + 3] });
                i += 4;
            }
            TX_DOTS => {
                commands.push(TextCommand::Dots(bytes[i]));
                i += 1;
            }
            TX_LOW => commands.push(TextCommand::Low),
            TX_PROMPT_BUTTON => commands.push(TextCommand::PromptButton),
            TX_WAIT_BUTTON => commands.push(TextCommand::WaitButton),
            TX_SCROLL => commands.push(TextCommand::Scroll),
            TX_PAUSE => commands.push(TextCommand::Pause),
            TX_START_ASM => {
                commands.push(TextCommand::Asm(at + i as u16));
                return Ok(());
            }
            TX_FAR => {
                let far = DmgPointer { bank: DmgBank::ROM { bank: bytes[i + 2] }, address: word(i) };
                decode_into(rom_slice(far), far, commands, depth + 1)?;
                i += 3;
            }
            _ => match TextSound::of(command) {
                Some(sound) => commands.push(TextCommand::Sound(sound)),
                None => return Err(format!("{at}: ${command:02X} is not a text command")),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charmap::encode;
    use crate::symbols::pokered_symbols::TEXT_LABELS;

    /// The decoder is total over the cartridge: every far text in it reads as text commands.
    #[test]
    fn every_far_text_in_the_cartridge_decodes() {
        let failures: Vec<String> = TEXT_LABELS
            .iter()
            .filter_map(|&(name, at)| decode(at).err().map(|why| format!("{name}: {why}")))
            .collect();
        assert!(failures.is_empty(), "{} of {}: {failures:#?}", failures.len(), TEXT_LABELS.len());
    }

    #[test]
    fn the_sweep_covers_the_whole_cartridge() {
        assert!(TEXT_LABELS.len() > 2_000, "only {} texts", TEXT_LABELS.len());
    }

    /// A number, a run of letters, and a `<PROMPT>` that ends the script from inside the run.
    #[test]
    fn a_number_and_the_prompt_that_ends_the_script() {
        let mut expected = encode(" EXP. Points!").unwrap();
        expected.push(PROMPT);
        assert_eq!(decode(sym::_ExpPointsText).unwrap(), [
            TextCommand::Number { source: TextNumber::ExpAmountGained, bytes: 2, digits: 4 },
            TextCommand::Text(expected),
        ]);
    }

    /// A buffer, two runs and a number, ended by a `TX_END` of its own rather than from a run.
    #[test]
    fn a_buffer_a_number_and_the_runs_between_them() {
        assert_eq!(decode(sym::_GrewLevelText).unwrap(), [
            TextCommand::Buffer(TextBuffer::NameBuffer),
            TextCommand::Text(encode(" grew<LINE>to level ").unwrap()),
            TextCommand::Number { source: TextNumber::CurEnemyLevel, bytes: 1, digits: 3 },
            TextCommand::Text(encode("!").unwrap()),
        ]);
    }

    /// `<DONE>` ends the script where it stands, and stays in the run for the printer.
    #[test]
    fn done_ends_the_script_from_inside_a_run() {
        let commands = decode(sym::_AntidoteText).unwrap();
        let TextCommand::Text(last) = commands.last().unwrap() else { panic!("{commands:?}") };
        assert_eq!(*last.last().unwrap(), DONE);
    }
}
