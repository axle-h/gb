use itertools::Itertools;
use strum::IntoEnumIterator;
use badge::Badge;
use map::Map;
use species::PokemonSpecies;
use battle::{BattleState, BattleStateReader};
use encoding::{GameMode, PokemonEncoding};
use party::PokemonParty;
use tile_map::MetaTileMap;
use gb::game_boy::GameBoy;
use poke_core::geometry::Point8;
use gb::joypad::{JoypadButton, JoypadButtonState};
use gb::mmu::MMU;
use gb::ram::{RAM, ROM};
use crate::pokemon::bag::{Bag, BagReader, BagWriter};
use bag::BagItem;
use crate::pokemon::font::{render_font_string, FontAware, FONT_BYTES};
use crate::pokemon::item::ItemId;
use crate::pokemon::menu::{MenuState, MenuStateReader};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::options::{GameOptions, GameOptionsReader};
use crate::pokemon::pokedex::PokedexReader;
use crate::pokemon::pokemon::Pokemon;
use pokedex::Pokedex;
use crate::pokemon::map_metadata::{MapMetadataCache, MapMetadataReader};
use crate::pokemon::strings::PokemonString;

pub use poke_core::badge;
pub use poke_core::rom_gfx;
pub use poke_core::badge_gfx;
pub use poke_core::mon_gfx;
pub mod learnset;
pub use poke_core::map_gfx;
pub use poke_core::map;
pub use poke_core::pokemon;
pub use poke_core::status;
pub use poke_core::species;
pub use poke_core::move_name;
pub use poke_core::sprite;
pub use poke_core::party;
pub mod agent;
pub mod actions;
pub mod battle;
pub mod observe;
pub mod policy;
/// The `Policy` an LLM drives.
pub mod llm_policy;
pub mod tile_map;
pub mod encoding;
pub use poke_core::strings;
pub mod symbols;
pub mod font;
pub use poke_core::roms;
mod text;
pub mod map_header;
pub use poke_core::item;
pub mod item_use;
pub mod bag;
mod menu;
pub mod delay;
pub use poke_core::damage;
pub mod world_graph;
pub use poke_core::wild;
pub mod postgame;

#[cfg(test)]
pub(crate) mod integration_tests;
pub mod data;
pub mod options;
pub mod map_metadata;
pub mod tile;
mod pokedex;

/// The longest player name the game allows (`PLAYER_NAME_LENGTH - 1`); a nickname gets ten.
pub const MAX_PLAYER_NAME: usize = 7;

pub trait PokemonApiTrait {
    fn release_all_buttons(&mut self);
    fn press_button(&mut self, button: JoypadButton);
    fn toggle_button(&mut self, button: JoypadButton);
    fn read_joypad_state(&self) -> JoypadButtonState;
    fn game_mode(&self) -> Option<GameMode>;
    fn a_game_is_loaded(&self) -> bool;

    /// A trainer has engaged the player and `wIsInBattle` has not yet flipped to a trainer battle.
    fn trainer_battle_pending(&self) -> bool;
    /// True while the player is inside a PC menu that A-mashing cannot leave.
    fn in_pc_menu(&self) -> bool;
    /// `wXCoord`/`wYCoord`, before the connection-strip offsets `MetaTileMap` adds.
    fn raw_player_coords(&self) -> Point8;
    fn game_state(&self) -> Result<GameState, String>;
    fn on_screen_text(&self, only_message_box: bool) -> Option<String>;
    fn menu_state(&self) -> Option<MenuState>;
    /// Currently-active list-menu template (`wListMenuID`).
    fn list_menu_id(&self) -> u8;
    /// `(top_x, top_y, current_item, scroll_offset)`, read regardless of `wTextBoxID`, which the
    /// START menu leaves unset.
    fn menu_geometry(&self) -> (u8, u8, u8, u8);
    /// `item`'s index in raw `wBagItems`, as the game's item menu lists it; `read_bag` drops ids
    /// outside `ItemId` and so shifts every later index.
    fn bag_item_position(&self, item: ItemId) -> Option<u8>;
    /// A mart's price for `item`, from the ROM's `ItemPrices` (three BCD bytes per item).
    fn item_price(&self, item: ItemId) -> Option<u32>;
    /// How many of `item` the bag holds (0 if absent), read from raw `wBagItems`.
    fn bag_item_quantity(&self, item: ItemId) -> u8;
    /// The same two reads against PC item storage (`wNumBoxItems`/`wBoxItems`).
    fn pc_box_item_position(&self, item: ItemId) -> Option<u8>;
    fn pc_box_item_quantity(&self, item: ItemId) -> u8;
    fn pc_stored_items(&self) -> Bag;
    /// The species being named on the nickname screen.
    fn naming_screen_species(&self) -> Result<PokemonSpecies, String>;

