use itertools::Itertools;
use strum::IntoEnumIterator;
use badge::Badge;
use map::Map;
use species::PokemonSpecies;
use battle::{BattleState, BattleStateReader};
use encoding::{GameMode, PokemonEncoding};
use party::PokemonParty;
use tile_map::MetaTileMap;
use std::rc::Rc;
use crate::game_boy::GameBoy;
use crate::geometry::Point8;
use crate::joypad::{JoypadButton, JoypadButtonState};
use crate::mmu::MMU;
use crate::ram::{RAM, ROM};
use crate::pokemon::bag::{Bag, BagReader, BagWriter};
use bag::BagItem;
use crate::pokemon::font::{render_font_string, FontAware, FONT_BYTES};
use crate::pokemon::item::ItemId;
use crate::pokemon::menu::{MenuState, MenuStateReader};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::options::{GameOptions, GameOptionsReader, GameOptionsWriter};
use crate::pokemon::pokedex::PokedexReader;
use crate::pokemon::pokemon::Pokemon;
use pokedex::Pokedex;
use crate::pokemon::map_metadata::{MapMetadataCache, MapMetadataReader};
use crate::pokemon::strings::PokemonString;

pub mod badge;
pub mod rom_gfx;
pub mod badge_gfx;
pub mod mon_gfx;
pub mod map_gfx;
pub mod map;
pub mod pokemon;
pub mod status;
pub mod species;
pub mod move_name;
pub mod sprite;
pub mod party;
pub mod agent;
pub mod actions;
pub mod battle;
pub mod observe;
pub mod policy;
/// **W4** — the `Policy` an LLM drives. Behind the `llm` feature, which is where its channel
/// endpoints and the worker it talks to live.
#[cfg(feature = "llm")]
pub mod llm_policy;
pub mod tile_map;
pub mod encoding;
pub mod strings;
pub mod symbols;
pub mod font;
pub mod roms;
mod text;
mod map_header;
// ⚠️ **W5 made these two public.** `ItemId` and `BagItem` are arguments the LLM tool layer parses —
// `buy_item`, `use_field_move { teach, use_item, toss_item }` — and it lives outside `pokemon`, so
// reaching them through `policy`'s one re-export would have meant re-exporting the other half too.
pub mod item;
pub mod bag;
mod menu;
pub mod delay;
pub mod damage;
pub mod world_graph;
pub mod wild;
pub mod postgame;

#[cfg(test)]
pub(crate) mod integration_tests;
pub mod data;
mod options;
pub mod map_metadata;
pub mod tile;
mod pokedex;

/// The longest player name the game itself allows: `PLAYER_NAME_LENGTH - 1`, checked by
/// `naming_screen.asm` for the player and the rival. A Pokémon's nickname gets ten.
pub const MAX_PLAYER_NAME: usize = 7;

