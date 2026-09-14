//! The SGB palette layer: `engine/gfx/palettes.asm`'s `_RunPaletteCommand` and every `SetPal_*`,
//! over the packets in `data/sgb/{sgb_packets,sgb_palettes}.asm`.
//!
//! Inputs: a [`PaletteCommand`], which is the command byte `_RunPaletteCommand` takes with the
//! globals each `SetPal_*` reads passed as arguments instead. Output: [`SgbState`], the four
//! `SuperPalettes` a `PAL_SET` chose and a palette per screen cell, which [`crate::gfx::colour`]
//! paints the composed frame with.
//!
//! Exact: which palette every screen, map and species gets; the `ATTR_BLK` rectangles and the
//! `PAL_SET` ids, both read out of the cartridge at their own labels; the trainer card's blanking
//! of a badge not won and the party menu's per-bar patching.
//!
//! Not emulated: the protocol. `SendSGBPacket` bit-bangs `rJOYP` at a console this does not model,
//! so a packet is decoded where it would have been sent.
//!
//! What the SGB colours is the *screen* rather than the game's layers. The console sees only the
//! finished two-bit picture the DMG would have shown, so an attribute covers an 8x8 screen cell
//! and a sprite is painted in the colours of the cell it stands in.

pub mod border;
mod packets;

use poke_core::map::Map;
use poke_core::map_header::TileSetId;
use poke_core::rom_gfx::rom_slice;
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_symbols;
use serde::{Deserialize, Serialize};
use crate::gfx::ui::{SCREEN_TILES_X, SCREEN_TILES_Y};
use crate::systems::hp_bar::HpBarColour;

/// Screen cells, which is how many attributes there are.
pub const CELLS: usize = SCREEN_TILES_X * SCREEN_TILES_Y;

/// `SuperPalettes` indices: the `PAL_*` constants, in their own order.
pub const PAL_ROUTE: u8 = 0x00;
pub const PAL_PALLET: u8 = 0x01;
pub const PAL_GRAYMON: u8 = 0x19;
pub const PAL_BLACK: u8 = 0x1E;
pub const PAL_GREENBAR: u8 = 0x1F;
pub const PAL_CAVE: u8 = 0x23;
pub const NUM_SGB_PALS: u8 = 0x25;

/// `NUM_CITY_MAPS`, `FIRST_INDOOR_MAP`: the map ids `SetPal_Overworld` divides the world on.
const NUM_CITY_MAPS: u8 = 0x0B;
const FIRST_INDOOR_MAP: u8 = 0x25;

/// `wPartyMenuHPBarColors` has one of these a mon, and the party menu's blk packet a block.
const PARTY_MENU_HP_BARS: usize = 6;

/// What a screen asks for, as `_RunPaletteCommand`'s `b` plus the globals the chosen `SetPal_*`
/// would have read. `SET_PAL_*` in the source order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaletteCommand {
    BattleBlack,
    Battle { player_hp_bar: HpBarColour, enemy_hp_bar: HpBarColour, player: u8, enemy: u8 },
    TownMap,
    StatusScreen { hp_bar: HpBarColour, mon: u8 },
    Pokedex { mon: u8 },
    Slots,
    TitleScreen,
    NidorinoIntro,
    Generic,
    Overworld(OverworldPalette),
    PartyMenu,
    PokemonWholeScreen { mon: u8, black: bool },
    GameFreakIntro,
    TrainerCard { badges: u8 },
    /// `SET_PAL_PARTY_MENU_HP_BARS`, which is not a palette function: it patches one bar's block
    /// of the party menu's own packet and sends nothing.
    PartyMenuHpBars { which: usize, colour: HpBarColour },
    /// `SET_PAL_DEFAULT`: whatever last claimed `wDefaultPaletteCommand`.
    Default,
}