    /// The move currently being learned on the level-up move-forget prompt (`wMoveNum`).
    fn move_to_learn(&self) -> Option<crate::pokemon::move_name::PokemonMoveName>;
    /// Party index of the Pokémon learning a move on the move-forget prompt (`wWhichPokemon`).
    fn learning_pokemon_index(&self) -> usize;
    /// Reads the mart's current item list from `wItemList` (up to 16 entries, FF-terminated).
    fn mart_item_list(&self) -> Vec<ItemId>;

    /// Reads `wItemQuantity` — the quantity currently shown on the buy-quantity selector.
    fn mart_item_quantity(&self) -> u8;

    /// True when the pokemart buy-quantity selector is active (wMaxItemQuantity == 99).
    fn mart_in_quantity_selector(&self) -> bool;

    fn write_max_item_quantity(&mut self, value: u8);

    /// Writes `nickname` into the naming screen's buffer, so pressing START submits it at once.
    fn write_naming_screen_buffer(&mut self, nickname: Option<&str>) -> Result<(), String>;

    /// Rename the player, by writing `wPlayerName` directly.
    fn write_player_name(&mut self, name: &str) -> Result<(), String>;

    fn read_game_options(&self) -> Result<GameOptions, String>;
}

#[derive(Debug)]
pub struct PokemonApi<'a> {
    game_boy: &'a mut GameBoy,
    map_cache: Option<&'a mut MapMetadataCache>,
}

impl<'a> PokemonApi<'a> {
    pub fn new(game_boy: &'a mut GameBoy) -> Self {
        Self { game_boy, map_cache: None }
    }

    pub fn with_cache(game_boy: &'a mut GameBoy, cache: &'a mut MapMetadataCache) -> Self {
        Self { game_boy, map_cache: Some(cache) }
    }

    pub fn mmu(&self) -> &MMU {
        self.game_boy.core().mmu()
    }

    fn mmu_mut(&mut self) -> &mut MMU {
        self.game_boy.core_mut().mmu_mut()
    }