pub trait PokemonApiTrait {
    fn release_all_buttons(&mut self);
    fn press_button(&mut self, button: JoypadButton);
    fn release_button(&mut self, button: JoypadButton);
    fn toggle_button(&mut self, button: JoypadButton);
    fn read_joypad_state(&self) -> JoypadButtonState;
    fn game_mode(&self) -> Option<GameMode>;
    /// True when a trainer has engaged the player (e.g. via line of sight) and the battle is
    /// about to start, but `wIsInBattle` has not yet flipped to its trainer-battle value.
    /// In this window the game initialises the battle on its own — the agent must NOT press
    /// any button (a held direction wedges the engagement and a battle never starts).
    fn trainer_battle_pending(&self) -> bool;
    /// True while the player is inside a **PC menu that A-mashing cannot leave**.
    ///
    /// ⚠️ **Every PC menu in Gen 1 is a closed loop under A-only input**, and this is what escapes
    /// them. Each one leaves only on B, and A on its resting cursor picks the first entry, which
    /// bounces off a refusal message straight back to the menu with the cursor untouched:
    ///
    /// | Menu | Leaves on | A on the resting cursor | Bounces back via |
    /// |---|---|---|---|
    /// | `PCMainMenu` (`menus/pc.asm:12`) | `bit B_PAD_B / jp nz, LogOff` | `BillsPC` | see the row below |
    /// | `BillsPCMenu` (`pokemon/bills_pc.asm`) | B | `WITHDRAW` | `NoMonText` on an empty box, `CantTakeMonText` on a full party (`:256`) |
    /// | `PlayerPCMenu` (`menus/players_pc.asm:19`) | `jp nz, ExitPlayerPC` | `WITHDRAW ITEM` | nothing stored |
    ///
    /// Nothing in any of those cycles moves the cursor, so A never reaches `LOG OFF` or `SEE YA!`
    /// either. An empty box is only the *first* way in, not the cause — a full party or a one-mon
    /// party (`CantDepositLastMonText`) trap just as well.
    ///
    /// **Two checks, because there are two ways into a PC and only one of them sets a flag.**
    /// `TextScript_PokemonCenterPC` goes through `ActivatePC`, which sets `wMiscFlags`'
    /// `BIT_USING_GENERIC_PC` and clears it only in `LogOff` — so that covers the whole parent tree
    /// including Bill's-PC submenus, where B is also what backs out a level at a time. But
    /// `TextScript_ItemStoragePC` — the PC in Red's bedroom, eight tiles from a fresh save — calls
    /// `PlayerPC` **directly** and deliberately leaves the flag clear (`players_pc.asm:11`, "accessing
    /// it directly"). That one is caught by its screen instead: `LOG OFF` is the last entry of both
    /// top-level PC menus and appears nowhere else in the game, which is the same match
    /// [`postgame::pc_box`](crate::pokemon::postgame::pc_box) uses and for the same reason — the
    /// first entry's *label* varies (`SOMEONE's PC` before `EVENT_MET_BILL`, `BILL's PC` after) but
    /// `LOG OFF` never does.
    fn in_pc_menu(&self) -> bool;
    /// The player's **raw** map coordinates (`wXCoord`/`wYCoord`) — i.e. before the
    /// connection-strip offsets that `MetaTileMap` adds to produce "expanded" coordinates.
    /// These are the coordinate space warp `to_position`s and world-graph node keys use, so the
    /// agent keys the incremental world graph by these when it lands on a new map.
    fn raw_player_coords(&self) -> Point8;
    fn game_state(&self) -> Result<GameState, String>;
    fn on_screen_text(&self, only_message_box: bool) -> Option<String>;
    fn menu_state(&self) -> Option<MenuState>;
    /// Currently-active list-menu template (`wListMenuID`). `0x04` (`SPECIALLISTMENU`) is the
    /// elevator floor list / badge list. Used to drive the elevator floor menu, whose `wTextBoxID`
    /// reads `MessageBox` (the "Which floor?" print) rather than `ListMenuBox`.
    fn list_menu_id(&self) -> u8;
    /// Raw menu geometry `(top_menu_item_x, top_menu_item_y, current_item, scroll_offset)` read
    /// directly from RAM, regardless of `wTextBoxID` (which the START menu leaves unset). Used to
    /// detect/drive the START-menu → bag → use-item menus when teaching an HM.
    fn menu_geometry(&self) -> (u8, u8, u8, u8);
    /// The list index of `item` in the bag as the game's item menu orders it — read from raw
    /// `wBagItems` so it matches the on-screen list exactly (unlike `read_bag`, which drops item ids
    /// not in the `ItemId` enum and so shifts every later index). Used to navigate the bag cursor.
    fn bag_item_position(&self, item: ItemId) -> Option<u8>;
    /// What a mart charges for `item`, read straight out of the ROM's `ItemPrices` table
    /// (`data/items/prices.asm` — three BCD bytes per item, in item-id order). `None` for an id the
    /// table prices at zero, i.e. one no mart sells. Used to size a purchase to the wallet: the game
    /// answers an unaffordable quantity with "You don't have enough money" and gives *nothing*, which
    /// from outside is indistinguishable from a dropped confirm.
    fn item_price(&self, item: ItemId) -> Option<u32>;
    /// How many of `item` the bag holds (0 if absent), read from raw `wBagItems`.
    fn bag_item_quantity(&self, item: ItemId) -> u8;
    /// The same two reads against **PC item storage** (`wNumBoxItems`/`wBoxItems`), which the
    /// player's-PC deposit/withdraw list shows. Same `(id, quantity)` pair layout as the bag.
    fn pc_box_item_position(&self, item: ItemId) -> Option<u8>;
    fn pc_box_item_quantity(&self, item: ItemId) -> u8;
    /// Returns the species currently being named on the nickname-entry screen.
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

