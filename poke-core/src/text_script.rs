//! `TextCommandProcessor`'s bytecode as a typed list, read from the disassembly's text macros.
//!
//! `text_far` is followed and flattened, since it is a static jump the assembler resolved, and a
//! script ends where the cartridge's would. What a command reads from RAM is a named buffer rather
//! than an address, because the WRAM layout is the program's and not the game's.

use serde::{Deserialize, Deserializer, Serialize};
use crate::charmap::encode;
use crate::tables::{TextMacro, TextPredef, TEXTS};

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
    /// `TX_START_ASM`: the label whose code takes the printing over. The chunk that recreates that
    /// routine is the one that gives it a step.
    Asm(#[serde(deserialize_with = "asm_label")] TextLabel),
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

macro_rules! named {
    ($name:ident, $($symbol:literal => $variant:ident),+ $(,)?) => {
        impl $name {
            fn named(symbol: &str) -> Option<Self> {
                match symbol {
                    $($symbol => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

named!(TextBuffer,
    "wStringBuffer" => StringBuffer,
    "wNameBuffer" => NameBuffer,
    "wEnemyMonNick" => EnemyMonNick,
    "wBattleMonNick" => BattleMonNick,
    "wTrainerName" => TrainerName,
    "wOaksAideRewardItemName" => OaksAideRewardItemName,
    "wLearnMoveMonName" => LearnMoveMonName,
    "wGymLeaderName" => GymLeaderName,
    "wGymCityName" => GymCityName,
    "wDayCareMonName" => DayCareMonName,
    "wBoxMonNicks" => BoxMonNicks,
    "wInGameTradeGiveMonName" => InGameTradeGiveMonName,
    "wInGameTradeReceiveMonName" => InGameTradeReceiveMonName,
    "wNameOfPlayerMonToBeTraded" => NameOfPlayerMonToBeTraded,
    "wLinkEnemyTrainerName" => LinkEnemyTrainerName,
    "wBoxNumString" => BoxNumString,
    "wBuffer" => Buffer,
);

named!(TextNumber,
    "hOaksAideRequirement" => OaksAideRequirement,
    "hOaksAideNumMonsOwned" => OaksAideNumMonsOwned,
    "wCurEnemyLevel" => CurEnemyLevel,
    "wPlayerNumHits" => PlayerNumHits,
    "wEnemyNumHits" => EnemyNumHits,
    "wHPBarHPDifference" => HpBarHpDifference,
    "wExpAmountGained" => ExpAmountGained,
    "wDexRatingNumMonsSeen" => DexRatingNumMonsSeen,
    "wDexRatingNumMonsOwned" => DexRatingNumMonsOwned,
    "hDexRatingNumMonsSeen" => DexRatingNumMonsSeenH,
    "hDexRatingNumMonsOwned" => DexRatingNumMonsOwnedH,
    "wDayCareNumLevelsGrown" => DayCareNumLevelsGrown,
    "hTextID" => TextId,
);

named!(TextMoney,
    "hMoney" => Money,
    "hCoins" => Coins,
    "wPlayerCoins" => PlayerCoins,
    "wTotalPayDayMoney" => TotalPayDayMoney,
    "wAmountMoneyWon" => AmountMoneyWon,
    "wDayCareTotalCost" => DayCareTotalCost,
);

named!(TextSound,
    "sound_get_item_1" => GetItem1,
    "sound_pokedex_rating" => PokedexRating,
    "sound_get_item_1_duplicate" => GetItem1Duplicate,
    "sound_get_item_2" => GetItem2,
    "sound_get_key_item" => GetKeyItem,
    "sound_caught_mon" => CaughtMon,
    "sound_dex_page_added" => DexPageAdded,
    "sound_cry_nidorina" => CryNidorina,
    "sound_cry_pidgeot" => CryPidgeot,
    "sound_cry_dewgong" => CryDewgong,
);

impl TextPredef {
    /// The id `PrintPredefTextID` is handed.
    pub fn from_id(id: u8) -> Option<Self> {
        Self::ALL.get((id as usize).checked_sub(1)?).copied()
    }

    pub fn label(self) -> &'static str {
        Self::LABELS[self as usize - 1]
    }
}

/// A `text_far` inside a `text_far`: the cartridge nests one deep, and this is room to spare.
const MAX_DEPTH: usize = 8;

/// The script a label names, global or `Parent.local`, with every `text_far` followed.
pub fn far_text(label: &str) -> Result<Vec<TextCommand>, String> {
    let mut commands = Vec::new();
    flatten(label, &mut commands, 0)?;
    Ok(commands)
}

/// `TEXTS`' own spelling of `label`.
pub fn text_label(label: &str) -> Option<&'static str> {
    TEXTS.binary_search_by(|(name, _)| (*name).cmp(label)).ok().map(|at| TEXTS[at].0)
}

/// Serde for a text held by its label; a save written before holds its address instead.
pub mod saved_text {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use crate::symbols::SavedLabel;

    pub fn serialize<S: Serializer>(label: &Option<&'static str>, serializer: S) -> Result<S::Ok, S::Error> {
        label.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<&'static str>, D::Error> {
        let Some(saved) = Option::<SavedLabel>::deserialize(deserializer)? else { return Ok(None) };
        saved.name().and_then(super::text_label).map(Some)
            .ok_or_else(|| serde::de::Error::custom("a saved text is not in TEXTS"))
    }
}

/// A text a save holds by its label, or by its address in a save written before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedText(pub &'static str);

impl serde::Serialize for SavedText {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0)
    }
}

impl<'de> serde::Deserialize<'de> for SavedText {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        crate::symbols::SavedLabel::resolve(deserializer, text_label).map(Self)
    }
}

fn macros(label: &str) -> Result<&'static [TextMacro], String> {
    TEXTS.binary_search_by(|(name, _)| (*name).cmp(label))
        .map(|at| TEXTS[at].1)
        .map_err(|_| format!("no text is labelled {label}"))
}

fn flatten(label: &str, commands: &mut Vec<TextCommand>, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{label} nests text more than {MAX_DEPTH} deep"));
    }
    let unnamed = |kind: &str, name: &str| format!("{label}: {kind} reads {name}, which has no name here");
    for &command in macros(label)? {
        commands.push(match command {
            TextMacro::Run(text) => TextCommand::Text(encode(text).map_err(|why| format!("{label}: {why}"))?),
            TextMacro::Ram(at) => TextCommand::Buffer(TextBuffer::named(at).ok_or_else(|| unnamed("text_ram", at))?),
            TextMacro::Decimal { at, bytes, digits } => TextCommand::Number {
                source: TextNumber::named(at).ok_or_else(|| unnamed("text_decimal", at))?,
                bytes,
                digits,
            },
            TextMacro::Bcd { at, flags } => TextCommand::Bcd {
                source: TextMoney::named(at).ok_or_else(|| unnamed("text_bcd", at))?,
                bytes: flags & !(MONEY_SIGN | LEFT_ALIGN | LEADING_ZEROES),
                skip_leading_zeroes: flags & LEADING_ZEROES != 0,
                left_align: flags & LEFT_ALIGN != 0,
                money_sign: flags & MONEY_SIGN != 0,
            },
            TextMacro::PromptButton => TextCommand::PromptButton,
            TextMacro::WaitButton => TextCommand::WaitButton,
            TextMacro::Pause => TextCommand::Pause,
            TextMacro::Low => TextCommand::Low,
            TextMacro::Scroll => TextCommand::Scroll,
            TextMacro::Dots(count) => TextCommand::Dots(count),
            TextMacro::Sound(name) => TextCommand::Sound(TextSound::named(name).ok_or_else(|| unnamed("a sound", name))?),
            TextMacro::Asm(holder) => TextCommand::Asm(holder),
            TextMacro::Far(far) => {
                flatten(far, commands, depth + 1)?;
                continue;
            }
        });
    }
    Ok(())
}