    pub fn pimp_out_pokemon(&mut self) -> Result<(), String> {
        let player_state = self.game_state()?;

        // One slot short of `Bag::MAX_ITEMS`, so a ground pickup still has room.
        const EPIC_BAG: [BagItem; 19] = [
            BagItem::new(ItemId::Revive, 99),
            BagItem::new(ItemId::FullHeal, 99),
            BagItem::new(ItemId::Potion, 99),
            BagItem::new(ItemId::SuperPotion, 99),
            BagItem::new(ItemId::HyperPotion, 99),
            BagItem::new(ItemId::MaxPotion, 99),
            BagItem::new(ItemId::Bicycle, 1),
            BagItem::new(ItemId::TownMap, 1),
            BagItem::new(ItemId::EscapeRope, 99),
            BagItem::new(ItemId::FireStone, 99),
            BagItem::new(ItemId::WaterStone, 99),
            BagItem::new(ItemId::LeafStone, 99),
            BagItem::new(ItemId::MoonStone, 99),
            BagItem::new(ItemId::ThunderStone, 99),
            BagItem::new(ItemId::PokeBall, 99),
            BagItem::new(ItemId::GreatBall, 99),
            BagItem::new(ItemId::UltraBall, 99),
            BagItem::new(ItemId::MasterBall, 99),
            BagItem::new(ItemId::RareCandy, 99),
        ];

        let mut party = PokemonParty::default();
        let charizard = Pokemon::maxed(
            PokemonSpecies::Charizard,
            "CHARIZARD",
            [
                PokemonMoveName::Flamethrower,
                PokemonMoveName::Slash,
                PokemonMoveName::Fly,
                PokemonMoveName::Earthquake,
            ],
            player_state.name.clone(),
            player_state.player_id
        );
        party.push(charizard)?;

        let venusaur = Pokemon::maxed(
            PokemonSpecies::Venusaur,
            "VENUSAUR",
            [
                PokemonMoveName::RazorLeaf,
                PokemonMoveName::Solarbeam,
                PokemonMoveName::Absorb,
                PokemonMoveName::Acid,
            ],
            player_state.name.clone(),
            player_state.player_id
        );
        party.push(venusaur)?;

        let blastoise = Pokemon::maxed(
            PokemonSpecies::Blastoise,
            "BLASTOISE",
            [
                PokemonMoveName::Surf,
                PokemonMoveName::HydroPump,
                PokemonMoveName::Blizzard,
                PokemonMoveName::Waterfall,
            ],
            player_state.name.clone(),
            player_state.player_id
        );
        party.push(blastoise)?;

        let mewtwo = Pokemon::maxed(
            PokemonSpecies::Mewtwo,
            "MEWTWO",
            [
                PokemonMoveName::Psychic,
                PokemonMoveName::Thunderbolt,
                PokemonMoveName::IceBeam,
                PokemonMoveName::Recover,
            ],
            player_state.name.clone(),
            player_state.player_id
        );
        party.push(mewtwo)?;

        let dragonite = Pokemon::maxed(
            PokemonSpecies::Dragonite,
            "DRAGONITE",
            [
                PokemonMoveName::DragonRage,
                PokemonMoveName::HyperBeam,
                PokemonMoveName::Slam,
                PokemonMoveName::ThunderWave,
            ],
            player_state.name.clone(),
            player_state.player_id
        );
        party.push(dragonite)?;

        let tauros = Pokemon::maxed(
            PokemonSpecies::Tauros,
            "TAUROS",
            [
                PokemonMoveName::HyperBeam,
                PokemonMoveName::BodySlam,
                PokemonMoveName::Earthquake,
                PokemonMoveName::Blizzard,
            ],
            player_state.name,
            player_state.player_id
        );
        party.push(tauros)?;

        let mmu = self.mmu_mut();
        mmu.write_bag(&Bag::from_slice(&EPIC_BAG));
        mmu.write_player_pokemon_party(&party)
    }

    /// Moves the member in `slot` to the lead, by writing the party straight to RAM.
    pub fn move_party_member_to_front(&mut self, slot: usize) -> Result<(), String> {
        let mut party = self.mmu().read_player_pokemon_party()?;
        party.move_to_front(slot);
        self.mmu_mut().write_player_pokemon_party(&party)
    }
}

impl<'a> PokemonApiTrait for PokemonApi<'a> {
    fn release_all_buttons(&mut self) {
        let joypad = self.mmu_mut().joypad_mut();
        for button in JoypadButton::iter() {
            joypad.release_button(button);
        }
    }
    fn press_button(&mut self, button: JoypadButton) {
        self.mmu_mut().joypad_mut().press_button(button);
    }