    /// Writes `value` to `wMaxItemQuantity` (used to clear the stale 99 before waiting for the
    /// quantity selector to open, so we can detect the fresh write reliably).
    fn write_max_item_quantity(&mut self, value: u8);

    /// Writes `nickname` (or an empty terminator for `None`) directly into the
    /// naming screen's string buffer so pressing START submits it immediately.
    fn write_naming_screen_buffer(&mut self, nickname: Option<&str>) -> Result<(), String>;

    /// Rename the player, by writing `wPlayerName` directly.
    ///
    /// ⚠️ **There is no other way to do it, because nothing here ever sees the name screen.** A run
    /// starts from `data::START_OF_GAME`, which is a save state captured in Red's bedroom — past
    /// the title screen, past Oak's speech and past both name screens, with a name a human typed
    /// once when the fixture was made. Everything before that point is invisible to the agent
    /// anyway: `game_mode` returns `None` for the whole intro (`wPlayerName` still holds
    /// `DebugNewGamePlayerName`), so `agent.update` answers `Err("Not in game")` and no policy is
    /// ever asked anything.
    ///
    /// ⚠️ **Seven characters, not ten.** `naming_screen.asm` checks the length against
    /// `PLAYER_NAME_LENGTH - 1` for a player or rival and `NAME_LENGTH - 1` for a Pokémon, so the
    /// cap here is the game's own for this field — the storage is eleven bytes either way, and a
    /// longer name would be one the game's own UI could never have produced.
    fn write_player_name(&mut self, name: &str) -> Result<(), String>;

    fn read_game_options(&self) -> Result<GameOptions, String>;
    fn write_game_options(&mut self, options: &GameOptions) -> Result<(), String>;
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

        // 19 items — deliberately one slot short of Bag::MAX_ITEMS (20) so the agent can still pick
        // up ground items (e.g. the Mt Moon fossil, whose pickup fails with "no room" on a full bag).
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

    /// Reorder the party so the member currently in `slot` becomes the lead (slot 0), shifting the
    /// rest down. Written straight to RAM (species list + mon blocks + names), so no in-game party-menu
    /// navigation is needed. Used to make a trained bench mon (e.g. Vaporeon) the battle lead so it
    /// fights from the start of every battle and earns the XP without any in-battle switch-in.
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

    fn release_button(&mut self, button: JoypadButton) {
        self.mmu_mut().joypad_mut().release_button(button);
    }

