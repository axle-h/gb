use std::collections::BTreeMap;
use poke_core::text_script::{TextBuffer, TextMoney, TextNumber};
use serde::{Deserialize, Serialize};
use crate::party::{Named, PartyMon, Pokedex};
use crate::systems::inventory::Inventory;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct World {
    pub options: Options,
    pub events: EventFlags,
    /// Charmap bytes, unterminated.
    pub player_name: Vec<u8>,
    pub rival_name: Vec<u8>,
    /// `wPartySpecies` and the mons beside it. The boxes and the day care belong to the chunk that
    /// first reads them, which is not this one.
    pub party: Vec<Named<PartyMon>>,
    /// `wPokedexOwned` and `wPokedexSeen`.
    pub pokedex: Pokedex,
    /// `wNumBagItems` and `wBagItems`.
    pub bag: Inventory,
    /// `wPlayerMoney`, BCD, two digits to a byte.
    pub money: [u8; 3],
    /// `BIT_NO_TEXT_DELAY`.
    pub no_text_delay: bool,
    pub text: TextVars,
}

/// `NUM_EVENTS`: the event space, most of it unused.
pub const NUM_EVENTS: usize = 0xA00;

/// `wEventFlags`, indexed by the constants `poke_core::symbols::pokered_events` generates.
/// `CheckEvent` reads bit `n % 8` of byte `n / 8`, least significant first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventFlags(Vec<u8>);

impl Default for EventFlags {
    fn default() -> Self {
        Self(vec![0; NUM_EVENTS / 8])
    }
}

impl EventFlags {
    pub fn is_set(&self, event: u16) -> bool {
        self.0[event as usize / 8] & 1 << (event % 8) != 0
    }

    pub fn set(&mut self, event: u16) {
        self.0[event as usize / 8] |= 1 << (event % 8);
    }

    pub fn clear(&mut self, event: u16) {
        self.0[event as usize / 8] &= !(1 << (event % 8));
    }
}

/// What a text command reads. The cartridge points at scratch WRAM a caller filled, and several of
/// those addresses are unions, so what is stored here is the name the text uses rather than a
/// layout. Nothing having filled one is an empty string or a zero, where the cartridge would find
/// whatever the last caller left.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextVars {
    pub strings: BTreeMap<TextBuffer, Vec<u8>>,
    pub numbers: BTreeMap<TextNumber, u32>,
    /// BCD, two digits to a byte, as the cartridge keeps money and coins.
    pub money: BTreeMap<TextMoney, Vec<u8>>,
}

impl TextVars {
    pub fn string(&self, buffer: TextBuffer) -> Vec<u8> {
        self.strings.get(&buffer).cloned().unwrap_or_default()
    }

    pub fn number(&self, source: TextNumber) -> u32 {
        self.numbers.get(&source).copied().unwrap_or_default()
    }

    pub fn bcd(&self, source: TextMoney) -> Vec<u8> {
        self.money.get(&source).cloned().unwrap_or_default()
    }
}

/// `wOptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    pub text_speed: TextSpeed,
    pub battle_animation: bool,
    pub battle_style: BattleStyle,
}

/// `InitOptions`: medium text, animations on, shift.
impl Default for Options {
    fn default() -> Self {
        Self { text_speed: TextSpeed::Medium, battle_animation: true, battle_style: BattleStyle::Shift }
    }
}

/// Frames per printed character: `TEXT_DELAY_FAST`, `_MEDIUM` and `_SLOW`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextSpeed {
    Fast = 1,
    #[default]
    Medium = 3,
    Slow = 5,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BattleStyle {
    #[default]
    Shift,
    Set,
}