    fn toggle_button(&mut self, button: JoypadButton) {
        let joypad = self.mmu_mut().joypad_mut();
        let pressed = !joypad.state().is_button_pressed(button);
        for btn in JoypadButton::iter() {
            joypad.release_button(btn);
        }
        joypad.update_button(button, pressed);
    }

    fn read_joypad_state(&self) -> JoypadButtonState {
        self.mmu().joypad().state()
    }

    fn game_state(&self) -> Result<GameState, String> {
        let mmu = self.mmu();
        let badges = Badge::from_bits(
            mmu.read_pointer(&pokered_symbols::wObtainedBadges)
        ).ok_or("cannot parse badges")?;
        let pokemon = mmu.read_player_pokemon_party()?;

        fn has_move(pokemon: &PokemonParty, match_move: PokemonMoveName) -> bool {
            pokemon.iter().any(|p| {
                p.moves.iter()
                    .any(|m| m.map_or(false, |m| m.name == match_move))
            })
        }

        // `BIT_ALWAYS_ON_BIKE` marks Cycling Road, where `IsSurfingAllowed` refuses Surf.
        const BIT_ALWAYS_ON_BIKE: u8 = 1 << 5;
        let forced_onto_bike = mmu.read_pointer(&pokered_symbols::wStatusFlags6) & BIT_ALWAYS_ON_BIKE != 0;
        // The Safari Zone refuses Surf too.
        let mut map = MetaTileMap::new(&match &self.map_cache {
            Some(c) => c.read_current_map(mmu)?,
            None    => mmu.read_current_map()?,
        });
        let in_safari_zone = matches!(map.map,
            Map::SafariZoneCenter | Map::SafariZoneEast | Map::SafariZoneNorth | Map::SafariZoneWest);
        let can_use_surf = badges.contains(Badge::SoulBadge)
            && has_move(&pokemon, PokemonMoveName::Surf)
            && !forced_onto_bike
            && !in_safari_zone;
        map.can_surf = can_use_surf;
        // `.cut`'s own badge and move check: `actions()` must not offer a walk whose only follow-up
        // is a field move the game refuses.
        let can_use_cut = badges.contains(Badge::CascadeBadge) && has_move(&pokemon, PokemonMoveName::Cut);
        map.can_cut = can_use_cut;
        // Strength likewise, or `actions()` offers a push the cartridge answers with silence.
        map.can_strength = badges.contains(Badge::RainbowBadge) && has_move(&pokemon, PokemonMoveName::Strength);
        // Likewise no fishing row without a rod.
        let bag = mmu.read_bag();
        map.best_rod = postgame::fishing::Rod::best_in_bag(&bag);
        // Bill's cell separator, while pressing it would do something.
        map.bill_cell_separator = map.map == Map::BillsHouse && {
            let flags = mmu.read(pokered_symbols::wEventFlags.address + 171);
            flags & 0x40 != 0 && flags & 0x08 == 0
        };
        // EVENT_1ST_LOCK_OPENED (0x161), EVENT_2ND_LOCK_OPENED (0x160): wEventFlags[44] bits 1, 0.
        let trash_cans = (map.map == Map::VermilionGym).then(|| {
            let flags = mmu.read(pokered_symbols::wEventFlags.address + 44);
            TrashCanPuzzle {
                first_target: trash_can_position(mmu.read_pointer(&pokered_symbols::wFirstLockTrashCanIndex)),
                second_target: trash_can_position(mmu.read_pointer(&pokered_symbols::wSecondLockTrashCanIndex)),
                first_opened: flags & 0x02 != 0,
                second_opened: flags & 0x01 != 0,
            }
        });

        Ok(GameState {
            player_id: mmu.read_pointer_u16_be(&pokered_symbols::wPlayerID),
            name: mmu.read_pointer_pokemon_string(&pokered_symbols::wPlayerName),
            rival_name: mmu.read_pointer_pokemon_string(&pokered_symbols::wRivalName),
            can_use_cut,
            can_use_surf,
            badges,
            money: encoding::reverse_bcd(mmu.read_pointer_u24_be(&pokered_symbols::wPlayerMoney)),
            coins: encoding::reverse_bcd(mmu.read_pointer_u16_be(&pokered_symbols::wPlayerCoins) as u32) as u16,
            map_is_dark: mmu.read_pointer(&pokered_symbols::wMapPalOffset) != 0,
            mode: mmu.read_game_mode(),
            pokemon,
            trash_cans,
            // EVENT_FOUND_ROCKET_HIDEOUT = 0x1b9 → wEventFlags[55] bit 1.
            found_rocket_hideout: mmu.read(pokered_symbols::wEventFlags.address + 55) & 0x02 != 0,
            // EVENT_MANSION_SWITCH_ON = 0x278 → wEventFlags[79] bit 0.
            mansion_switch_on: mmu.read(pokered_symbols::wEventFlags.address + 79) & 0x01 != 0,
            // BIT_STRENGTH_ACTIVE = bit 0 of wStatusFlags1.
            strength_active: mmu.read_pointer(&pokered_symbols::wStatusFlags1) & 0x01 != 0,
            hall_of_fame_teams: mmu.read_pointer(&pokered_symbols::wNumHoFTeams),
            repel_steps: postgame::items::repel_steps(mmu),
            on_bicycle: postgame::items::on_bicycle(mmu),
            day_care_in_use: mmu.read_pointer(&pokered_symbols::wDayCareInUse) != 0,
            safari: postgame::safari::read_state(mmu),
            map,
            battle: mmu.read_battle_state(),
            bag,
            boxed_pokemon: postgame::pc_box::read_current_box(mmu),
            current_box: postgame::pc_box::current_box_num(mmu),
            has_pokedex: mmu.read_has_pokedex(),
            pokedex_owned: mmu.read_pokedex(&pokered_symbols::wPokedexOwned)?,
            pokedex_seen: mmu.read_pokedex(&pokered_symbols::wPokedexSeen)?,
        })
    }