    fn toggle_button(&mut self, button: JoypadButton) {
        let joypad = self.mmu_mut().joypad_mut();
        let pressed = !joypad.state().is_button_pressed(button);
        // release all other buttons
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

        // `BIT_ALWAYS_ON_BIKE` (wStatusFlags6 bit 5) is set while the player is forced onto the bike —
        // i.e. on Cycling Road — and `IsSurfingAllowed` refuses Surf outright there
        // (`engine/overworld/field_move_messages.asm:21-45`: *"Surfing isn't allowed on the Cycling
        // Road…"*, answered with "Cycling is fun! Forget SURFing!"). It has to be part of `can_surf`
        // because Routes 16–18 run **along the sea**: with Surf believed available, the BFS routes
        // straight down the water rather than along the road, and the agent then stands on the last dry
        // tile trying to mount Surf for the rest of the budget. Discovered riding Cycling Road for
        // workstream B — see `docs/postgame-coverage-plan.md` §11.
        const BIT_ALWAYS_ON_BIKE: u8 = 1 << 5;
        let forced_onto_bike = mmu.read_pointer(&pokered_symbols::wStatusFlags6) & BIT_ALWAYS_ON_BIKE != 0;
        // The four Safari Zone areas are the other place Surf is refused, and the refusal is quieter
        // than the Cycling Road's. Their tileset is `FOREST`, which `TilePairCollisionsWater` gives two
        // rules for — `db FOREST, $14, $2E` and `db FOREST, $48, $2E`
        // (`data/tilesets/pair_collision_tile_ids.asm:20-23`) — so a mount from the bank answers "No
        // SURFing here!" and nothing happens. With `can_surf` left true the BFS treats the centre's
        // pond as pass-through, and since a route across it can tie with the route around it, a plain
        // walk to the *nearest grass tile* picks the water at random and the agent then re-issues a
        // refused mount for the rest of the budget (workstream E; see `probe_safari_centre_from_the_
        // entrance`). Treating the zone's water as a wall is also what the pre-existing Safari steps
        // already assume — they cross Centre → East → North → West "the long way round".
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
        // Vermilion Gym trash-can puzzle: EVENT_1ST_LOCK_OPENED = 0x161 → wEventFlags[44] bit 1,
        // EVENT_2ND_LOCK_OPENED = 0x160 → wEventFlags[44] bit 0.
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
            can_use_cut: badges.contains(Badge::CascadeBadge) && has_move(&pokemon, PokemonMoveName::Cut),
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
            // EVENT_MANSION_SWITCH_ON = 0x278 → wEventFlags[79] bit 0. Toggled by any Mansion statue.
            mansion_switch_on: mmu.read(pokered_symbols::wEventFlags.address + 79) & 0x01 != 0,
            // BIT_STRENGTH_ACTIVE = bit 0 of wStatusFlags1 — set by using Strength from the party menu,
            // reset on every map change. Required before a boulder will move when pushed.
            strength_active: mmu.read_pointer(&pokered_symbols::wStatusFlags1) & 0x01 != 0,
            hall_of_fame_teams: mmu.read_pointer(&pokered_symbols::wNumHoFTeams),
            repel_steps: postgame::items::repel_steps(mmu),
            on_bicycle: postgame::items::on_bicycle(mmu),
            safari: postgame::safari::read_state(mmu),
            map,
            battle: mmu.read_battle_state(),
            bag: mmu.read_bag(),
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
        let mut prev_pos: Option<Point8> = None;
        for (char_id, pos) in coordinates {
            if only_message_box && pos.y < MESSAGE_BOX_MIN_Y {
                continue;
            }

            if let Some(prev) = prev_pos {
                if pos.y != prev.y {
                    // line break
                    lines.push(current_line);
                    current_line = Vec::new();
                } else {
                    let is_space = pos.x.saturating_sub(prev.x) > 1;
                    // only add a space (char=64) if the previous character is not a space
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
            // intro screens
            return None;
        }

        let new_game_player = mmu.read_pointer_pokemon_string(&pokered_symbols::DebugNewGamePlayerName);
        let player_name = mmu.read_pointer_pokemon_string(&pokered_symbols::wPlayerName);
        if player_name == new_game_player {
            // on new game screen
            return None;
        }
        Some(mmu.read_game_mode())
    }

    fn trainer_battle_pending(&self) -> bool {
        let mmu = self.mmu();
        // wCurOpponent is set by the trainer encounter script before InitBattle runs; for a
        // trainer battle wIsInBattle only becomes 2 after the engage/transition completes.
        mmu.read_pointer(&pokered_symbols::wCurOpponent) != 0
            && mmu.read_pointer(&pokered_symbols::wIsInBattle) == 0
    }

    fn in_pc_menu(&self) -> bool {
        // `BIT_USING_GENERIC_PC` is bit 3 of `wMiscFlags` (`constants/ram_constants.asm:11`). Cheap,
        // and it covers the parent tree's submenus, which show no `LOG OFF` of their own — so it is
        // tried first and the screen is only scanned when it says no.
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
        /// Entries in `ItemPrices` — MASTER_BALL (id 1) through FLOOR_B4F (id 97). **TMs and HMs are
        /// not in it**: their ids start at `$C4`, far past the end, and the TM prices live in their own
        /// `TMPrices` table (`data/items/tm_prices.asm`, one nibble each). Without this bound an HM
        /// reads 3 bytes of whatever follows the table and decodes them as a BCD price.
        const ITEM_PRICES_LEN: u8 = 97;

        // `table_width 3`, indexed by (item id - 1): the table starts at MASTER_BALL, which is id 1.
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
        // wItemList format: [count, item1, item2, ..., 0xFF] — skip the count byte at index 0
        (1..16u16)
            .map(|i| mmu.read(pokered_symbols::wItemList.address + i))
            .take_while(|&b| b != 0xFF)
            .filter_map(ItemId::from_repr)
            .collect()
    }

    fn mart_item_quantity(&self) -> u8 {
        self.mmu().read(pokered_symbols::wItemQuantity.address)
    }

    /// True when the buy-quantity selector is active (pokemart sets wMaxItemQuantity=99).
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
            // ⚠️ No em dash: this reaches the page inside a `Notice`.
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

    fn write_game_options(&mut self, options: &GameOptions) -> Result<(), String> {
        self.mmu_mut().write_game_options(options)
    }
}

#[derive(Debug, Clone, Default)]
pub struct GameState {
    pub player_id: u16,
    pub name: PokemonString,
    pub rival_name: PokemonString,
    pub badges: Badge,
    pub money: u32,
    /// Game Corner coins (`wPlayerCoins`, two BCD bytes, so 0–9999). Only spendable in the prize
    /// room; bought at the counter at ¥1000 → 50. Zero until the Coin Case is held — the counter
    /// refuses without it — which is why workstream F starts there.
    pub coins: u16,
    /// True on a **dark** map, i.e. one that HM05 Flash lights. `wMapPalOffset` is set to 6 on
    /// entering `ROCK_TUNNEL_1F` (`home/overworld.asm:497-501`) and cleared to 0 by the Flash field
    /// move (`engine/menus/start_sub_menus.asm:183-191`) — the palette offset *is* the darkness, so
    /// this one byte is both the precondition for using Flash and the proof that it worked.
    pub map_is_dark: bool,
    pub pokemon: PokemonParty,
    pub mode: GameMode,
    pub map: MetaTileMap,
    pub bag: Bag,
    /// Populated whenever `mode` is `WildBattle` or `TrainerBattle`.
    pub battle: Option<BattleState>,
    /// Contents of the **currently open** PC box (`wBoxCount`/`wBoxMons`). Only ever one box of the
    /// twelve — see [`postgame::pc_box::read_current_box`] for why the other eleven aren't readable.
    pub boxed_pokemon: Vec<postgame::pc_box::BoxedPokemon>,
    /// Which box is open, 0-based (`wCurrentBoxNum`).
    pub current_box: u8,
    /// True when the player has the Cascade Badge and at least one party Pokémon
    /// knows HM Cut — the two requirements to use Cut outside of battle in pokémon Red.
    /// Currently always false until the player has earned these.
    pub can_use_cut: bool,
    /// True when the player has the Soul Badge and at least one party Pokémon knows HM Surf — the two
    /// requirements to Surf outside of battle in pokémon Red. Gates water traversal in the pathfinder.
    pub can_use_surf: bool,
    /// True once EVENT_GOT_POKEDEX is set (Oak gives the player the Pokédex).
    pub has_pokedex: bool,
    /// Species the player owns (caught or received) — set bit in `wPokedexOwned`.
    pub pokedex_owned: Pokedex,
    /// Species the player has seen (encountered in battle) — set bit in `wPokedexSeen`.
    pub pokedex_seen: Pokedex,
    /// Vermilion Gym trash-can switch puzzle state — `Some` only when `map` is `VermilionGym`.
    pub trash_cans: Option<TrashCanPuzzle>,
    /// True once EVENT_FOUND_ROCKET_HIDEOUT is set — the Celadon Game Corner poster switch has been
    /// flipped, opening the hidden staircase down to the Rocket Hideout.
    pub found_rocket_hideout: bool,
    /// State of EVENT_MANSION_SWITCH_ON — the single global Pokémon Mansion switch that every statue
    /// on every floor toggles, opening/closing the sliding-door gates on all four floors.
    pub mansion_switch_on: bool,
    /// Live state of a Safari Zone trip — `Some` only while `EVENT_IN_SAFARI_ZONE` is set, which is
    /// every tick between paying the ¥500 and the gate printing "good haul". Carries the two budgets
    /// the game enforces (502 steps, 30 balls); see [`postgame::safari::SafariState`].
    pub safari: Option<postgame::safari::SafariState>,
    /// True while Strength is active (BIT_STRENGTH_ACTIVE in `wStatusFlags1`) — set by using Strength
    /// from the party menu, reset on every map change. A boulder only moves when pushed with this set.
    pub strength_active: bool,
    /// `wNumHoFTeams` — how many times this save has entered the Hall of Fame. **Non-zero means the
    /// game has been beaten**, and it is the only byte that means that both during the ceremony and
    /// long after it: `AnimateHallOfFame` increments it at its very first frame
    /// (`engine/movie/hall_of_fame.asm:27-32`), saturating rather than wrapping, and it sits inside
    /// the `wMainDataStart..wMainDataEnd` block that `engine/menus/save.asm` round-trips through SRAM
    /// — so the credits' save-and-soft-reset carries it across. It is what the ROM's own main menu
    /// reads to warp a returning Champion home (`main_menu.asm:116-125`).
    ///
    /// ⚠️ **This is not what [`agent::PokemonAgent`]'s completion detector reads.** `game_state` is
    /// fallible and answers `Err` through every screen transition, and a Hall of Fame ceremony is
    /// made of them; the detector reads the byte straight out of the MMU for that reason. This field
    /// is here so the model and the tests can see it.
    pub hall_of_fame_teams: u8,
    /// **I5** — `wRepelRemainingSteps`: overworld steps left before the Repel wears off. Repel sets
    /// 100, Super Repel 200, Max Repel 250, and the counter is decremented one per step; while it is
    /// non-zero the ROM suppresses any wild encounter whose level is below the lead's.
    pub repel_steps: u8,
    /// **I6** — true while riding the Bicycle (`wWalkBikeSurfState == 1`). The bike doubles overworld
    /// speed, and it is a *toggle*: using the item again dismounts.
    pub on_bicycle: bool,
}

/// State of the Vermilion Gym two-switch trash-can puzzle that unlocks the door to Lt. Surge.
///
/// The two switches hide in the cans indexed by `wFirstLockTrashCanIndex` /
/// `wSecondLockTrashCanIndex`; `first_target` / `second_target` are those cans' map coordinates.
/// Check the first can (opens the 1st lock, which then randomly places the 2nd switch in an adjacent
/// can), then check the second can (opens the 2nd lock and unlocks the door). Checking a wrong can
/// for the second switch resets both locks — reading the indices from RAM lets the agent go straight
/// to the correct cans and never reset.
#[derive(Debug, Clone)]
pub struct TrashCanPuzzle {
    pub first_target: crate::geometry::Point8,
    pub second_target: crate::geometry::Point8,
    pub first_opened: bool,
    pub second_opened: bool,
}

/// Both the bag (`wNumBagItems`/`wBagItems`) and PC item storage (`wNumBoxItems`/`wBoxItems`) are a
/// count byte followed by `(id, quantity)` pairs, so the two readers below serve either.
///
/// These read **raw** RAM rather than going through [`Bag`], which silently drops every id [`ItemId`]
/// cannot name — most of the TMs — and so reports both the wrong count and shifted indices. Menu
/// navigation and occupancy checks must use these.
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

/// Map coordinate of gym trash-can hidden event `index` (0..=14), from pokered
/// `data/events/hidden_events.asm` (`HiddenEventsFor_VERMILION_GYM`): cans laid out in a 5×3 grid at
/// odd columns 1,3,5,7,9 and rows 7,9,11, indexed column-major.
pub fn trash_can_position(index: u8) -> crate::geometry::Point8 {
    crate::geometry::Point8 { x: 1 + 2 * (index / 3), y: 7 + 2 * (index % 3) }
}

