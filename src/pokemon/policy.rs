use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver};
use rand::prelude::StdRng;
use rand::seq::IteratorRandom;
use rand::SeedableRng;
use crate::mmu::MMU;
use crate::pokemon::GameState;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::badge::Badge;
use crate::pokemon::bag::BagItem;
use crate::pokemon::battle::{BattleAction, BattleType};
use crate::pokemon::damage::pick_best_move;
use crate::pokemon::data::PokemonNamePicker;
use crate::pokemon::tile::MetaTile;
pub use crate::pokemon::item::ItemId;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::world_graph::WorldGraph;

/// Non-blocking policy interface.
///
/// All methods return `Option<_>`. `None` means "not ready yet — ask again next frame".
/// This keeps the game loop running while the policy waits for input.
pub trait Policy {
    fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction>;
    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction>;

    /// Called when the nickname-entry screen opens for `species`.
    ///
    /// - `None`          → not ready yet; will be called again next frame.
    /// - `Some(None)`    → decline a nickname; the game keeps the default species name.
    /// - `Some(Some(s))` → give this nickname (up to 10 characters, A-Z / a-z / 0-9 / common punctuation).
    fn pick_nickname(&mut self, _species: PokemonSpecies) -> Option<Option<String>> {
        Some(None) // default: keep the default species name
    }

    /// Called when the mart's Buy/Sell/Quit menu first appears.
    ///
    /// - `None`       → not ready yet; will be called again next frame.
    /// - `Some(None)` → do not buy anything.
    /// - `Some(Some(item))` → buy the item.
    fn pick_mart_purchase(&mut self, _state: &GameState) -> Option<Option<BagItem>> {
        Some(None) // default: open the mart but buy nothing
    }

    fn is_exhausted(&self) -> bool {
        false
    }
}

// ── Random (always-ready) ─────────────────────────────────────────────────────

#[derive(Default)]
pub struct RandomPolicy;

impl Policy for RandomPolicy {
    fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction> {
        state.map.actions().into_iter().choose(&mut rand::rng())
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        battle_options(state)?.into_iter().choose(&mut rand::rng())
    }
}

// ── Console (human-driven, non-blocking) ─────────────────────────────────────

/// Displays a numbered menu, then reads the user's choice from stdin on a
/// background thread so the game loop is never blocked.
pub struct ConsolePolicy {
    overworld_rx:   Option<Receiver<usize>>,
    battle_rx:      Option<Receiver<usize>>,
    nickname_rx:    Option<Receiver<Option<String>>>,
    ow_menu_shown:  bool,
    btl_menu_shown: bool,
    /// Tiles shown when the last overworld menu was displayed; used to match
    /// the user's selection by destination rather than by list index, since
    /// the action list can reorder between display and selection.
    ow_shown_tiles: Vec<MetaTile>,
}

impl Default for ConsolePolicy {
    fn default() -> Self {
        Self {
            overworld_rx:   None,
            battle_rx:      None,
            nickname_rx:    None,
            ow_menu_shown:  false,
            btl_menu_shown: false,
            ow_shown_tiles: vec![],
        }
    }
}