    fn on_screen_text(&self, only_message_box: bool) -> Option<String> {
        let mmu = self.mmu();
        if mmu.read_game_mode() == GameMode::Overworld || !mmu.pokemon_font_loaded() {
            return None;
        }
        let ppu = mmu.ppu();
        let font_tiles = ppu.tile_indexes_of_vram_addresses(pokered_symbols::vFont.address, FONT_BYTES.len());
        if font_tiles.is_empty() {
            return None;
        }
        let mut coordinates = ppu.tile_coordinates(&font_tiles);
        coordinates.sort_by_key(|(_, p)| *p);

        const MESSAGE_BOX_MIN_Y: u8 = 13;

        let mut lines = Vec::new();
        let mut current_line = Vec::new();
        let mut prev_pos: Option<gb::geometry::Point8> = None;
        for (char_id, pos) in coordinates {
            if only_message_box && pos.y < MESSAGE_BOX_MIN_Y {
                continue;
            }

            if let Some(prev) = prev_pos {
                if pos.y != prev.y {
                    lines.push(current_line);
                    current_line = Vec::new();
                } else {
                    let is_space = pos.x.saturating_sub(prev.x) > 1;
                    // 64 is the space glyph; never two in a row.
                    if is_space && current_line.last() != Some(&64) {
                        current_line.push(64);
                    }
                }
            }

            current_line.push(char_id);
            prev_pos = Some(pos);
        }
        if !current_line.is_empty() {
            lines.push(current_line);
        }

        Some(
            lines.into_iter()
                .map(|line| render_font_string(&line, false).trim().to_string())
                .join(" ")
        )
    }

