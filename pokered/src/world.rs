use std::collections::BTreeMap;
use poke_core::item::ItemId;
use poke_core::species::PokemonSpecies;
use poke_core::text_script::{TextBuffer, TextMoney, TextNumber};
use serde::{Deserialize, Serialize};
use crate::party::{BoxMon, Named, PartyMon, Pokedex};
use crate::systems::hall_of_fame::HallOfFameMon;
use crate::systems::inventory::Inventory;
use crate::systems::overworld::Location;
use crate::systems::play_time::PlayTime;

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
    /// `wObtainedBadges`, bit 0 the Boulder Badge.
    #[serde(default)]
    pub badges: u8,
    #[serde(default)]
    pub play_time: PlayTime,
    /// `wPlayerID`: a party mon with another OT gets boosted experience and may disobey.
    #[serde(default)]
    pub player_id: u16,
    /// `sBox1`..`sBox12`: every PC box, each up to `MONS_PER_BOX` mons with their OT and nickname,
    /// newest first. `wBoxCount`, `wBoxMons`, `wBoxMonOT` and `wBoxMonNicks` in WRAM are the
    /// cartridge's working copy of `boxes[current_box]`, written back to its SRAM box when the box
    /// is changed; a box missing from the list is an empty one.
    #[serde(default)]
    pub boxes: Vec<Vec<Named<BoxMon>>>,
    /// `wCurrentBoxNum` without its `BIT_HAS_CHANGED_BOXES`, counting from 0.
    #[serde(default)]
    pub current_box: u8,
    /// `wNumSafariBalls`.
    #[serde(default)]
    pub safari_balls: u8,
    /// `wBoxItems`: the items in the player's PC. A new game makes it with `Inventory::pc`.
    #[serde(default = "Inventory::default_pc")]
    pub pc_items: Inventory,
    /// `wNumHoFTeams`, which stops at 255 rather than wrapping.
    #[serde(default)]
    pub hall_of_fame_teams: u8,
    /// `sHallOfFame`: the teams recorded, oldest first, at most `HOF_TEAM_CAPACITY`.
    #[serde(default)]
    pub hall_of_fame: Vec<Vec<HallOfFameMon>>,
    /// `wLetterPrintingDelayFlags` with `BIT_FAST_TEXT_DELAY` clear: a printed letter waits one frame
    /// whatever the text speed. The Hall of Fame clears the bit; `InitOptions` sets it.
    #[serde(default)]
    pub one_frame_letter_delay: bool,
    /// `wPlayerCoins`, BCD, two digits to a byte.
    #[serde(default)]
    pub coins: [u8; 2],
    /// `wDayCareInUse`, with `wDayCareMonName`, `wDayCareMonOT` and `wDayCareMon`.
    #[serde(default)]
    pub day_care: Option<Named<BoxMon>>,
    /// `wObtainedHiddenItemsFlags`: a bit per entry of `HiddenItemCoords`.
    #[serde(default)]
    pub hidden_items: [u8; 14],
    /// `wObtainedHiddenCoinsFlags`: a bit per entry of `HiddenCoinCoords`.
    #[serde(default)]
    pub hidden_coins: [u8; 2],
    /// `wCompletedInGameTradeFlags`: a bit per `TRADE_FOR_*`.
    #[serde(default)]
    pub in_game_trades: u16,
    /// `wStatusFlags4`'s `BIT_USED_POKECENTER`: the nurse has asked once and skips the question after.
    #[serde(default)]
    pub used_pokecenter: bool,
    /// `wSafariSteps`.
    #[serde(default)]
    pub safari_steps: u16,
    /// `wFossilItem` and `wFossilMon`: what the Cinnabar lab was given, and what it revives.
    #[serde(default)]
    pub fossil: Option<(ItemId, PokemonSpecies)>,
    /// Every map's `w<Map>CurScript` and the script variables that outlive a pass.
    #[serde(default)]
    pub scripts: crate::scripts::ScriptState,
    /// `BIT_NO_TEXT_DELAY`.
    pub no_text_delay: bool,
    pub text: TextVars,
    /// Where the player stands: the map, the square, the facing, and the map state a save keeps.
    pub location: Location,
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

    /// `wEventFlags` as the cartridge lays it out, for a reader that works in bytes and masks
    /// rather than in event numbers.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
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