impl Policy for ConsolePolicy {
    fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction> {
        let actions = state.map.actions();
        if actions.is_empty() { return None; }

        if !self.ow_menu_shown || self.overworld_rx.is_none() {
            println!("\nYou are on {} at {}. Available actions:", state.map.map, state.map.player_position);
            for (i, a) in actions.iter().enumerate() {
                println!("  {}. {}", i + 1, a);
            }
            let max = actions.len();
            // Cache the destinations so we can match by tile, not index.
            self.ow_shown_tiles = actions.iter().map(|a| a.tile.clone()).collect();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                loop {
                    print!("Pick (1-{max}): ");
                    io::stdout().flush().ok();
                    let mut line = String::new();
                    if io::stdin().read_line(&mut line).is_err() { break; }
                    if let Ok(n) = line.trim().parse::<usize>() {
                        if n >= 1 && n <= max { tx.send(n).ok(); break; }
                    }
                    println!("Invalid.");
                }
            });
            self.overworld_rx = Some(rx);
            self.ow_menu_shown = true;
        }

        if let Ok(n) = self.overworld_rx.as_ref().unwrap().try_recv() {
            let chosen_tile = self.ow_shown_tiles.get(n - 1).cloned();
            self.overworld_rx  = None;
            self.ow_menu_shown = false;
            self.ow_shown_tiles.clear();
            if let Some(tile) = chosen_tile {
                return actions.into_iter().find(|a| a.tile == tile);
            }
            return None;
        }
        None
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        if !self.btl_menu_shown || self.battle_rx.is_none() {
            let battle_state = state.battle.as_ref()?;

            println!("\n═══ BATTLE ═══");
            println!("Enemy:  {:?} Lv.{}  HP {}/{}  {}",
                battle_state.enemy.species, battle_state.enemy.level,
                battle_state.enemy.current_hp, battle_state.enemy.stats.hp,
                battle_state.enemy.status);
            println!("Player: {:?} Lv.{}  HP {}/{}  {}",
                battle_state.player.species, battle_state.player.level,
                battle_state.player.current_hp, battle_state.player.stats.hp,
                battle_state.player.status);
            println!("\nBattle actions:");

            let opts = battle_options(state)?;
            for (i, battle_action) in opts.iter().enumerate() {
                println!("  {}. {}", i + 1, battle_action);
            }

            let max = opts.len();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                loop {
                    print!("Pick (1-{max}): ");
                    io::stdout().flush().ok();
                    let mut line = String::new();
                    if io::stdin().read_line(&mut line).is_err() { break; }
                    if let Ok(n) = line.trim().parse::<usize>() {
                        if n >= 1 && n <= max { tx.send(n).ok(); break; }
                    }
                    println!("Invalid.");
                }
            });
            self.battle_rx    = Some(rx);
            self.btl_menu_shown = true;
        }

        if let Ok(n) = self.battle_rx.as_ref().unwrap().try_recv() {
            self.battle_rx     = None;
            self.btl_menu_shown = false;
            let mut opts = battle_options(state)?;
            return Some(opts.remove(n - 1));
        }
        None
    }

    fn pick_nickname(&mut self, species: PokemonSpecies) -> Option<Option<String>> {
        if self.nickname_rx.is_none() {
            println!("\nGive a nickname to {}?", species);
            println!("  Enter a nickname (up to 10 chars), or press Enter to keep the default.");
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                print!("> ");
                io::stdout().flush().ok();
                let mut line = String::new();
                if io::stdin().read_line(&mut line).is_err() { return; }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    tx.send(None).ok();
                } else {
                    tx.send(Some(trimmed.to_string())).ok();
                }
            });
            self.nickname_rx = Some(rx);
        }

        if let Ok(decision) = self.nickname_rx.as_ref().unwrap().try_recv() {
            self.nickname_rx = None;
            return Some(decision);
        }
        None
    }
}


fn battle_options(state: &GameState) -> Option<Vec<BattleAction>> {
    let battle_state = state.battle.as_ref()?;

    let mut opts = battle_state.player.available_battle_moves();

    for (i, item) in state.bag.iter().enumerate() {
        opts.push(BattleAction::UseItem { slot: i as u8, item: item.clone() });
    }

    for (i, pokemon) in state.pokemon.iter().enumerate() {
        if i == battle_state.active_party_slot as usize { continue; }
        if pokemon.stats.hp == 0 { continue; }
        opts.push(BattleAction::SwitchPokemon { slot: i as u8, pokemon: pokemon.summary() });
    }

    if battle_state.battle_type == BattleType::Wild {
        opts.push(BattleAction::Run);
    }

    Some(opts)
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PolicyStep {
    Goto { map: Map, strict: bool },
    /// Walk to and interact with a visible sprite by name.
    Interact(MapSprite),
    DefeatGymLeader { leader: MapSprite, badge: Badge },
    /// Walk in grass and throw Pokéballs until a Pokémon is caught. TODO add location in case we blackout
    CatchPokemon { species: PokemonSpecies, on_map: Map },
    /// Walk in grass until the leading party member reaches at least this level. TODO add location in case we blackout
    GrindUntilLevel { target_level: u8, on_map: Map },
    /// Buy item from the currently open Pokémart (must follow an Interact with the clerk).
    BuyFromMart { map: Map, item: BagItem },
}

impl PolicyStep {
    pub const fn goto(map: Map) -> Self {
        Self::Goto { map, strict: true }
    }

    pub const fn soft_goto(map: Map) -> Self {
        Self::Goto { map, strict: false }
    }

    pub fn complete_game_steps() -> Vec<Self> { vec![
        // Try to leave Pallet Town, Oak stops you and gives you a starter Pokémon
        Self::goto(Map::PalletTown),
        Self::soft_goto(Map::Route1),
        Self::Interact(MapSprite::OAKSLAB_SQUIRTLE_POKE_BALL),

        // Pick up Oak's parcel from Viridian Pokémart
        Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
        Self::goto(Map::ViridianMart),

        // Deliver parcel to get the Pokédex
        Self::Interact(MapSprite::OAKSLAB_OAK1),

        // Heal at Mom's
        Self::Interact(MapSprite::REDSHOUSE1F_MOM),

        // Get the town map from Daisy
        Self::Interact(MapSprite::BLUESHOUSE_DAISY1),

        // Walk back through Route 1, heal up and buy Pokéballs
        Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
        Self::BuyFromMart { item: BagItem::new(ItemId::PokeBall, 5), map: Map::ViridianMart },

        // Catch a Pidgey
        Self::CatchPokemon { species: PokemonSpecies::Pidgey, on_map: Map::Route1 },
        Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),

        // Grind until Squirtle is level 10
        Self::GrindUntilLevel { target_level: 8, on_map: Map::Route1 },
        Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
        Self::GrindUntilLevel { target_level: 9, on_map: Map::Route1 },
        Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
        Self::GrindUntilLevel { target_level: 10, on_map: Map::Route1 },
        Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),

        // Walk through Viridian Forest to Pewter City
        Self::Interact(MapSprite::PEWTERPOKECENTER_NURSE),

        // ── Defeat Brock ──
        Self::DefeatGymLeader { leader: MapSprite::PEWTERGYM_BROCK, badge: Badge::BoulderBadge },
        Self::Interact(MapSprite::PEWTERPOKECENTER_NURSE),

        // // ── Walk through Mt Moon to Cerulean City ──
        // Self::goto(Map::MtMoonPokecenter),
        // Self::Interact(MapSprite::MTMOONPOKECENTER_NURSE),
        // Self::goto(Map::CeruleanCity),
        //
        // // ── Defeat Misty ──
        // Self::goto(Map::CeruleanPokecenter),
        // Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
        // Self::goto(Map::CeruleanGym),
        // Self::Interact(MapSprite::CERULEANGYM_MISTY),
        // Self::goto(Map::CeruleanPokecenter),
        // Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
    ] }
}