    fn game_mode(&self) -> Option<GameMode> {
        let mmu = self.mmu();
        let player_id = mmu.read_pointer_u16_be(&pokered_symbols::wPlayerID);
        if player_id == 0 {
            // Intro screens
            return None;
        }

        let new_game_player = mmu.read_pointer_pokemon_string(&pokered_symbols::DebugNewGamePlayerName);
        let player_name = mmu.read_pointer_pokemon_string(&pokered_symbols::wPlayerName);
        if player_name == new_game_player {
            // On new game screen
            return None;
        }
        Some(mmu.read_game_mode())
    }

    fn a_game_is_loaded(&self) -> bool {
        self.mmu().read_pointer_u16_be(&pokered_symbols::wPlayerID) != 0
    }

    fn trainer_battle_pending(&self) -> bool {
        let mmu = self.mmu();
        // `wCurOpponent` is set before InitBattle; `wIsInBattle` becomes 2 only after the engage.
        mmu.read_pointer(&pokered_symbols::wCurOpponent) != 0
            && mmu.read_pointer(&pokered_symbols::wIsInBattle) == 0
    }

    fn in_pc_menu(&self) -> bool {
        // `BIT_USING_GENERIC_PC` is bit 3 of `wMiscFlags` (`constants/ram_constants.asm:11`).
        self.mmu().read_pointer(&pokered_symbols::wMiscFlags) & 0x08 != 0
            || self.on_screen_text(false).is_some_and(|text| text.contains("LOG OFF"))
    }

    fn raw_player_coords(&self) -> Point8 {
        let mmu = self.mmu();
        Point8 {
            x: mmu.read_pointer(&pokered_symbols::wXCoord),
            y: mmu.read_pointer(&pokered_symbols::wYCoord),
        }
    }

    fn menu_state(&self) -> Option<MenuState> {
        self.mmu().read_menu_state()
    }

    fn list_menu_id(&self) -> u8 {
        self.mmu().read_pointer(&pokered_symbols::wListMenuID)
    }

    fn menu_geometry(&self) -> (u8, u8, u8, u8) {
        let mmu = self.mmu();
        (
            mmu.read_pointer(&pokered_symbols::wTopMenuItemX),
            mmu.read_pointer(&pokered_symbols::wTopMenuItemY),
            mmu.read_pointer(&pokered_symbols::wCurrentMenuItem),
            mmu.read_pointer(&pokered_symbols::wListScrollOffset),
        )
    }

    fn bag_item_position(&self, item: ItemId) -> Option<u8> {
        let mmu = self.mmu();
        let count = mmu.read_pointer(&pokered_symbols::wNumBagItems) as usize;
        let base = pokered_symbols::wBagItems.address;
        (0..count).find(|&i| mmu.read(base + i as u16 * 2) == item as u8).map(|i| i as u8)
    }

    fn bag_item_quantity(&self, item: ItemId) -> u8 {
        inventory_quantity(self.mmu(), &pokered_symbols::wNumBagItems, &pokered_symbols::wBagItems, item)
    }

    fn item_price(&self, item: ItemId) -> Option<u32> {
        /// Entries in `ItemPrices` — MASTER_BALL (id 1) through FLOOR_B4F (id 97).
        const ITEM_PRICES_LEN: u8 = 97;

        // Indexed by item id - 1: the table starts at MASTER_BALL, id 1.
        let id = item as u8;
        if id > ITEM_PRICES_LEN { return None; }
        let entry = pokered_symbols::ItemPrices + (id.checked_sub(1)? as u16) * 3;
        match encoding::reverse_bcd(self.mmu().read_pointer_u24_be(&entry)) {
            0 => None,
            price => Some(price),
        }
    }

    fn pc_box_item_position(&self, item: ItemId) -> Option<u8> {
        inventory_position(self.mmu(), &pokered_symbols::wNumBoxItems, &pokered_symbols::wBoxItems, item)
    }

    fn pc_stored_items(&self) -> Bag {
        self.mmu().read_pc_items()
    }

    fn pc_box_item_quantity(&self, item: ItemId) -> u8 {
        inventory_quantity(self.mmu(), &pokered_symbols::wNumBoxItems, &pokered_symbols::wBoxItems, item)
    }

