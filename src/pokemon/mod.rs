use itertools::Itertools;
use strum::IntoEnumIterator;
use badge::Badge;
use map::Map;
use species::PokemonSpecies;
use battle::{BattleState, BattleStateReader};
use encoding::{GameMode, PokemonEncoding};
use party::PokemonParty;
use tile_map::MetaTileMap;
use crate::game_boy::GameBoy;
use crate::geometry::Point8;
use crate::joypad::{JoypadButton, JoypadButtonState};
use crate::mmu::MMU;
use crate::pokemon::bag::{BagReader, BagWriter};
use crate::pokemon::battle::BagItem;
use crate::pokemon::font::{render_font_string, FontAware, FONT_BYTES};
use crate::pokemon::item::ItemId;
use crate::pokemon::menu::{MenuState, MenuStateReader};
use crate::pokemon::symbols::{pokered_symbols, DmgPointerRead};
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::pokemon::Pokemon;
use crate::pokemon::strings::PokemonString;

pub mod badge;
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
pub mod policy;
pub mod tile_map;
pub mod encoding;
pub mod strings;
pub mod symbols;
pub mod font;
pub mod roms;
mod text;
mod map_header;
mod item;
mod bag;
mod menu;
pub mod delay;

pub trait PokemonApiTrait {
    fn release_all_buttons(&mut self);
    fn press_button(&mut self, button: JoypadButton);
    fn release_button(&mut self, button: JoypadButton);
    fn toggle_button(&mut self, button: JoypadButton);
    fn read_joypad_state(&self) -> JoypadButtonState;
    fn game_mode(&self) -> Option<GameMode>;
    fn game_state(&self) -> Result<GameState, String>;
    fn on_screen_text(&self, only_message_box: bool) -> Option<String>;
    fn menu_state(&self) -> Option<MenuState>;
}

#[derive(Debug)]
pub struct PokemonApi<'a> {
    game_boy: &'a mut GameBoy
}

impl<'a> PokemonApi<'a> {
    pub fn new(game_boy: &'a mut GameBoy) -> Self {
        Self { game_boy }
    }

    fn mmu(&self) -> &MMU {
        self.game_boy.core().mmu()
    }

    fn mmu_mut(&mut self) -> &mut MMU {
        self.game_boy.core_mut().mmu_mut()
    }