pub struct DeterministicPolicy {
    rng: StdRng,
    queue: VecDeque<PolicyStep>,
    world_graph: WorldGraph,
    name_picker: PokemonNamePicker,
}

impl DeterministicPolicy {
    pub fn new(seed: u64, steps: impl IntoIterator<Item = PolicyStep>, world_graph: WorldGraph) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
            queue: steps.into_iter().collect(),
            world_graph,
            name_picker: PokemonNamePicker::seed_from_u64(seed),
        }
    }

    pub fn complete_game(seed: u64, mmu: &MMU) -> Self {
        Self::new(seed, PolicyStep::complete_game_steps(), WorldGraph::build(mmu))
    }
}

impl Policy for DeterministicPolicy {

    fn pick_overworld_action(&mut self, state: &GameState) -> Option<OverworldAction> {
        let actions = state.map.actions();
        let action_tiles: Vec<_> = actions.iter().map(|a| format!("{:?}", a.tile)).collect();
        println!("[policy] map={} actions=[{}]", state.map.map, action_tiles.join(", "));
        loop {
            let step = self.queue.front()?.clone();
            return match step {
                PolicyStep::Goto { map: target, strict } => {
                    if state.map.map == target {
                        self.queue.pop_front();
                        continue;
                    }
                    let action = self.world_graph.pick_shortest_path_action(&actions, target);
                    if !strict && action.is_some() {
                        // a non-strict goto action can be interrupted
                        self.queue.pop_front();
                    }
                    action
                },
                PolicyStep::CatchPokemon { species, on_map } => {
                    if state.map.map != on_map {
                        let action = self.world_graph.pick_shortest_path_action(&actions, on_map);
                        if action.is_none() {
                            println!("[policy] want to catch pokemon {} in {}, but no path there!", species, on_map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if state.pokedex_owned.contains(&species) {
                        // caught the pokemon (note this only works once for each species)
                        self.queue.pop_front();
                        continue;
                    } else if let Some(action) = actions.iter()
                        .find(|a| a.tile == MetaTile::Grass) {
                        if state.bag.best_pokeball().is_some() {
                            // walk in grass
                            Some(action.clone())
                        } else {
                            println!("[policy] want to catch a {}, but no Pokéballs left!", species);
                            self.queue.pop_front();
                            continue;
                        }
                    } else {
                        println!("[policy] want to catch a {}, but no grass nearby!", species);
                        self.queue.pop_front();
                        continue;
                    }
                },
                PolicyStep::GrindUntilLevel { target_level, on_map } => {
                    if state.map.map != on_map {
                        let action = self.world_graph.pick_shortest_path_action(&actions, on_map);
                        if action.is_none() {
                            println!("[policy] want to grind until level {} in {}, but no path there!", target_level, on_map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if let Some(pokemon) = state.pokemon.get(0) {
                        if pokemon.level >= target_level {
                            self.queue.pop_front();
                            continue;
                        } else if let Some(action) = actions.iter()
                            .find(|a| a.tile == MetaTile::Grass) {
                            // walk in grass
                            Some(action.clone())
                        } else {
                            println!("[policy] cannot level up a Pokemon, no grass nearby!");
                            self.queue.pop_front();
                            continue;
                        }
                    } else {
                        println!("[policy] no Pokemon in party to level up");
                        self.queue.pop_front();
                        continue;
                    }
                },
                PolicyStep::DefeatGymLeader { leader, badge } => {
                    if state.badges.contains(Badge::BoulderBadge) {
                        self.queue.pop_front();
                        continue;
                    } else if state.map.map != leader.map() {
                        let action = self.world_graph.pick_shortest_path_action(&actions, leader.map());
                        if action.is_none() {
                            println!("[policy] want to defeat {} to obtain the {}, but no path there!", leader, badge);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        let action = actions.iter()
                            .find(|a| a.tile == MetaTile::Sprite(leader.name));
                        if action.is_some() {
                            self.queue.pop_front();
                        }
                        action.cloned()
                    }
                },
                PolicyStep::Interact(sprite) => {
                    let map = sprite.map();
                    if state.map.map != map {
                        let action = self.world_graph.pick_shortest_path_action(&actions, map);
                        if action.is_none() {
                            println!("[policy] want to interact with {} on {}, but no path there!", sprite, map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        let action = actions.iter()
                            .find(|a| a.tile == MetaTile::Sprite(sprite.name));
                        if action.is_some() {
                            self.queue.pop_front();
                        }
                        action.cloned()
                    }
                }
                PolicyStep::BuyFromMart { item, map } => {
                    if state.map.map != map {
                        let action = self.world_graph.pick_shortest_path_action(&actions, map);
                        if action.is_none() {
                            println!("[policy] want to buy {} from {} but no path there!", item, map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        // If triggered in the overworld, talk to the "Clerk" sprite to initiate the pokemart agent
                        let action = actions.iter()
                            .find(|a| matches!(a.tile, MetaTile::Sprite(sprite) if sprite == "Clerk"));

                        if action.is_none() {
                            println!("[policy] BuyFromMart step encountered in pick_overworld_action and no clerk available — skipping");
                            self.queue.pop_front();
                            continue;
                        }

                        action.cloned()
                    }
                }
            }
        }
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        let battle_state = state.battle.as_ref()?;
        let actions = battle_options(state)?;

        // When catching, throw a Pokéball immediately if one is available.
        if let Some(PolicyStep::CatchPokemon { species, .. }) = self.queue.front() {
            if battle_state.battle_type == BattleType::Wild && battle_state.enemy.species == *species {
                if let Some(best_pokeball) = state.bag.best_pokeball() {
                    if let Some(use_pokeball_action) = actions.iter()
                        .find(|a| matches!(a, BattleAction::UseItem { item, .. } if item.id == best_pokeball.id )) {

                        // If enemy HP > 50%, try to weaken it first with the move that does
                        // the most damage without knocking the Pokémon out.
                        if battle_state.enemy.remaining_hp() > 0.5 {
                            if let Some(mv) = pick_best_move(&battle_state, &actions, true) {
                                println!("[policy] enemy HP > 50% — weakening before throwing ball");
                                return Some(mv);
                            }
                        }

                        return Some(use_pokeball_action.clone());
                    } else {
                        println!("[policy] want to catch a {}, but no use Pokéball actions were provided!", species);
                    }
                } else {
                    println!("[policy] want to catch a {}, but no Pokéballs left!", species);
                }
            }
        }

        // 1. pick the strongest move
        let result = pick_best_move(&battle_state, &actions, false);
        if result.is_some() {
            return result;
        }

        // 2. pick a random fight action
        let random_battle = actions.iter()
            .filter(|a| matches!(a, BattleAction::Fight { .. }))
            .choose(&mut self.rng);
        if random_battle.is_some() {
            return random_battle.cloned();
        }

        // 3. pick any random action
        actions.into_iter().choose(&mut self.rng)
    }

    fn pick_nickname(&mut self, _species: PokemonSpecies) -> Option<Option<String>> {
        let name = self.name_picker.pick().to_string();
        println!("[policy] pick name={}", name);
        Some(Some(name))
    }

    fn pick_mart_purchase(&mut self, _state: &GameState) -> Option<Option<BagItem>> {
        let result = match self.queue.front() {
            Some(PolicyStep::BuyFromMart { item, .. }) => {
                println!("[policy] BuyFromMart: {:?}", item);
                Some(*item)
            }
            _ => {
                println!("[policy] pick_mart_purchase called but no BuyFromMart step queued — returning None");
                None
            },
        };

        if result.is_some() {
            self.queue.pop_front();
        }

        Some(result)
    }

    fn is_exhausted(&self) -> bool {
        self.queue.is_empty()
    }
}