    fn naming_screen_species(&self) -> Result<PokemonSpecies, String> {
        let byte = self.mmu().read_pointer(&pokered_symbols::wCurPartySpecies);
        PokemonSpecies::from_repr(byte)
            .ok_or_else(|| format!("Invalid species byte {byte:#04x} on naming screen"))
    }

    fn move_to_learn(&self) -> Option<crate::pokemon::move_name::PokemonMoveName> {
        crate::pokemon::move_name::PokemonMoveName::from_repr(self.mmu().read_pointer(&pokered_symbols::wMoveNum))
    }

    fn learning_pokemon_index(&self) -> usize {
        self.mmu().read_pointer(&pokered_symbols::wWhichPokemon) as usize
    }

    fn mart_item_list(&self) -> Vec<ItemId> {
        let mmu = self.mmu();
        // WItemList format: [count, item1, item2, ..., 0xFF] — skip the count byte at index 0
        (1..16u16)
            .map(|i| mmu.read(pokered_symbols::wItemList.address + i))
            .take_while(|&b| b != 0xFF)
            .filter_map(ItemId::from_repr)
            .collect()
    }

    fn mart_item_quantity(&self) -> u8 {
        self.mmu().read(pokered_symbols::wItemQuantity.address)
    }

    fn mart_in_quantity_selector(&self) -> bool {
        self.mmu().read(pokered_symbols::wMaxItemQuantity.address) == 99
    }

    fn write_max_item_quantity(&mut self, value: u8) {
        self.mmu_mut().write(pokered_symbols::wMaxItemQuantity.address, value);
    }

    fn write_naming_screen_buffer(&mut self, nickname: Option<&str>) -> Result<(), String> {
        let bytes: Vec<u8> = match nickname {
            None | Some("") => vec![PokemonString::TERMINATOR],
            Some(name) => {
                let mut ps = PokemonString::from_string(name).0;
                // Clamp to 10 encoded chars + terminator.
                if let Some(pos) = ps.iter().position(|&b| b == PokemonString::TERMINATOR) {
                    if pos > 10 { ps[10] = PokemonString::TERMINATOR; ps.truncate(11); }
                } else {
                    ps.truncate(10); ps.push(PokemonString::TERMINATOR);
                }
                ps
            }
        };
        self.mmu_mut().write_pointer_slice(&pokered_symbols::wStringBuffer, &bytes)
    }

    fn write_player_name(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            // No em dash: this reaches the page inside a `Notice`.
            return Err("a player name cannot be empty; the game's own screen refuses one".to_string());
        }
        let mut bytes = PokemonString::from_string(name).0;
        match bytes.iter().position(|&b| b == PokemonString::TERMINATOR) {
            Some(end) if end > MAX_PLAYER_NAME => {
                bytes[MAX_PLAYER_NAME] = PokemonString::TERMINATOR;
                bytes.truncate(MAX_PLAYER_NAME + 1);
            }
            Some(_) => {}
            None => {
                bytes.truncate(MAX_PLAYER_NAME);
                bytes.push(PokemonString::TERMINATOR);
            }
        }
        self.mmu_mut().write_pointer_slice(&pokered_symbols::wPlayerName, &bytes)
    }

    fn read_game_options(&self) -> Result<GameOptions, String> {
        self.mmu().read_game_options()
    }

}