/// A label spelled so serde's derive does not borrow it from what it deserializes.
pub type TextLabel = &'static str;

/// A saved `Asm` label, as the one `TEXTS` holds.
fn asm_label<'de, D: Deserializer<'de>>(deserializer: D) -> Result<&'static str, D::Error> {
    let label = String::deserialize(deserializer)?;
    TEXTS.iter().flat_map(|(_, script)| script.iter())
        .find_map(|command| match command {
            TextMacro::Asm(holder) if *holder == label => Some(*holder),
            _ => None,
        })
        .ok_or_else(|| serde::de::Error::custom(format!("no text_asm is under {label}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every text in the source reads as text commands: its runs encode, its operands have names
    /// and its far halves are there.
    #[test]
    fn every_text_in_the_source_reads() {
        let failures: Vec<String> = TEXTS.iter().filter_map(|&(name, _)| far_text(name).err()).collect();
        assert!(failures.is_empty(), "{} of {}: {failures:#?}", failures.len(), TEXTS.len());
        assert!(TEXTS.len() > 5_000, "only {} texts", TEXTS.len());
    }

    /// A number, a run of letters, and a `<PROMPT>` that ends the script from inside the run.
    #[test]
    fn a_number_and_the_prompt_that_ends_the_script() {
        assert_eq!(far_text("_ExpPointsText").unwrap(), [
            TextCommand::Number { source: TextNumber::ExpAmountGained, bytes: 2, digits: 4 },
            TextCommand::Text(encode(" EXP. Points!<PROMPT>").unwrap()),
        ]);
    }

    /// A buffer, two runs and a number, ended by a `text_end` of its own rather than from a run.
    #[test]
    fn a_buffer_a_number_and_the_runs_between_them() {
        assert_eq!(far_text("_GrewLevelText").unwrap(), [
            TextCommand::Buffer(TextBuffer::NameBuffer),
            TextCommand::Text(encode(" grew<LINE>to level ").unwrap()),
            TextCommand::Number { source: TextNumber::CurEnemyLevel, bytes: 1, digits: 3 },
            TextCommand::Text(encode("!").unwrap()),
        ]);
    }

    /// `<DONE>` ends the script where it stands, and stays in the run for the printer.
    #[test]
    fn done_ends_the_script_from_inside_a_run() {
        let commands = far_text("_AntidoteText").unwrap();
        let TextCommand::Text(last) = commands.last().unwrap() else { panic!("{commands:?}") };
        assert_eq!(last.last(), encode("<DONE>").unwrap().last());
    }

    /// A `text_far` is followed, and a `text_asm` names the label whose code it hands over to.
    #[test]
    fn a_far_text_is_flattened_up_to_its_asm() {
        let commands = far_text("OneTwoAndText").unwrap();
        assert_eq!(commands[..commands.len() - 2], far_text("_OneTwoAndText").unwrap()[..]);
        assert_eq!(commands[commands.len() - 2..], [TextCommand::Pause, TextCommand::Asm("OneTwoAndText")]);
    }

    #[test]
    fn a_saved_asm_label_loads_as_the_one_in_the_table() {
        let saved = serde_json::to_string(&TextCommand::Asm("OneTwoAndText")).unwrap();
        assert_eq!(serde_json::from_str::<TextCommand>(&saved).unwrap(), TextCommand::Asm("OneTwoAndText"));
        assert!(serde_json::from_str::<TextCommand>(&saved.replace("OneTwo", "Nothing")).is_err());
    }
}