/// What `SetPal_Overworld` reads: `wCurMap`, `wCurMapTileset` and `wLastMap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverworldPalette {
    #[serde(with = "map_id")]
    pub map: Map,
    pub tileset: TileSetId,
    #[serde(with = "map_id")]
    pub last_map: Map,
}

/// The console's side: the palettes a `PAL_SET` chose and the attribute file the `ATTR_BLK`s
/// wrote. Both outlive the packet that set them, which is what lets the trainer card colour ten
/// badge rectangles and leave the rest of the screen as the menu before it left it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SgbState {
    /// `SuperPalettes` ids for SGB palettes 0 to 3.
    palettes: [u8; 4],
    /// One of those four per screen cell, row by row.
    attributes: Vec<u8>,
    /// `wPartyMenuBlkPacket`, whose HP bar blocks are patched a bar at a time.
    party_menu: Vec<u8>,
    /// `wDefaultPaletteCommand`. A zeroed byte is `SET_PAL_BATTLE_BLACK`.
    default: Box<PaletteCommand>,
}

impl Default for SgbState {
    fn default() -> Self {
        Self {
            palettes: [0; 4],
            attributes: vec![0; CELLS],
            party_menu: packets::transfer(pokered_symbols::BlkPacket_PartyMenu),
            default: Box::new(PaletteCommand::BattleBlack),
        }
    }
}

impl SgbState {
    /// `_RunPaletteCommand` and the `SendSGBPackets` it pushes a return to.
    pub fn run(&mut self, command: &PaletteCommand) {
        let command = match command {
            PaletteCommand::PartyMenuHpBars { which, colour } => {
                return self.update_party_menu_blk_packet(*which, *colour);
            }
            PaletteCommand::Default => (*self.default).clone(),
            other => other.clone(),
        };
        // `wDefaultPaletteCommand` is claimed by three of the functions, and the intro claims it
        // for the generic one rather than for itself.
        match &command {
            PaletteCommand::Battle { .. } | PaletteCommand::Overworld(_) => {
                self.default = Box::new(command.clone());
            }
            PaletteCommand::GameFreakIntro => self.default = Box::new(PaletteCommand::Generic),
            _ => {}
        }
        let (pal, blk) = self.set_pal(&command);
        self.palettes = packets::pal_set(&pal);
        for data in packets::attr_blk(&blk) {
            data.apply(&mut self.attributes);
        }
    }

    /// The palette a screen cell is painted with, as four RGB555 colours.
    pub fn cell_palette(&self, column: usize, row: usize) -> [u16; 4] {
        super_palette(self.palettes[self.attributes[row * SCREEN_TILES_X + column] as usize])
    }

    /// Which of the four palettes a cell uses.
    pub fn attribute(&self, column: usize, row: usize) -> u8 {
        self.attributes[row * SCREEN_TILES_X + column]
    }

    /// The `SuperPalettes` ids in force, for SGB palettes 0 to 3.
    pub fn palette_ids(&self) -> [u8; 4] {
        self.palettes
    }

