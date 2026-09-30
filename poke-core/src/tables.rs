//! Tables read out of the disassembly's `data/` by `build.rs`, in the form they are written there
//! rather than the bytes they assemble to. `poke-agent`'s `rom_equality` checks each against the
//! cartridge.

/// One `GrowthRateTable` row: `numerator/denominator·n³ + squared·n² + linear·n − constant`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrowthRate {
    pub numerator: u8,
    pub denominator: u8,
    pub squared: i8,
    pub linear: u8,
    pub constant: u8,
}

/// One `Moves` row. `accuracy` is the percentage written there; the cartridge stores it out of 255.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Move {
    pub animation: u8,
    pub effect: u8,
    pub power: u8,
    pub move_type: u8,
    pub accuracy: u8,
    pub pp: u8,
}

/// One `TradeMons` row: the species given, the one received, the dialogue set and its nickname.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trade {
    pub give: u8,
    pub get: u8,
    pub dialog: u8,
    pub nickname: &'static str,
}

/// One species' base data, as its file under `data/pokemon/base_stats/` writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaseStats {
    pub dex: u8,
    /// HP, attack, defense, speed, special.
    pub stats: [u8; 5],
    pub types: [u8; 2],
    pub catch_rate: u8,
    pub base_exp: u8,
    pub level_1_moves: [u8; 4],
    pub growth_rate: u8,
    /// The moves `tmhm` lists, which the cartridge packs into a bit per machine.
    pub tm_hm: &'static [u8],
}

/// How a species evolves: `EVOLVE_LEVEL`, `EVOLVE_ITEM` or `EVOLVE_TRADE` and its arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evolution {
    Level { level: u8, into: u8 },
    Item { item: u8, min_level: u8, into: u8 },
    Trade { min_level: u8, into: u8 },
}

/// One species' evolutions and its level-up learnset, `(level, move)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvosMoves {
    pub evolutions: &'static [Evolution],
    pub learnset: &'static [(u8, u8)],
}

/// One map's wild lists, `(level, species)` a slot. A rate of 0 has an empty list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WildData {
    pub grass_rate: u8,
    pub grass: &'static [(u8, u8)],
    pub water_rate: u8,
    pub water: &'static [(u8, u8)],
}

/// One trainer's party, `(level, species)` a mon. Only a party written with a level per mon can be
/// given a special move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrainerParty {
    pub per_mon_levels: bool,
    pub mons: &'static [(u8, u8)],
}

/// One `connection`: the neighbour on the side `direction` names (`NORTH` ... `EAST`), and how many
/// blocks along that side it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Connection {
    pub direction: u8,
    pub map: u8,
    pub offset: i8,
}

/// One `warp_event`: `(x, y)` leads to warp `warp` of `map`, counted from 1 as written there, and
/// `LAST_MAP` is `0xFF`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WarpEvent {
    pub x: u8,
    pub y: u8,
    pub map: u8,
    pub warp: u8,
}

/// One `bg_event`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BgEvent {
    pub x: u8,
    pub y: u8,
    pub text_id: u8,
}

/// One `object_event`, its coordinates as written there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectEvent {
    pub x: u8,
    pub y: u8,
    pub sprite: u8,
    pub movement: u8,
    pub range_or_direction: u8,
    pub text_id: u8,
    pub kind: crate::map_objects::ObjectKind,
}

/// The events of a map's `_Object` label, up to its `def_warps_to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapObjects {
    pub border_block: u8,
    pub warps: &'static [WarpEvent],
    pub signs: &'static [BgEvent],
    pub objects: &'static [ObjectEvent],
}

/// One `map_header`, its connections in the order written, and the blocks and objects its labels
/// name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapHeader {
    pub name: &'static str,
    pub tileset: u8,
    pub height: u8,
    pub width: u8,
    pub connections: &'static [Connection],
    pub blocks: &'static [u8],
    pub objects: MapObjects,
}

/// One `Tilesets` row and the walkable tiles its `_Coll` label lists. A tile id of `0xFF` is none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tileset {
    pub name: &'static str,
    /// The tiles a person can be talked to across.
    pub counter_tiles: [u8; 3],
    pub grass_tile: u8,
    /// `TILEANIM_*`.
    pub animation: u8,
    pub collision: &'static [u8],
}