#[derive(Debug, Clone, Default)]
pub struct GameState {
    pub player_id: u16,
    pub name: PokemonString,
    pub rival_name: PokemonString,
    pub badges: Badge,
    pub money: u32,
    /// Game Corner coins, two BCD bytes (0–9999).
    pub coins: u16,
    /// A dark map: `wMapPalOffset` is set on entering one and cleared by Flash, so this byte is
    /// both Flash's precondition and its proof.
    pub map_is_dark: bool,
    pub pokemon: PokemonParty,
    pub mode: GameMode,
    pub map: MetaTileMap,
    pub bag: Bag,
    /// Populated whenever `mode` is `WildBattle` or `TrainerBattle`.
    pub battle: Option<BattleState>,
    /// The open PC box only; see [`postgame::pc_box::read_current_box`] for why.
    pub boxed_pokemon: Vec<postgame::pc_box::BoxedPokemon>,
    /// Which box is open, 0-based (`wCurrentBoxNum`).
    pub current_box: u8,
    /// The Cascade Badge and a party member knowing Cut, which Cut outside battle needs.
    pub can_use_cut: bool,
    /// The Soul Badge and a party member knowing Surf, off Cycling Road and the Safari Zone.
    pub can_use_surf: bool,
    /// True once EVENT_GOT_POKEDEX is set (Oak gives the player the Pokédex).
    pub has_pokedex: bool,
    /// Species owned: set bits of `wPokedexOwned`.
    pub pokedex_owned: Pokedex,
    /// Species seen: set bits of `wPokedexSeen`.
    pub pokedex_seen: Pokedex,
    /// `Some` only when `map` is `VermilionGym`.
    pub trash_cans: Option<TrashCanPuzzle>,
    /// The Game Corner poster has been pressed, opening the stairs to the Rocket Hideout.
    pub found_rocket_hideout: bool,
    /// EVENT_MANSION_SWITCH_ON: the one switch every Mansion statue toggles.
    pub mansion_switch_on: bool,
    /// A Safari Zone trip in progress, with the step and ball budgets the game enforces.
    pub safari: Option<postgame::safari::SafariState>,
    /// Strength is active: a boulder moves only while it is, and a map change resets it.
    pub strength_active: bool,
    /// `wNumHoFTeams`, non-zero once the game is beaten: incremented on the ceremony's first frame
    /// and saved through the credits' soft reset.
    pub hall_of_fame_teams: u8,
    /// `wRepelRemainingSteps`: steps left before the Repel wears off.
    pub repel_steps: u8,
    /// Riding the Bicycle (`wWalkBikeSurfState == 1`).
    pub on_bicycle: bool,
    /// The Day Care is boarding a Pokémon, one at a time.
    pub day_care_in_use: bool,
}

/// State of the Vermilion Gym two-switch trash-can puzzle that unlocks the door to Lt. Surge.
#[derive(Debug, Clone)]
pub struct TrashCanPuzzle {
    pub first_target: poke_core::geometry::Point8,
    pub second_target: poke_core::geometry::Point8,
    pub first_opened: bool,
    pub second_opened: bool,
}

/// The bag and PC item storage are both a count byte then `(id, quantity)` pairs.
fn inventory_position(mmu: &MMU, count_ptr: &symbols::DmgPointer, base_ptr: &symbols::DmgPointer, item: ItemId) -> Option<u8> {
    let count = mmu.read_pointer(count_ptr) as usize;
    (0..count)
        .find(|&i| mmu.read(base_ptr.address + i as u16 * 2) == item as u8)
        .map(|i| i as u8)
}

fn inventory_quantity(mmu: &MMU, count_ptr: &symbols::DmgPointer, base_ptr: &symbols::DmgPointer, item: ItemId) -> u8 {
    match inventory_position(mmu, count_ptr, base_ptr, item) {
        Some(i) => mmu.read(base_ptr.address + i as u16 * 2 + 1),
        None => 0,
    }
}

/// Map coordinate of gym trash can `index` (0..=14): a 5×3 grid at odd columns 1-9 and rows
/// 7-11, column-major (`HiddenEventsFor_VERMILION_GYM`).
pub fn trash_can_position(index: u8) -> poke_core::geometry::Point8 {
    poke_core::geometry::Point8 { x: 1 + 2 * (index / 3), y: 7 + 2 * (index % 3) }
}