    /// Each `SetPal_*`: the pal packet it leaves in `hl` and the blk packet it leaves in `de`.
    fn set_pal(&self, command: &PaletteCommand) -> (Vec<u8>, Vec<u8>) {
        let transfer = packets::transfer;
        match command {
            PaletteCommand::BattleBlack => (
                transfer(pokered_symbols::PalPacket_Black),
                transfer(pokered_symbols::BlkPacket_Battle),
            ),
            PaletteCommand::Battle { player_hp_bar, enemy_hp_bar, player, enemy } => {
                let mut pal = transfer(pokered_symbols::PalPacket_Empty);
                pal[1] = PAL_GREENBAR + *player_hp_bar as u8;
                pal[3] = PAL_GREENBAR + *enemy_hp_bar as u8;
                pal[5] = *player;
                pal[7] = *enemy;
                (pal, transfer(pokered_symbols::BlkPacket_Battle))
            }
            PaletteCommand::TownMap => (
                transfer(pokered_symbols::PalPacket_TownMap),
                transfer(pokered_symbols::BlkPacket_WholeScreen),
            ),
            PaletteCommand::StatusScreen { hp_bar, mon } => {
                let mut pal = transfer(pokered_symbols::PalPacket_Empty);
                pal[1] = PAL_GREENBAR + *hp_bar as u8;
                pal[3] = *mon;
                (pal, transfer(pokered_symbols::BlkPacket_StatusScreen))
            }
            PaletteCommand::Pokedex { mon } => {
                let mut pal = transfer(pokered_symbols::PalPacket_Pokedex);
                pal[3] = *mon;
                (pal, transfer(pokered_symbols::BlkPacket_Pokedex))
            }
            PaletteCommand::Slots => (
                transfer(pokered_symbols::PalPacket_Slots),
                transfer(pokered_symbols::BlkPacket_Slots),
            ),
            PaletteCommand::TitleScreen => (
                transfer(pokered_symbols::PalPacket_Titlescreen),
                transfer(pokered_symbols::BlkPacket_Titlescreen),
            ),
            PaletteCommand::NidorinoIntro => (
                transfer(pokered_symbols::PalPacket_NidorinoIntro),
                transfer(pokered_symbols::BlkPacket_NidorinoIntro),
            ),
            PaletteCommand::Generic => (
                transfer(pokered_symbols::PalPacket_Generic),
                transfer(pokered_symbols::BlkPacket_WholeScreen),
            ),
            PaletteCommand::Overworld(overworld) => {
                let mut pal = transfer(pokered_symbols::PalPacket_Empty);
                pal[1] = overworld_palette(*overworld);
                (pal, transfer(pokered_symbols::BlkPacket_WholeScreen))
            }
            PaletteCommand::PartyMenu => (
                transfer(pokered_symbols::PalPacket_PartyMenu),
                self.party_menu.clone(),
            ),
            PaletteCommand::PokemonWholeScreen { mon, black } => {
                let mut pal = transfer(pokered_symbols::PalPacket_Empty);
                pal[1] = if *black { PAL_BLACK } else { *mon };
                (pal, transfer(pokered_symbols::BlkPacket_WholeScreen))
            }
            PaletteCommand::GameFreakIntro => (
                transfer(pokered_symbols::PalPacket_GameFreakIntro),
                transfer(pokered_symbols::BlkPacket_GameFreakIntro),
            ),
            PaletteCommand::TrainerCard { badges } => (
                transfer(pokered_symbols::PalPacket_TrainerCard),
                trainer_card_blk_packet(*badges),
            ),
            PaletteCommand::PartyMenuHpBars { .. } | PaletteCommand::Default => {
                unreachable!("neither reaches SetPalFunctions")
            }
        }
    }

    /// `UpdatePartyMenuBlkPacket`. Green, yellow and red are palettes 1, 2 and 3 of the party
    /// menu's own `PAL_SET`, and each is written as both the inside and the line colour.
    fn update_party_menu_blk_packet(&mut self, which: usize, colour: HpBarColour) {
        assert!(which < PARTY_MENU_HP_BARS, "the party menu has {PARTY_MENU_HP_BARS} bars, not {which}");
        let palette = colour as u8 + 1;
        self.party_menu[8 + 1 + 6 * which] = (palette << 2) | palette;
    }
}