/// One text macro as a script in `text/`, `data/text/` or beside the code that prints it writes it.
/// The operands are the names the source writes; `text_script` resolves them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextMacro {
    /// `text`, then `line`, `cont`, `para`, `next` and `page` as their control codes: a printed run
    /// in charmap spelling, ended by an `@` (left out) or by the `<DONE>`, `<PROMPT>` or `<DEXEND>`
    /// that ends the script (kept).
    Run(&'static str),
    Ram(&'static str),
    Decimal { at: &'static str, bytes: u8, digits: u8 },
    /// `text_bcd`'s second argument: the byte count and the print flags.
    Bcd { at: &'static str, flags: u8 },
    PromptButton,
    WaitButton,
    Pause,
    Low,
    Scroll,
    Dots(u8),
    /// A `sound_*` macro, by name.
    Sound(&'static str),
    Far(&'static str),
    /// `text_asm`, naming the label whose code takes over. It ends the script.
    Asm(&'static str),
}

/// A `script_*` text: not a script but the `TX_SCRIPT_*` byte `DisplayTextID` dispatches on, and a
/// mart's stock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDispatch {
    PokecenterNurse,
    Mart(&'static [u8]),
    BillsPc,
    PlayersPc,
    PokecenterPc,
    PrizeVendor,
    CableClubReceptionist,
    VendingMachine,
}

/// One `trainer` row: its label, the bit `def_trainers` counts to it (also its sprite's index), the
/// `wEventFlags` bit beating it sets, its sight range in squares and its texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trainer {
    pub label: &'static str,
    pub sprite: u8,
    pub event: u16,
    pub range: u8,
    pub before_battle: &'static str,
    pub end_battle: &'static str,
    pub after_battle: &'static str,
}

/// One step of a `BattleTransition_HalfCircle*`: from `(x, y)`, runs of blackened cells rightward
/// or leftward, each run's count followed by how far back the next row starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HalfCircleStep {
    pub right: bool,
    pub runs: &'static [u8],
    pub x: u8,
    pub y: u8,
}

/// A piece of a `db` string: text in charmap spelling, or a byte written as a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chars {
    Text(&'static str),
    Byte(u8),
}

impl Chars {
    /// `pieces` as the bytes the cartridge holds.
    pub fn encode(pieces: &[Chars]) -> Vec<u8> {
        pieces.iter().flat_map(|piece| match piece {
            Chars::Text(text) => crate::charmap::encode(text).unwrap_or_else(|e| panic!("{e}")),
            Chars::Byte(byte) => vec![*byte],
        }).collect()
    }
}

/// One Pokedex entry: the species line, the height, the weight in tenths of a pound and the
/// description's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DexEntry {
    pub species: &'static str,
    pub feet: u8,
    pub inches: u8,
    pub weight: u16,
    pub text: &'static str,
}

/// One `hidden_event` or `hidden_text_predef`: the square, the routine, and the byte it is handed,
/// a text predef's id for the second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HiddenEventRow {
    pub x: u8,
    pub y: u8,
    pub routine: HiddenRoutine,
    pub argument: u8,
}

// `WAVE_POINTERS[i]` indexes `WAVE_SAMPLES`, bar the pointers past its end, which name a label
// with no data under it. `GYM_TRASH_CANS` is `(mask, neighbours)`; the tower's rows are
// `((x, y), movement)`, the movement with its own `$FF`. The Pewter guides' squares are `((x, y),
// presses)` and the arrows' `((x, y), (press, count) runs)`; a prize window is `(prizes, costs in
// coins)`, and a Pokédex rating `(owned below, text)`.
// `BOOKSHELF_TILE_IDS` is `(tileset, tile, text)` and `BENCH_GUY_TEXTS` `(map, facing, text)`;
// `HIDDEN_EVENTS` is `HiddenEventMaps`' order, each map's rows beside it. `HIDDEN_ITEM_COORDS` and
// `HIDDEN_COIN_COORDS` are `(map, x, y)`, `CINNABAR_GYM_GATE_COORDS` `(x, y, block)`, the cut and dust
// offsets `(dx, dy)` and the fly coordinates `(y, x)`; a credit is `(column offset, text)`.
use crate::battle_anims::AnimCommand;

include!(concat!(env!("OUT_DIR"), "/tables.rs"));

/// The `db` string at `label`, as the bytes the cartridge holds.
pub fn db_string(label: &str) -> Vec<u8> {
    let at = DB_STRINGS.binary_search_by(|(name, _)| (*name).cmp(label)).unwrap_or_else(|_| panic!("no db string is labelled {label}"));
    Chars::encode(DB_STRINGS[at].1)
}

impl HiddenRoutine {
    pub fn named(label: &str) -> Option<Self> {
        Self::LABELS.iter().position(|&name| name == label).map(|at| Self::ALL[at])
    }

    pub fn label(self) -> &'static str {
        Self::LABELS[self as usize]
    }
}

/// Saved by its label; a save written before names holds the routine's address.
impl serde::Serialize for HiddenRoutine {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.label())
    }
}

impl<'de> serde::Deserialize<'de> for HiddenRoutine {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        crate::symbols::SavedLabel::resolve(deserializer, Self::named)
    }
}
