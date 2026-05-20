use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver};
use rand::prelude::StdRng;
use rand::seq::IteratorRandom;
use rand::{RngCore, SeedableRng};
use crate::mmu::MMU;
use crate::pokemon::GameState;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::battle::{BattleAction, BattleType};
use crate::pokemon::damage::expected_damage;
use crate::pokemon::data::PokemonNamePicker;
use crate::pokemon::encoding::MetaTile;
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
            println!("\nAvailable actions:");
            for (i, a) in actions.iter().enumerate() {
                let label = match &a.tile {
                    MetaTile::Warp(m)       => format!("Warp → {m}"),
                    MetaTile::Connection(m) => format!("Go to {m}"),
                    MetaTile::Sprite(n)     => format!("Talk to {n}"),
                    other                   => format!("{other}"),
                };
                println!("  {}. {}", i + 1, label);
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
    Navigate { map: Map, strict: bool },
    /// Walk to and interact with a visible sprite by name.
    Interact(MapSprite),
}

impl PolicyStep {
    pub const fn navigate(map: Map) -> Self {
        Self::Navigate { map, strict: true }
    }

    pub const fn navigate_until_interrupted(map: Map) -> Self {
        Self::Navigate { map, strict: false }
    }

    pub const COMPLETE_GAME: &[Self] = &[
        Self::navigate(Map::PalletTown),
        Self::navigate_until_interrupted(Map::Route1),      // triggers Oak's script → lands in OaksLab
        Self::Interact(MapSprite::OAKSLAB_BULBASAUR_POKE_BALL),
        Self::navigate(Map::Route1),      // navigate for real now that Oak is done
    ];
}

pub struct DeterministicPolicy {
    rng: StdRng,
    queue: VecDeque<PolicyStep>,
    world_graph: WorldGraph,
    name_picker: PokemonNamePicker,
}

impl DeterministicPolicy {
    pub fn new(seed: u64, steps: &[PolicyStep], world_graph: WorldGraph) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
            queue: steps.iter().cloned().collect(),
            world_graph,
            name_picker: PokemonNamePicker::seed_from_u64(seed),
        }
    }

    pub fn complete_game(seed: u64, mmu: &MMU) -> Self {
        Self::new(seed, PolicyStep::COMPLETE_GAME, WorldGraph::build(mmu))
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
                PolicyStep::Navigate { map: target, strict } => {
                    if state.map.map == target {
                        self.queue.pop_front();
                        continue;
                    }
                    let path = self.world_graph.shortest_path(state.map.map, target)?;
                    let next_map = path.get(1)?.map;
                    let action = actions.into_iter().find(|a| match a.tile {
                        MetaTile::Connection(m) | MetaTile::Warp(m) => m == next_map,
                        _ => false,
                    });

                    if !strict && action.is_some() {
                        // non-struct navigations are triggered only once
                        self.queue.pop_front();
                    }

                    action
                }
                PolicyStep::Interact(sprite) => {
                    let action = actions.into_iter()
                        .find(|a| a.tile == MetaTile::Sprite(sprite.name));
                    println!("[policy] Interact({}): found={}", sprite.name, action.is_some());
                    if action.is_some() {
                        self.queue.pop_front();
                    }
                    action
                }
            }
        }
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        let battle_state = state.battle.as_ref()?;
        let actions = battle_options(state)?;
        let mut result: Option<BattleAction> = None;
        let mut most_damage = 0;

        // 1. pick the battle move that does the most damage
        for action in actions.iter() {
            if let BattleAction::Fight { battle_move, ..} = action {
                if let Some(damage) = expected_damage(&battle_state.player, battle_move.name, &battle_state.enemy) {
                    if damage > most_damage {
                        result = Some(*action);
                        most_damage = damage;
                    }
                }
            }
        }
        if result.is_some() {
            return result;
        }

        // 2. pick a random battle action
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
}