/// `SetPal_Overworld`: a town is its own map id plus one, a route is `PAL_ROUTE`, and an indoor
/// map takes the palette of the town or route it is in (`wLastMap`). The caves and Agatha's and
/// Bruno's rooms are the exceptions, and Lorelei's room is Pallet Town's blue.
pub fn overworld_palette(overworld: OverworldPalette) -> u8 {
    let OverworldPalette { map, tileset, last_map } = overworld;
    // `ld a, PAL_ROUTE - 1` assembles to `$FF`, and the `inc a` that every path ends on wraps it
    // back to `PAL_ROUTE`.
    let town_or_route = |map: u8| if map < NUM_CITY_MAPS { map } else { PAL_ROUTE.wrapping_sub(1) };
    let id = match tileset {
        TileSetId::Cemetery => PAL_GRAYMON - 1,
        TileSetId::Cavern => PAL_CAVE - 1,
        _ => match map as u8 {
            id if id < FIRST_INDOOR_MAP => town_or_route(id),
            id if id < Map::CeruleanCave2F as u8 => town_or_route(last_map as u8),
            id if id <= Map::CeruleanCave1F as u8 => PAL_CAVE - 1,
            id if id == Map::LoreleisRoom as u8 => 0,
            id if id == Map::BrunosRoom as u8 => PAL_CAVE - 1,
            _ => town_or_route(last_map as u8),
        },
    };
    id.wrapping_add(1)
}

/// `DeterminePaletteID`: a mon that has used Transform is Ditto's grey whatever it looks like.
pub fn determine_palette_id(transformed: bool, species: u8) -> u8 {
    if transformed { PAL_GRAYMON } else { determine_palette_id_out_of_battle(species) }
}

/// `DeterminePaletteIDOutOfBattle`, over `MonsterPalettes`, which is indexed by Pokédex number
/// rather than by species index. Index 0 skips the conversion and lands on the table's first
/// entry, and so does every index no Pokémon has, because `PokedexOrder` answers 0 for those.
pub fn determine_palette_id_out_of_battle(species: u8) -> u8 {
    let dex = PokemonSpecies::from_repr(species).map_or(0, |species| species.metadata().pokedex_number);
    rom_slice(pokered_symbols::MonsterPalettes)[dex as usize]
}

/// One `SuperPalettes` entry, four RGB555 colours. Colour 0 is `31,29,31` in every entry, which is
/// why nothing here models the SGB sharing colour 0 across its four palettes.
pub fn super_palette(id: u8) -> [u16; 4] {
    assert!(id < NUM_SGB_PALS, "{id} is not a SuperPalettes entry");
    let bytes = rom_slice(pokered_symbols::SuperPalettes + id as u16 * 8);
    std::array::from_fn(|i| u16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]))
}

/// `SetPal_TrainerCard`: the card's packet with the blk data of every badge not won zeroed, which
/// leaves the block counted and colouring nothing. The Rainbow Badge has three blocks to itself.
fn trainer_card_blk_packet(badges: u8) -> Vec<u8> {
    /// `BadgeBlkDataLengths`.
    const BADGE_BLK_DATA_LENGTHS: [usize; 8] = [6, 6, 6, 6 * 3, 6, 6, 6, 6];
    let mut packet = packets::transfer(pokered_symbols::BlkPacket_TrainerCard);
    let mut at = 2;
    for (badge, length) in BADGE_BLK_DATA_LENGTHS.iter().enumerate() {
        if badges & (1 << badge) == 0 {
            packet[at..at + length].fill(0);
        }
        at += length;
    }
    packet
}