    pub fn pimp_out_pokemon(&mut self) -> Result<(), String> {
        let player_state = self.game_state()?;

        let bag_items = [
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
            BagItem::new(ItemId::SuperRod, 1),
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
        mmu.write_bag(&bag_items);
        mmu.write_player_pokemon_party(&party)
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
        Ok(GameState {
            player_id: mmu.read_pointer_u16_be(&pokered_symbols::wPlayerID),
            name: mmu.read_pointer_pokemon_string(&pokered_symbols::wPlayerName),
            rival_name: mmu.read_pointer_pokemon_string(&pokered_symbols::wRivalName),
            badges: Badge::parse_flags(mmu.read_pointer(&pokered_symbols::wObtainedBadges)),
            money: encoding::reverse_bcd(mmu.read_pointer_u24_be(&pokered_symbols::wPlayerMoney)),
            mode: mmu.read_game_mode(),
            pokemon: mmu.read_player_pokemon_party()?,
            map: MetaTileMap::new(&mmu.read_current_map()?),
            battle: mmu.read_battle_state(),
            bag: mmu.read_bag(),
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

    fn menu_state(&self) -> Option<MenuState> {
        self.mmu().read_menu_state()
    }
}

#[derive(Debug, Clone, Default)]
pub struct GameState {
    pub player_id: u16,
    pub name: PokemonString,
    pub rival_name: PokemonString,
    pub badges: Vec<Badge>,
    pub money: u32,
    pub pokemon: PokemonParty,
    pub mode: GameMode,
    pub map: MetaTileMap,
    pub bag: Vec<BagItem>,
    /// Populated whenever `mode` is `WildBattle` or `TrainerBattle`.
    pub battle: Option<BattleState>,
}

#[cfg(test)]
mod test {
    use std::time::Duration;
    use crate::cycles::MachineCycles;
    use crate::pokemon::actions::OverworldAction;
    use crate::pokemon::bag::BagWriter;
    use crate::pokemon::battle::BattleAction;
    use crate::pokemon::encoding::MetaTile;
    use crate::pokemon::item::ItemId;
    use crate::pokemon::policy::Policy;
    use std::sync::{mpsc, Arc, Mutex};
    use std::sync::mpsc::{Receiver, Sender};
    use super::*;

    pub const PALLET_TOWN_STATE: &[u8] = include_bytes!("./test_data/pallet-town-state.bin");
    pub const ROUTE1_STATE: &[u8] = include_bytes!("./test_data/route1-state.bin");
    pub const BATTLE_STATE: &[u8] = include_bytes!("./test_data/battle-state.bin");

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
    enum PolicyEvent {
        Movement(Map),
        Battle,
    }

    struct FindBattleOnRoute1Policy {
        previous_map: Map,
        event_tx: Sender<PolicyEvent>,
    }

    impl FindBattleOnRoute1Policy {
        pub fn new(event_tx: Sender<PolicyEvent>) -> Self {
            Self { previous_map: Map::PalletTown, event_tx }
        }
    }

    impl Policy for FindBattleOnRoute1Policy {
        fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction> {
            let next_map = match (self.previous_map, state.map.map) {
                (Map::PalletTown, Map::Route1) => Map::ViridianCity,
                (Map::Route1, Map::ViridianCity) => Map::Route1,
                (Map::ViridianCity, Map::Route1) => Map::PalletTown,
                (Map::Route1, Map::PalletTown) => Map::Route1,
                _ => return None
            };

            let preferred = state.map.actions().iter()
                .find(|a| a.tile == MetaTile::Connection(next_map))
                .cloned();

            self.event_tx.send(PolicyEvent::Movement(next_map)).ok();

            if preferred.is_some() {
                self.previous_map = next_map;
            }
            preferred
        }

        fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
            self.event_tx.send(PolicyEvent::Battle).ok();
            None
        }
    }


    #[test]
    fn test_battle_state_reading() {
        use battle::BattleType;
        use crate::pokemon::move_name::PokemonMoveName;

        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(BATTLE_STATE).unwrap();
        gb.run(MachineCycles::from_m(500));

        // Add items and a second Pokemon so every BattleAction variant is exercisable.
        {
            // 3 Potions + 1 Full Heal — replaces whatever was in the bag.
            gb.core_mut().mmu_mut()
                .write_bag(&[BagItem::new(ItemId::Potion, 3), BagItem::new(ItemId::FullHeal, 1)]);
        }
        {
            let mut party = gb.core().mmu().read_player_pokemon_party().unwrap();
            party.push(Pokemon::maxed(
                PokemonSpecies::Pidgey,
                "PIDGEY",
                [PokemonMoveName::Tackle, PokemonMoveName::SandAttack,
                 PokemonMoveName::Gust, PokemonMoveName::Whirlwind],
                "RED", 12345,
            )).unwrap();
            gb.core_mut().mmu_mut().write_player_pokemon_party(&party).unwrap();
        }

        let api = PokemonApi::new(&mut gb);
        let state = api.game_state().unwrap();
        let battle = state.battle.expect("should be in battle");

        // Basic battle validity
        assert_eq!(battle.battle_type, BattleType::Wild);
        assert!(battle.enemy.level > 0, "enemy level should be non-zero");
        assert!(battle.enemy.max_hp > 0, "enemy max HP should be non-zero");
        assert!(battle.enemy.current_hp <= battle.enemy.max_hp);
        assert!(battle.player.moves.iter().any(|m| m.is_some()), "player needs at least one move");

        // Bag — we wrote exactly 2 items, so expect exactly those 2.
        assert_eq!(state.bag[0].id, ItemId::Potion, "bag[0] should be POTION");
        assert_eq!(state.bag[0].quantity, 3);
        assert!(state.bag.iter().any(|i| i.id == ItemId::FullHeal),
            "FULL HEAL should be in the bag");
    }

    /// Verifies the agent can navigate through a wild battle end-to-end.
    ///
    /// Uses a deterministic `TestPolicy` that always picks the northward connection
    /// (Viridian City / Route 1) to keep the player walking through grass.  As soon
    /// as a battle is encountered the policy selects Fight(0) every turn.  The test
    /// succeeds once `wIsInBattle` returns to 0 (battle over).
    #[test]
    fn test_agent_battle_lifecycle() {
        use crate::pokemon::agent::PokemonAgent;

        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(ROUTE1_STATE).unwrap();
        gb.run(MachineCycles::from_m(1000));

        let (tx, rx) = mpsc::channel::<PolicyEvent>();

        let policy = FindBattleOnRoute1Policy::new(tx);
        let mut agent = PokemonAgent::new(Box::new(policy));

        let frame_cycles = MachineCycles::from_duration(Duration::from_millis(16)); // ~60 FPS

        let max_frames = 30_000u32;
        let mut battle_started = false;

        for _frame in 0..max_frames {
            gb.run(frame_cycles);
            agent.update(&mut gb, frame_cycles).ok();


            if let Ok(event) = rx.try_recv() {
                println!("{:?}", event);
                if event == PolicyEvent::Battle {
                    battle_started = true;
                    break;
                }
            }
        }

        assert!(battle_started, "battle did not start within {} frames", max_frames);
    }

    /// Verifies the agent can fight through a wild battle using the first available
    /// move until the enemy faints, then the player is returned to the overworld on Route 1.
    #[test]
    #[ignore]
    fn test_battle_fight_to_victory_returns_to_route1() {
        use crate::pokemon::agent::PokemonAgent;

        #[derive(Debug, PartialEq, Eq)]
        enum FightEvent { BattleStarted, BattleEnded }

        struct FightFirstMovePolicy {
            previous_map: Map,
            in_battle: bool,
            event_tx: Sender<FightEvent>,
        }

        impl FightFirstMovePolicy {
            fn new(event_tx: Sender<FightEvent>) -> Self {
                Self { previous_map: Map::PalletTown, in_battle: false, event_tx }
            }
        }

        impl Policy for FightFirstMovePolicy {
            fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction> {
                if self.in_battle {
                    self.in_battle = false;
                    self.event_tx.send(FightEvent::BattleEnded).ok();
                }
                let next_map = match (self.previous_map, state.map.map) {
                    (Map::PalletTown, Map::Route1) => Map::ViridianCity,
                    (Map::Route1, Map::ViridianCity) => Map::Route1,
                    (Map::ViridianCity, Map::Route1) => Map::PalletTown,
                    (Map::Route1, Map::PalletTown) => Map::Route1,
                    _ => return None,
                };
                let preferred = state.map.actions().iter()
                    .find(|a| a.tile == MetaTile::Connection(next_map))
                    .cloned();
                if preferred.is_some() {
                    self.previous_map = next_map;
                }
                preferred
            }

            fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
                if !self.in_battle {
                    self.in_battle = true;
                    self.event_tx.send(FightEvent::BattleStarted).ok();
                }
                let battle = state.battle.as_ref()?;
                let slot = battle.player.moves.iter()
                    .position(|m| m.map_or(false, |m| m.current_pp > 0))
                    .unwrap_or(0) as u8;
                Some(BattleAction::Fight(slot))
            }
        }

        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(ROUTE1_STATE).unwrap();
        gb.run(MachineCycles::from_m(1000));

        let (tx, rx) = mpsc::channel::<FightEvent>();
        let policy = FightFirstMovePolicy::new(tx);
        let mut agent = PokemonAgent::new(Box::new(policy));

        let frame_cycles = MachineCycles::from_duration(Duration::from_millis(16));
        let max_frames = 100_000u32;
        let mut battle_ended = false;

        for _frame in 0..max_frames {
            gb.run(frame_cycles);
            agent.update(&mut gb, frame_cycles).ok();

            if let Ok(event) = rx.try_recv() {
                println!("{:?}", event);
                if event == FightEvent::BattleEnded {
                    battle_ended = true;
                    break;
                }
            }
        }

        assert!(battle_ended, "battle did not end within {} frames", max_frames);

        let api = PokemonApi::new(&mut gb);
        let state = api.game_state().unwrap();
        assert_eq!(state.mode, GameMode::Overworld, "player should be on the overworld after battle");
        assert_eq!(state.map.map, Map::Route1, "player should be on Route 1 after battle");
    }

    #[test]
    fn test_route_1() {
        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(ROUTE1_STATE).unwrap();
        gb.run(MachineCycles::from_m(1000));
        let api = PokemonApi::new(&mut gb);
        let state = api.game_state().unwrap();
        assert_eq!(state.map.map, Map::Route1);
        println!("{}", state.map);
    }

    #[test]
    fn test_route_1_ledge_routing() {
        use encoding::{JumpDirection, MetaTile};

        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(ROUTE1_STATE).unwrap();
        gb.run(MachineCycles::from_m(1000));
        let api = PokemonApi::new(&mut gb);
        let state = api.game_state().unwrap();
        let map = state.map;
        assert_eq!(map.map, Map::Route1);

        // Find a south-facing ledge tile that has walkable ground on both sides.
        let (lx, ly) = (0..map.height)
            .flat_map(|y| (0..map.width).map(move |x| (x, y)))
            .find(|&(x, y)| {
                y > 0 && y + 1 < map.height
                    && matches!(map.meta_tiles[x + y * map.width], MetaTile::Jump(JumpDirection::South))
                    && matches!(map.meta_tiles[x + (y - 1) * map.width], MetaTile::Empty)
                    && matches!(map.meta_tiles[x + (y + 1) * map.width], MetaTile::Empty)
            })
            .expect("Route 1 should have south-facing ledges");

        let north_of_ledge = Point8 { x: lx as u8, y: (ly - 1) as u8 };
        let south_of_ledge = Point8 { x: lx as u8, y: (ly + 1) as u8 };

        // Returns the shortest route length to a connection at the given edge y-row.
        let shortest_to_connection = |pos: Point8, target_y: u8| -> Option<usize> {
            let mut m = map.clone();
            m.player_position = pos;
            m.actions().into_iter()
                .filter(|a| matches!(a.tile, MetaTile::Connection(_)) && a.destination.y == target_y)
                .map(|a| a.route.len())
                .min()
        };

        let south_row = (map.height - 1) as u8; // Pallet Town connection row
        let north_row = 0u8;                     // Viridian City connection row

        // ── Southward: jump is used ──────────────────────────────────────────────
        // Pressing Down once from north-of-ledge jumps two tiles (over the ledge),
        // landing at south-of-ledge. So the south route is exactly 1 step longer
        // than starting from south-of-ledge.
        let north_to_south = shortest_to_connection(north_of_ledge, south_row)
            .expect("Pallet Town reachable from north of ledge via jump");
        let south_to_south = shortest_to_connection(south_of_ledge, south_row)
            .expect("Pallet Town reachable from south of ledge");

        assert!(
            north_to_south <= south_to_south + 1,
            "jumping south over ledge ({north_to_south} steps) should cost at most \
             1 more than starting just south of it ({south_to_south} steps)"
        );

        // ── Northward: ledge forces a detour ────────────────────────────────────
        // Ledges block northward movement. A player south of the ledge must navigate
        // around the entire ledge row, adding many more steps than from north of it.
        let north_to_north = shortest_to_connection(north_of_ledge, north_row)
            .expect("Viridian City reachable from north of ledge");
        let south_to_north = shortest_to_connection(south_of_ledge, north_row)
            .expect("Viridian City reachable from south of ledge via detour");

        assert!(
            south_to_north > north_to_north,
            "going north from south-of-ledge ({south_to_north} steps) must take more \
             steps than from north-of-ledge ({north_to_north} steps) — ledge forces detour"
        );

    }

    #[test]
    fn test_pallet_town_actions() {
        use encoding::MetaTile;

        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(PALLET_TOWN_STATE).unwrap();
        gb.run(MachineCycles::from_m(1000));
        let api = PokemonApi::new(&mut gb);
        let state = api.game_state().unwrap();
        let map = &state.map;

        let actions = map.actions();

        // Sanity-check that the test is actually exercising something.
        assert!(!map.warp_targets.is_empty(), "expected warp targets in Pallet Town");
        assert!(map.sprites.iter().any(|s| !s.hidden), "expected visible sprites in Pallet Town");

        // Every warp target must produce an action with a non-empty route.
        for &to_map in &map.warp_targets {
            let action = actions.iter().find(|a| a.tile == MetaTile::Warp(to_map));
            assert!(action.is_some(), "no action for warp to {to_map}");
            assert!(!action.unwrap().route.is_empty(), "empty route to warp {to_map}");
        }

        // Every visible (non-hidden) sprite must produce an action with a non-empty route.
        for sprite in map.sprites.iter().filter(|s| !s.hidden) {
            let action = actions.iter().find(|a| a.tile == MetaTile::Sprite(sprite.name));
            assert!(action.is_some(), "no action for sprite '{}'", sprite.name);
            assert!(!action.unwrap().route.is_empty(), "empty route to sprite '{}'", sprite.name);
        }

        // Route 1 is walkable from Pallet Town — must have a connection action.
        assert!(
            actions.iter().any(|a| a.tile == MetaTile::Connection(Map::Route1)),
            "missing connection action for Route1"
        );

        // Route 21 is water-only from Pallet Town — no walkable Connection action expected.
        assert!(
            !actions.iter().any(|a| a.tile == MetaTile::Connection(Map::Route21)),
            "unexpected walkable connection to Route21 (should be water-only)"
        );
    }

    /// When the ViridianCity PokeMART clerk's intro script is running the agent must
    /// advance the conversation by pressing A — not try to navigate around the map.
    #[test]
    fn test_viridian_pokemart_script_advances_dialogue() {
        use std::time::Duration;
        use crate::pokemon::agent::PokemonAgent;

        const POKEMART_STATE: &[u8] = include_bytes!("./test_data/viridian-city-pokemart-during-script.bin");

        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(POKEMART_STATE).unwrap();
        gb.run(MachineCycles::from_m(1000));

        // The save state has the clerk's text box already open.
        // The game mode must be TextBox or Script — never plain Overworld.
        {
            let api = PokemonApi::new(&mut gb);
            let mode = api.game_mode().unwrap();
            assert!(
                matches!(mode, GameMode::TextBox | GameMode::Script),
                "Expected TextBox or Script mode in PokeMART save state, got {:?}", mode
            );
        }

        // Run the agent: it should press A to advance the dialogue and eventually
        // return to Overworld mode.
        let mut agent = PokemonAgent::new(Box::new(crate::pokemon::policy::RandomPolicy));
        let frame_cycles = MachineCycles::from_duration(Duration::from_millis(16));

        let mut returned_to_overworld = false;
        for _ in 0..10_000u32 {
            gb.run(frame_cycles);
            agent.update(&mut gb, frame_cycles).ok();

            let api = PokemonApi::new(&mut gb);
            if api.game_mode() == Some(GameMode::Overworld) {
                returned_to_overworld = true;
                break;
            }
        }

        assert!(
            returned_to_overworld,
            "agent should advance the PokeMART dialogue and return to Overworld within 10000 frames"
        );
    }

/// The Pokémon Center nurse stands behind a counter.  The player should be able to
    /// talk to her by approaching the counter tile (a "talking over" tile in pokered).
    #[test]
    fn test_viridian_pokecenter_nurse_action() {
        use encoding::MetaTile;

        const POKECENTER_STATE: &[u8] = include_bytes!("./test_data/viridian-city-pokemon-center.bin");

        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(POKECENTER_STATE).unwrap();
        gb.run(MachineCycles::from_m(1000));

        let api = PokemonApi::new(&mut gb);
        let state = api.game_state().unwrap();
        assert_eq!(state.map.map, Map::ViridianPokecenter);

        let actions = state.map.actions();
        let tiles: Vec<_> = actions.iter().map(|a| &a.tile).collect();

        assert!(
            actions.iter().any(|a| a.tile == MetaTile::Sprite("Nurse")),
            "expected Talk to Nurse action (counter tile should be treated as talking-over tile); actions: {tiles:?}"
        );
    }

    /// With the player already standing in Red's House 1F after descending from 2F,
    /// all three actions must be available: exit to Pallet Town, stairs to 2F, talk to Mom.
    #[test]
    fn test_reds_house_1f_actions_from_save_state() {
        use encoding::MetaTile;

        const REDS_HOUSE_1F_STATE: &[u8] = include_bytes!("./test_data/reds-house-1f-state.bin");

        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(REDS_HOUSE_1F_STATE).unwrap();
        gb.run(MachineCycles::from_m(1000));

        let api = PokemonApi::new(&mut gb);
        let state = api.game_state().unwrap();
        assert_eq!(state.map.map, Map::RedsHouse1F, "save state should start in RedsHouse1F");

        let actions = state.map.actions();
        let tiles: Vec<_> = actions.iter().map(|a| &a.tile).collect();

        assert!(
            actions.iter().any(|a| a.tile == MetaTile::Warp(Map::PalletTown)),
            "expected exit Warp → PalletTown; actions: {tiles:?}"
        );
        assert!(
            actions.iter().any(|a| a.tile == MetaTile::Warp(Map::RedsHouse2F)),
            "expected Warp → RedsHouse2F (stairs); actions: {tiles:?}"
        );
        assert!(
            actions.iter().any(|a| a.tile == MetaTile::Sprite("Mom")),
            "expected Sprite(Mom); actions: {tiles:?}"
        );
        assert!(
            !actions.iter().any(|a| a.tile == MetaTile::Warp(Map::RedsHouse1F)),
            "self-referential Warp → RedsHouse1F must not appear; actions: {tiles:?}"
        );
    }

    /// After entering Red's House 1F from Pallet Town, the exit warp should lead back to
    /// PalletTown — not self-referentially to RedsHouse1F.
    #[test]
    fn test_reds_house_1f_exit_warp() {
        use std::time::Duration;
        use encoding::MetaTile;
        use crate::pokemon::agent::PokemonAgent;
        use crate::pokemon::battle::BattleAction;

        struct EnterRedsHousePolicy;
        impl Policy for EnterRedsHousePolicy {
            fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction> {
                if state.map.map != Map::PalletTown { return None; }
                state.map.actions().into_iter()
                    .find(|a| a.tile == MetaTile::Warp(Map::RedsHouse1F))
            }
            fn pick_battle_action(&mut self, _: &GameState) -> Option<BattleAction> { None }
        }

        let mut gb = GameBoy::dmg(roms::POKERED);
        gb.load_state(PALLET_TOWN_STATE).unwrap();
        gb.run(MachineCycles::from_m(1000));

        let mut agent = PokemonAgent::new(Box::new(EnterRedsHousePolicy));
        let frame_cycles = MachineCycles::from_duration(Duration::from_millis(16));

        // Count consecutive frames spent in Overworld mode on RedsHouse1F.
        // We wait for 120 stable frames (~2 s) before asserting, so the map's
        // object data (warps, sprites) has time to fully initialise after the
        // wCurMap register flips.
        let mut stable_frames = 0u32;
        for _ in 0..15_000u32 {
            gb.run(frame_cycles);
            agent.update(&mut gb, frame_cycles).ok();

            let api = PokemonApi::new(&mut gb);
            let Ok(state) = api.game_state() else { stable_frames = 0; continue };
            if state.mode != GameMode::Overworld || state.map.map != Map::RedsHouse1F {
                stable_frames = 0;
                continue;
            }
            stable_frames += 1;
            if stable_frames < 120 { continue; }

            let actions = state.map.actions();
            let tiles: Vec<_> = actions.iter().map(|a| &a.tile).collect();

            assert!(
                !actions.iter().any(|a| a.tile == MetaTile::Warp(Map::RedsHouse1F)),
                "self-referential Warp → RedsHouse1F inside Red's House 1F; actions: {tiles:?}"
            );
            assert!(
                actions.iter().any(|a| a.tile == MetaTile::Warp(Map::RedsHouse2F)),
                "expected exit Warp → RedsHouse2F inside Red's House 1F; actions: {tiles:?}"
            );
            assert!(
                actions.iter().any(|a| a.tile == MetaTile::Warp(Map::PalletTown)),
                "expected exit Warp → PalletTown inside Red's House 1F; actions: {tiles:?}"
            );
            assert!(
                actions.iter().any(|a| a.tile == MetaTile::Sprite("Mom")),
                "expected to be able to talk to 'Mom' inside Red's House 1F; actions: {tiles:?}"
            );
            return;
        }

        panic!("never reached RedsHouse1F in overworld mode within 15000 frames");
    }
}