/// A saved command holds a map by its cartridge id: `Map` carries no serde of its own, and giving
/// it any belongs to whichever chunk first needs one in a fixture.
mod map_id {
    use poke_core::map::Map;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(map: &Map, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(*map as u8)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Map, D::Error> {
        let id = u8::deserialize(deserializer)?;
        Map::from_repr(id).ok_or_else(|| serde::de::Error::custom(format!("no map {id:#04X}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overworld(map: Map, last_map: Map) -> u8 {
        overworld_palette(OverworldPalette { map, tileset: TileSetId::Overworld, last_map })
    }

    #[test]
    fn a_town_takes_its_own_palette_and_a_route_the_one_route_palette() {
        assert_eq!(overworld(Map::PalletTown, Map::PalletTown), PAL_PALLET);
        assert_eq!(overworld(Map::SaffronCity, Map::SaffronCity), 0x0B, "PAL_SAFFRON");
        assert_eq!(overworld(Map::Route1, Map::PalletTown), PAL_ROUTE);
        assert_eq!(overworld(Map::UnusedMap0B, Map::PalletTown), PAL_ROUTE);
    }

    #[test]
    fn a_building_takes_the_palette_of_the_map_it_is_in() {
        assert_eq!(overworld(Map::RedsHouse1F, Map::PalletTown), PAL_PALLET);
        assert_eq!(overworld(Map::CeladonMart1F, Map::CeladonCity), 0x07, "PAL_CELADON");
        assert_eq!(overworld(Map::ViridianForest, Map::Route2), PAL_ROUTE);
    }

    #[test]
    fn the_caves_and_the_elite_four_are_the_exceptions() {
        let cave = |map, last_map| overworld_palette(OverworldPalette { map, tileset: TileSetId::Cavern, last_map });
        assert_eq!(cave(Map::MtMoon1F, Map::Route4), PAL_CAVE);
        assert_eq!(overworld_palette(OverworldPalette {
            map: Map::PokemonTower2F, tileset: TileSetId::Cemetery, last_map: Map::LavenderTown,
        }), PAL_GRAYMON);
        assert_eq!(overworld(Map::CeruleanCave2F, Map::CeruleanCity), PAL_CAVE);
        assert_eq!(overworld(Map::CeruleanCave1F, Map::CeruleanCity), PAL_CAVE);
        assert_eq!(overworld(Map::BrunosRoom, Map::IndigoPlateau), PAL_CAVE);
        assert_eq!(overworld(Map::LoreleisRoom, Map::IndigoPlateau), PAL_PALLET);
        assert_eq!(overworld(Map::AgathasRoom, Map::IndigoPlateau), 0x0A, "PAL_INDIGO, by way of wLastMap");
    }

    #[test]
    fn a_species_takes_its_own_colour_and_a_transformed_one_takes_dittos() {
        assert_eq!(determine_palette_id_out_of_battle(PokemonSpecies::Bulbasaur as u8), 0x16, "PAL_GREENMON");
        assert_eq!(determine_palette_id_out_of_battle(PokemonSpecies::Charmander as u8), 0x12, "PAL_REDMON");
        assert_eq!(determine_palette_id_out_of_battle(PokemonSpecies::Squirtle as u8), 0x13, "PAL_CYANMON");
        assert_eq!(determine_palette_id_out_of_battle(0), 0x10, "MISSINGNO's entry, PAL_MEWMON");
        assert_eq!(determine_palette_id(true, PokemonSpecies::Squirtle as u8), PAL_GRAYMON);
    }

    /// `SetPal_StatusScreen` asks for index 1 when what is on screen is not a Pokémon.
    #[test]
    fn the_not_a_pokemon_index_lands_on_rhydons_colour() {
        assert_eq!(determine_palette_id_out_of_battle(1), determine_palette_id_out_of_battle(PokemonSpecies::Rhydon as u8));
    }

    #[test]
    fn every_palette_shares_its_first_colour() {
        let white = super_palette(PAL_ROUTE)[0];
        for id in 0..NUM_SGB_PALS {
            assert_eq!(super_palette(id)[0], white, "palette {id:#04X}");
        }
    }

    #[test]
    fn the_generic_screen_is_one_palette_everywhere() {
        let mut sgb = SgbState::default();
        sgb.run(&PaletteCommand::Generic);
        assert_eq!(sgb.palette_ids()[0], 0x10, "PAL_MEWMON");
        for row in 0..SCREEN_TILES_Y {
            for column in 0..SCREEN_TILES_X {
                assert_eq!(sgb.attribute(column, row), 0, "({column}, {row})");
            }
        }
    }

    /// `BlkPacket_Battle`'s five rectangles, and the ids `SetPal_Battle` patches into an empty
    /// packet for them.
    #[test]
    fn a_battle_colours_the_two_mons_the_two_bars_and_the_message_box() {
        let mut sgb = SgbState::default();
        sgb.run(&PaletteCommand::Battle {
            player_hp_bar: HpBarColour::Green,
            enemy_hp_bar: HpBarColour::Red,
            player: determine_palette_id_out_of_battle(PokemonSpecies::Squirtle as u8),
            enemy: determine_palette_id_out_of_battle(PokemonSpecies::Charmander as u8),
        });
        assert_eq!(sgb.palette_ids(), [PAL_GREENBAR, PAL_GREENBAR + 2, 0x13, 0x12]);
        assert_eq!(sgb.attribute(0, 12), 2, "the message box");
        assert_eq!(sgb.attribute(1, 0), 1, "the enemy's HP bar");
        assert_eq!(sgb.attribute(10, 7), 0, "the player's HP bar");
        assert_eq!(sgb.attribute(4, 8), 2, "the player's mon");
        assert_eq!(sgb.attribute(15, 3), 3, "the enemy's mon");
    }

    /// The attribute file outlives the packet that wrote it, which is the whole reason the trainer
    /// card can name ten rectangles and nothing else.
    #[test]
    fn the_trainer_card_only_colours_the_badges_it_names() {
        let mut sgb = SgbState::default();
        sgb.run(&PaletteCommand::Generic);
        sgb.run(&PaletteCommand::TrainerCard { badges: 0b0000_0011 });
        assert_eq!(sgb.attribute(3, 12), 0, "the Boulder Badge");
        assert_eq!(sgb.attribute(7, 12), 1, "the Cascade Badge");
        assert_eq!(sgb.attribute(11, 12), 0, "the Thunder Badge, not won, so still the screen's own");
        assert_eq!(sgb.attribute(19, 17), 0, "and so is everything the card does not name");
    }

    #[test]
    fn a_party_menu_bar_is_patched_into_the_menus_own_packet() {
        let mut sgb = SgbState::default();
        sgb.run(&PaletteCommand::PartyMenuHpBars { which: 2, colour: HpBarColour::Red });
        sgb.run(&PaletteCommand::PartyMenu);
        assert_eq!(sgb.attribute(5, 5), 3, "the third bar, on red");
        assert_eq!(sgb.attribute(5, 1), 0, "the first, which nothing has patched");
        assert_eq!(sgb.attribute(19, 17), 1, "everything outside the sprite column");
    }

    /// The default command is a kind rather than a screen: the overworld claims it and a later
    /// `SET_PAL_DEFAULT` puts the same map's palette back.
    #[test]
    fn the_default_command_is_whatever_last_claimed_it() {
        let mut sgb = SgbState::default();
        let pallet = OverworldPalette { map: Map::PalletTown, tileset: TileSetId::Overworld, last_map: Map::PalletTown };
        sgb.run(&PaletteCommand::Overworld(pallet));
        sgb.run(&PaletteCommand::BattleBlack);
        assert_eq!(sgb.palette_ids()[0], PAL_BLACK);
        sgb.run(&PaletteCommand::Default);
        assert_eq!(sgb.palette_ids()[0], PAL_PALLET);
    }

    /// Every map the cartridge has, against the palette `SetPal_Overworld` chose for it.
    #[test]
    fn every_map_takes_the_palette_the_cartridge_gives_it() {
        let cases = crate::fixtures::cases::<OverworldPalette, u8>(include_str!(
            "../../../fixtures/palettes/set_pal_overworld.jsonl"
        ));
        for (input, palette, _) in cases {
            assert_eq!(overworld_palette(input), palette, "{input:?}");
        }
    }

    /// Every species index, against the palette `DeterminePaletteIDOutOfBattle` chose for it.
    #[test]
    fn every_species_takes_the_colour_the_cartridge_gives_it() {
        let cases = crate::fixtures::cases::<u8, u8>(include_str!(
            "../../../fixtures/palettes/determine_palette_id.jsonl"
        ));
        for (species, palette, _) in cases {
            assert_eq!(determine_palette_id_out_of_battle(species), palette, "species {species:#04X}");
        }
    }
}
