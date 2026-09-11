use std::collections::{VecDeque, HashSet};
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver};
use rand::prelude::StdRng;
use rand::seq::IteratorRandom;
use rand::SeedableRng;
use crate::pokemon::{GameState, PokemonApi};
use crate::pokemon::actions::OverworldAction;
use gb::geometry::Point8;
use crate::pokemon::badge::Badge;
use crate::pokemon::bag::BagItem;
use crate::pokemon::battle::{is_ghost_battle, BattleAction, BattleType};
use crate::pokemon::damage::{expected_damage, is_damaging_move, pick_best_move};
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::data::PokemonNamePicker;
use crate::pokemon::tile::MetaTile;
use crate::pokemon::tile_map::MetaTileMap;
pub use crate::pokemon::item::ItemId;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::world_graph::WorldGraph;

/// The one [`Policy::name`] that means a model is playing.
pub const LLM_POLICY_NAME: &str = "llm";

/// Non-blocking policy interface.
pub trait Policy {
    /// What this decider is called: `"llm"`, `"random"`, `"console"` or `"scripted"`.
    fn name(&self) -> &'static str;

    fn pick_overworld_action(&mut self, state: &GameState, world_graph: &WorldGraph) -> Option<OverworldAction>;
    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction>;

    /// What the player is called, asked once when a new game starts.
    fn player_name(&self) -> Option<String> {
        None
    }

    /// Called when the nickname-entry screen opens for `species`.
    fn pick_nickname(&mut self, _species: PokemonSpecies) -> Option<Option<String>> {
        Some(None) // default: keep the default species name
    }

    /// Called when the mart's Buy/Sell/Quit menu first appears.
    fn pick_mart_purchase(&mut self, _state: &GameState) -> Option<Option<BagItem>> {
        Some(None) // default: open the mart but buy nothing
    }

    /// Another purchase for the same mart visit, asked once the Buy/Sell/Quit menu is back.
    fn next_mart_purchase(&mut self) -> Option<BagItem> {
        None
    }

    fn pick_move_to_forget(
        &mut self,
        _party_slot: usize,
        _current_moves: &[PokemonMove],
        _new_move: PokemonMoveName,
    ) -> Option<Option<usize>> {
        Some(None) // default: never drop an existing move
    }

    /// Called each idle overworld tick.
    fn pick_field_move(&mut self, _state: &GameState) -> Option<FieldMove> {
        None
    }

    /// Called for every event the agent emits, as it is emitted and before it is buffered.
    fn on_event(&mut self, _event: &crate::pokemon::agent::AgentEvent) {}

    /// Called at the top of every policy poll, before any `pick_*` for that decision point.
    fn service_tools(&mut self, _state: &GameState, _api: &mut PokemonApi<'_>, _graph: &WorldGraph) {}

    /// How much emulated time may pass without a policy poll before the watchdog wakes this one.
    fn stuck_timeout(&self) -> Option<std::time::Duration> {
        None
    }

    /// The agent has gone [`Self::stuck_timeout`] without asking anything, and is asking now.
    fn pick_unstick(&mut self, _state: &GameState, _jam: Jam<'_>) {}

    fn restart(&mut self, _run_dir: Option<&std::path::Path>) {}

    /// `POST /api/clear`: forget what this policy remembers about the game, keep the game.
    fn clear_conversation(&mut self, _run_dir: Option<&std::path::Path>) -> Result<(), String> {
        Err("this run is not being played by a model, so there is no conversation to clear".to_string())
    }

    /// Raw button presses for the agent to deliver at the top of its next tick.
    fn take_manual_input(&mut self) -> Vec<gb::joypad::JoypadButton> {
        Vec::new()
    }

    fn is_exhausted(&self) -> bool {
        false
    }

    /// Returns the number of steps remaining in the policy queue, if known.
    fn steps_remaining(&self) -> Option<usize> {
        None
    }

    /// Whether the current step may run long without advancing the queue (a grind, a catch).
    fn current_step_is_long_running(&self) -> bool {
        false
    }
}

/// What the watchdog knows about the jam it is waking the policy for.
#[derive(Debug, Clone, Copy)]
pub struct Jam<'a> {
    /// What the agent thinks it is doing (`PokemonAgent::state_debug`).
    pub agent_state: &'a str,
    /// Emulated time since the agent last asked the policy anything.
    pub stuck_for: std::time::Duration,
}

// ── Random (always-ready) ─────────────────────────────────────────────────────

/// Picks uniformly from whatever the agent offers; `integration_tests::soak` fuzzes with it.
#[derive(Default)]
pub struct RandomPolicy {
    /// `None` draws from the thread RNG, so no two runs are alike.
    rng: Option<StdRng>,
    seed: Option<u64>,
    /// The ids of the last [`EXPLORE_MEMORY`] overworld actions taken, oldest first.
    recent: VecDeque<String>,
    explore: bool,
}

/// How many overworld choices back [`RandomPolicy::exploring`] remembers.
const EXPLORE_MEMORY: usize = 24;

/// What one occurrence in [`RandomPolicy::recent`] multiplies an action's weight by.
const EXPLORE_DECAY: f64 = 0.35;

impl RandomPolicy {
    /// A policy whose choices are fixed by `seed` — the same seed always plays the same game.
    pub fn seeded(seed: u64) -> Self {
        Self { rng: Some(StdRng::seed_from_u64(seed)), seed: Some(seed), ..Self::default() }
    }

    /// [`Self::seeded`], but biased away from what it has just done on the overworld.
    pub fn exploring(seed: u64) -> Self {
        Self { explore: true, ..Self::seeded(seed) }
    }

    /// The key an action is remembered by: the id the model chooses from (`PalletTown:5,6:Warp`).
    fn action_key(action: &OverworldAction) -> String {
        action.id()
    }

    /// `EXPLORE_DECAY ^ (times this action appears in the window)`.
    fn novelty_weight(&self, action: &OverworldAction) -> f64 {
        let key = Self::action_key(action);
        let seen = self.recent.iter().filter(|k| **k == key).count();
        EXPLORE_DECAY.powi(seen as i32)
    }

    fn choose_weighted(rng: &mut StdRng, actions: Vec<OverworldAction>, weights: &[f64])
        -> Option<OverworldAction> {
        let total: f64 = weights.iter().sum();
        if !total.is_finite() || total <= 0.0 {
            return actions.into_iter().choose(rng);
        }
        let mut draw = rand::Rng::random_range(rng, 0.0..total);
        for (action, weight) in actions.into_iter().zip(weights) {
            draw -= weight;
            if draw <= 0.0 {
                return Some(action);
            }
        }
        // Floating-point slack only.
        None
    }

    fn remember(&mut self, action: &OverworldAction) {
        self.recent.push_back(Self::action_key(action));
        while self.recent.len() > EXPLORE_MEMORY {
            self.recent.pop_front();
        }
    }
}

/// What [`RandomPolicy`] might call itself.
const RANDOM_NAMES: &[&str] = &[
    "DICEY", "CHANCE", "FLUKE", "RANDOM", "SHUFFLE", "ROLL", "COINTOS", "HAZARD", "LOTTO", "WHIM",
    "SCATTER", "DRIFT", "ENTROPY", "JITTER", "NOISE", "STRAY",
];

impl Policy for RandomPolicy {
    fn name(&self) -> &'static str { "random" }

    /// Off `self.rng` when there is one, so a seeded run stays a seeded run.
    fn player_name(&self) -> Option<String> {
        // The seed picks the name directly, so the stream the run plays from is not advanced.
        let index = match &self.rng {
            Some(_) => self.seed.unwrap_or(0) as usize % RANDOM_NAMES.len(),
            None => rand::random::<u64>() as usize % RANDOM_NAMES.len(),
        };
        Some(RANDOM_NAMES[index].to_string())
    }

    /// Uniform, unless built by [`RandomPolicy::exploring`], which weights recent choices down.
    fn pick_overworld_action(&mut self, state: &GameState, _world_graph: &WorldGraph) -> Option<OverworldAction> {
        let actions = state.map.actions();
        // Weighed before `self.rng` is borrowed mutably, because `novelty_weight` reads `self`.
        let weights: Option<Vec<f64>> =
            self.explore.then(|| actions.iter().map(|a| self.novelty_weight(a)).collect());
        let chosen = match (&mut self.rng, weights) {
            (Some(rng), Some(weights)) => Self::choose_weighted(rng, actions, &weights),
            (Some(rng), None) => actions.into_iter().choose(rng),
            (None, _) => actions.into_iter().choose(&mut rand::rng()),
        };
        if self.explore {
            if let Some(action) = &chosen {
                self.remember(action);
            }
        }
        chosen
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        let options = battle_options(state)?;
        match &mut self.rng {
            Some(rng) => options.into_iter().choose(rng),
            None => options.into_iter().choose(&mut rand::rng()),
        }
    }
}

// ── Console (human-driven, non-blocking) ─────────────────────────────────────

/// Reads the choice from stdin on a background thread so the game loop never blocks.
pub struct ConsolePolicy {
    overworld_rx:   Option<Receiver<usize>>,
    battle_rx:      Option<Receiver<usize>>,
    nickname_rx:    Option<Receiver<Option<String>>>,
    ow_menu_shown:  bool,
    btl_menu_shown: bool,
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
    fn name(&self) -> &'static str { "console" }

    /// The one policy with a person behind it, so the trainer card says so.
    fn player_name(&self) -> Option<String> {
        Some("HUMAN".to_string())
    }

    fn pick_overworld_action(&mut self, state: &GameState, _world_graph: &WorldGraph) -> Option<OverworldAction> {
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

/// Whether a step can be interrupted by a walk to a Pokémon Centre and still finish afterwards.
fn step_finds_its_own_way_back(step: &PolicyStep) -> bool {
    matches!(step,
        PolicyStep::GrindUntilLevel { .. } | PolicyStep::CatchPokemon { .. }
        | PolicyStep::SweepDex { .. } | PolicyStep::Goto { .. }
        | PolicyStep::DefeatGymLeader { .. } | PolicyStep::CutTree { .. })
}

/// Whether the run should walk to a Pokémon Centre *before* the next battle rather than after it.
fn needs_a_centre(state: &GameState, grinding: bool) -> bool {
    let Some(lead) = state.pokemon.get(0) else { return false };
    if lead.current_hp == 0 { return true; }

    // A grind goes home on empty PP, not on low.
    let (pp, max) = lead.moves.iter().flatten()
        .filter(|m| is_damaging_move(m.name))
        .fold((0u32, 0u32), |(have, cap), m| (have + m.pp as u32, cap + m.name.metadata().pp as u32));
    let dry = match grinding { true => pp == 0, false => pp * 5 <= max };
    if max > 0 && dry { return true; }

    let heals = |id: ItemId| match id {
        ItemId::FullRestore | ItemId::MaxPotion => u32::MAX,
        ItemId::HyperPotion => 200,
        ItemId::SuperPotion => 50,
        ItemId::Potion => 20,
        _ => 0,
    };
    let best = state.bag.iter().filter(|i| i.quantity > 0).map(|i| heals(i.id)).max().unwrap_or(0);
    let missing = lead.stats.hp.saturating_sub(lead.current_hp) as u32;
    lead.current_hp as u32 * 3 < lead.stats.hp as u32 && best < missing
}

/// Every party member at full HP, full PP and no status — what a Pokémon Centre leaves behind.
fn party_is_fresh(state: &GameState) -> bool {
    state.pokemon.iter().all(|p| p.current_hp == p.stats.hp
        && p.status == crate::pokemon::status::PokemonStatus::None
        && p.moves.iter().flatten().all(|m| m.pp == m.name.metadata().pp))
}

pub(crate) fn battle_options(state: &GameState) -> Option<Vec<BattleAction>> {
    let battle_state = state.battle.as_ref()?;

    // Safari Zone battles have their own menu (no FIGHT/PKMN/ITEM).
    if battle_state.battle_type == BattleType::Safari {
        return Some(vec![
            BattleAction::SafariBall,
            BattleAction::SafariBait,
            BattleAction::SafariRock,
            BattleAction::Run,
        ]);
    }

    // A ghost battle offers only `Run`, the one thing the cartridge will do.
    if is_ghost_battle(state.map.map, &state.bag, battle_state.battle_type) {
        return Some(vec![BattleAction::Run]);
    }

    let mut opts = battle_state.player.available_battle_moves();

    // Every move at zero PP is Struggle; reading that as no move stops the run in silence.
    if opts.is_empty() {
        opts.extend(battle_state.player.moves.iter().enumerate()
            .filter_map(|(i, m)| m.map(|battle_move| BattleAction::Fight { slot: i as u8, battle_move })));
    }

    for (i, item) in state.bag.iter().enumerate() {
        opts.push(BattleAction::UseItem { slot: i as u8, item: item.clone() });
    }

    for (i, pokemon) in state.pokemon.iter().enumerate() {
        if i == battle_state.active_party_slot as usize { continue; }
        if pokemon.current_hp == 0 { continue; }
        opts.push(BattleAction::SwitchPokemon { slot: i as u8, pokemon: pokemon.summary() });
    }

    if battle_state.battle_type == BattleType::Wild {
        opts.push(BattleAction::Run);
    }

    Some(opts)
}

/// Whether the PP left across damaging moves is under 20% of their maximum.
fn all_damaging_moves_low_pp(actions: &[BattleAction]) -> bool {
    const MIN_PP_PCT: f32 = 0.2;

    let mut total_damaging_pp = 0;
    let mut total_max_pp = 0;

    for action in actions.iter() {
        if let BattleAction::Fight { battle_move, .. } = action {
            if is_damaging_move(battle_move.name) {
                total_damaging_pp += battle_move.pp as usize;
                total_max_pp += battle_move.name.metadata().pp as usize;
            }
        }
    }

    if total_max_pp == 0 {
        return false;
    }

    (total_damaging_pp as f32 / total_max_pp as f32) < MIN_PP_PCT
}

/// How a step names a party member.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PartyRef {
    /// The member at this party index.
    Slot(u8),
    /// The first member of this species.
    Species(PokemonSpecies),
    /// The first member of any of these species — an evolution line named as one mon.
    Line(&'static [PokemonSpecies]),
}

impl PartyRef {
    /// The party index this reference points at now, or `None`.
    pub fn resolve(&self, state: &GameState) -> Option<u8> {
        match *self {
            Self::Slot(slot) => (usize::from(slot) < state.pokemon.len()).then_some(slot),
            Self::Species(species) => state.pokemon.iter()
                .position(|p| p.species == species)
                .map(|i| i as u8),
            Self::Line(line) => state.pokemon.iter()
                .position(|p| line.contains(&p.species))
                .map(|i| i as u8),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PolicyStep {
    Goto { map: Map, strict: bool },
    /// Take one transition to `to_map`; `to_position` picks among several landings (Mt Moon).
    EnterMap { to_map: Map, to_position: Option<Point8> },
    /// Walk to and interact with a visible sprite by name.
    Interact(MapSprite),
    /// Like `Interact`, but pops after a bounded wait if the sprite is walled off.
    InteractIfReachable(MapSprite),
    /// Walk to the map's PC tile, face it, and press A (e.g. Bill's cell-separator PC).
    UsePc { map: Map },

    Fly { to: Map },
    UseFlash { slot: u8 },
    /// A one-NPC script that opens the party menu on `slot`: the Day Care or the Name Rater.
    PartyScript { script: crate::pokemon::postgame::gifts::PartyScript, slot: u8 },
    /// Use bag `item` from the overworld on nothing, a party member, or one of its moves.
    UseBagItem { item: ItemId, target: crate::pokemon::postgame::items::UseTarget },
    /// Like [`Self::EnterMap`], but pops with a printed reason after a bounded wait.
    EnterMapIfReachable { to_map: Map },
    /// Pace `on_map`'s grass into a wild battle and use each of `items` once, in order.
    UseItemsInBattle { on_map: Map, items: &'static [ItemId] },

    /// Move `qty` of `item` between the bag and PC item storage, at the PC on `map`.
    UseItemPc { op: crate::pokemon::postgame::item_storage::PcItemOp, item: ItemId, qty: u8, map: Map },
    /// Deposit, withdraw, release or change box at the PC on `map`, via `BILL's PC`.
    UsePcBox { op: crate::pokemon::postgame::pc_box::PcBoxOp, map: Map },
    /// A fishing session on `map` with `rod`, running until `goal` is met.
    Fish {
        rod: crate::pokemon::postgame::fishing::Rod,
        map: Map,
        goal: crate::pokemon::postgame::fishing::FishGoal,
    },
    /// Hunt `targets` in the Safari Zone on `map`, for at most `max_trips` paid entries.
    SafariHunt { targets: &'static [PokemonSpecies], map: Map, max_trips: u32 },
    /// Walk out of the Safari Zone to the gate mat, from whichever area a hunt ended in.
    SafariExit,
    /// Buy Game Corner coins at the counter until at least `target` are held.
    BuyGameCoins { target: u16 },
    /// Sell `item` to the mart clerk on `map`.
    SellToMart { map: Map, item: BagItem },
    /// Buy `prize` from a Game Corner prize vendor.
    RedeemPrize { prize: crate::pokemon::postgame::game_corner::Prize },
    /// Pick up an item sprite, staying on this step until the sprite is gone.
    CollectItem(MapSprite),
    DefeatGymLeader { leader: MapSprite, badge: Badge },
    /// Battle a fixed trainer (an Elite Four member) by walking into its line of sight.
    BattleTrainer { trainer: MapSprite },
    /// Walk in grass and throw Pokéballs until a Pokémon is caught.
    CatchPokemon { species: PokemonSpecies, on_map: Map, ball: Option<ItemId> },
    /// Catch every unowned species whose share of `on_map`'s encounters is at least `min_share`%.
    SweepDex { on_map: Map, min_share: u8, ball: Option<ItemId> },
    /// Make `target` the lead, written straight to RAM with no menus.
    MovePokemonToFront { target: PartyRef },
    /// Walk in grass until `target` reaches at least `target_level`.
    GrindUntilLevel { target_level: u8, on_map: Map, target: PartyRef },
    /// Buy item from the currently open Pokémart (must follow an Interact with the clerk).
    BuyFromMart { map: Map, item: BagItem },
    /// Teach HM/TM `item` to `target` from the overworld.
    TeachMove { item: ItemId, target: PartyRef },
    /// Use evolution `stone` from the bag on `target`.
    EvolveWithStone { stone: ItemId, target: PartyRef },
    /// Use a Rare Candy from the bag on the party member in `slot`.
    UseRareCandy { slot: u8 },
    /// Toss `item` from the bag to free a slot.
    TossItem { item: ItemId },
    /// Use Dig (TM28) from the party menu with `target`: Gen 1's reusable Escape Rope.
    Dig { target: PartyRef },
    /// Cut down a tree blocking the way on `map` (requires Cut + the Cascade Badge).
    CutTree { map: Map },
    /// Activate Strength using the party mon `target` names (an HM-slave that knows it).
    UseStrength { target: PartyRef },
    /// Push a boulder onto the Strength switch at `switch`, solving the floor's puzzle.
    SolveBoulders { switch: gb::geometry::Point8, boulder: Option<gb::geometry::Point8> },
    /// Push a boulder into floor `hole` so it falls to the floor below (Victory Road 3F).
    DropBoulderInHole { hole: gb::geometry::Point8, boulder: Option<gb::geometry::Point8> },
    /// Check the Vermilion Gym's two switch cans in turn, unlocking the door to Lt. Surge.
    SolveTrashCans,
    /// Face the hidden switch at `at` on `map` and press A until a reachable warp to `reveals`
    /// appears (the Game Corner poster).
    FlipSwitch { map: Map, at: Point8, reveals: Map },
    /// Inside an elevator room, use the floor panel to travel to the floor at menu index `floor`.
    UseElevator { panel: Point8, floor: u8 },
    /// Face sprite `target` and use bag `item` on it from the field.
    UseFieldItem { item: ItemId, target: MapSprite },
    /// Buy `drink` at the machine at `at`: its menu opens on the cheapest drink, so A buys it.
    UseVendingMachine { at: Point8, drink: ItemId },
}

/// A non-walking action the agent performs itself (menus, field moves) for the front step.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FieldMove {
    /// Reorder the party so `slot` becomes the lead (RAM write, no menus).
    ReorderParty { slot: u8 },
    /// Teach `item` (an HM/TM) to the party member in `target_slot` via the bag.
    TeachMove { item: ItemId, target_slot: u8 },
    /// Use evolution `stone` on the party member in `target_slot`.
    EvolveWithStone { stone: ItemId, target_slot: u8, evolve_from: PokemonSpecies },
    /// Use the Cut field move on the tree the player is currently facing.
    CutTree,
    /// Walk to the tile at `target` and press A.
    CheckTrashCan { target: gb::geometry::Point8, facing: Option<crate::pokemon::map_metadata::PlayerFacingDirection> },
    /// Drive the elevator menu at `panel` to index `floor`, then ride the redirected warp out.
    UseElevator { panel: gb::geometry::Point8, floor: u8 },
    /// Face the sprite at `target`, then use bag `item` on it (START → ITEM → select → USE).
    UseFieldItem { item: ItemId, target: gb::geometry::Point8 },
    /// A party-menu script: walk to `npc`, a (tile, facing) pair from `actions()` because
    /// `route_to_face_dir` cannot reach behind a counter, then pick `slot`.
    UsePartyScript {
        script: crate::pokemon::postgame::gifts::PartyScript,
        slot: u8,
        npc: (gb::geometry::Point8, crate::pokemon::map_metadata::PlayerFacingDirection),
    },
    /// A field move from the party menu: the mon at `slot`, the entry at `move_index`.
    UseFieldMove { slot: u8, move_index: u8 },
    /// Toss `item` from the bag (START → ITEM → the item → TOSS → quantity → YES) to free a slot.
    TossItem { item: ItemId },
    /// Move `qty` of `item` between the bag and PC item storage at the PC at `pc`.
    UseItemPc { op: crate::pokemon::postgame::item_storage::PcItemOp, item: ItemId, qty: u8, pc: gb::geometry::Point8 },
    /// Walk to the PC at `pc` and drive Bill's PC box menus.
    UsePcBox { op: crate::pokemon::postgame::pc_box::PcBoxOp, pc: gb::geometry::Point8 },

    Fly { to: Map },
    /// Cast `rod` once at the water tile `at`.
    Fish { rod: crate::pokemon::postgame::fishing::Rod, at: gb::geometry::Point8 },
    /// Walk to the clerk at `clerk` and sell `item`.
    SellToMart { item: BagItem, clerk: (gb::geometry::Point8, crate::pokemon::map_metadata::PlayerFacingDirection) },
    /// Walk to the prize vendor and buy `prize` with coins.
    RedeemPrize { prize: crate::pokemon::postgame::game_corner::Prize },
    /// Use bag `item` on `target` from the overworld.
    UseBagItem { item: ItemId, target: crate::pokemon::postgame::items::UseTarget },
    /// Shove the boulder at `boulder` one tile in `dir`; Strength must be armed.
    PushBoulder { boulder: gb::geometry::Point8, dir: gb::joypad::JoypadButton },
}

/// The Pokémon Mansion floors, whose statue switches trigger only when faced from below.
fn is_mansion_floor(map: Map) -> bool {
    matches!(map, Map::PokemonMansion1F | Map::PokemonMansion2F | Map::PokemonMansion3F | Map::PokemonMansionB1F)
}

pub fn hm_move(item: ItemId) -> Option<PokemonMoveName> {
    match item {
        ItemId::Hm01Cut => Some(PokemonMoveName::Cut),
        ItemId::Hm02Fly => Some(PokemonMoveName::Fly),
        ItemId::Hm03Surf => Some(PokemonMoveName::Surf),
        ItemId::Hm04Strength => Some(PokemonMoveName::Strength),
        ItemId::Hm05Flash => Some(PokemonMoveName::Flash),
        ItemId::Tm14Blizzard => Some(PokemonMoveName::Blizzard), // TM (consumed on use); the E4 Lance answer
        ItemId::Tm28Dig => Some(PokemonMoveName::Dig),           // TM (consumed on use); the way out of a cave
        _ => None,
    }
}

/// Moves with their own party-menu field entry, from pokered `FieldMoveDisplayData`.
fn is_field_move(name: PokemonMoveName) -> bool {
    matches!(name, PokemonMoveName::Cut | PokemonMoveName::Fly | PokemonMoveName::Surf
        | PokemonMoveName::Strength | PokemonMoveName::Flash | PokemonMoveName::Dig
        | PokemonMoveName::Teleport | PokemonMoveName::Softboiled)
}

/// Where `want` sits in `slot`'s field-move menu; 0 when the mon or move is missing, which a
/// lone HM carrier would use anyway.
pub(crate) fn field_move_index(state: &GameState, slot: u8, want: PokemonMoveName) -> u8 {
    state.pokemon.get(slot as usize).map_or(0, |mon| field_move_index_of(mon, want))
}

/// The party member that knows `want`, and `want`'s row in that member's field-move box.
pub(crate) fn field_move_carrier(
    state: &GameState,
    want: PokemonMoveName,
) -> Option<(u8, u8)> {
    state.pokemon.iter().enumerate()
        .find(|(_, p)| p.moves.iter().flatten().any(|m| m.name == want))
        .map(|(i, p)| (i as u8, field_move_index_of(p, want)))
}

/// `want`'s row in `mon`'s field-move box.
pub(crate) fn field_move_index_of(mon: &crate::pokemon::pokemon::Pokemon, want: PokemonMoveName) -> u8 {
    mon.moves.iter().flatten().map(|m| m.name).filter(|&n| is_field_move(n))
        .position(|n| n == want).unwrap_or(0) as u8
}

impl PolicyStep {

    pub const fn goto(map: Map) -> Self {
        Self::Goto { map, strict: true }
    }

    pub const fn soft_goto(map: Map) -> Self {
        Self::Goto { map, strict: false }
    }

    /// Explicit single forward map transition (any warp/connection to `map`).
    pub const fn enter(map: Map) -> Self {
        Self::EnterMap { to_map: map, to_position: None }
    }

    /// Bank `qty` of `item` in PC item storage; `map` must have a PC. A partial deposit frees no
    /// bag slot, so pass `u8::MAX` for all of it.
    pub const fn deposit_item(item: ItemId, qty: u8, map: Map) -> Self {
        Self::UseItemPc { op: crate::pokemon::postgame::item_storage::PcItemOp::Deposit, item, qty, map }
    }

    /// Take `qty` of `item` back out of PC item storage.
    pub const fn withdraw_item(item: ItemId, qty: u8, map: Map) -> Self {
        Self::UseItemPc { op: crate::pokemon::postgame::item_storage::PcItemOp::Withdraw, item, qty, map }
    }

    /// A transition to `map` landing at raw `(x, y)`. A preference: an unreachable landing falls
    /// through to any crossing.
    pub const fn enter_at(map: Map, x: u8, y: u8) -> Self {
        Self::EnterMap { to_map: map, to_position: Some(Point8 { x, y }) }
    }

    /// Mt Moon 1F → B2F for the Helix Fossil → Route 4 → Cerulean; starts in Mt Moon 1F.
    pub fn mt_moon_traversal() -> Vec<Self> { vec![
        Self::enter_at(Map::MtMoonB1F, 5, 5),
        Self::enter_at(Map::MtMoonB2F, 21, 17),
        Self::CollectItem(MapSprite::MTMOONB2F_HELIX_FOSSIL),
        Self::enter_at(Map::MtMoonB1F, 23, 3),
        Self::enter(Map::Route4),
        Self::enter(Map::CeruleanCity),
    ] }

    /// Bill's House for the SS Ticket, from inside: his Pokémon, the PC, then Bill. His exit
    /// aborts an early `Interact`, so the talk repeats; extra talks are harmless.
    pub fn bill_ss_ticket_steps() -> Vec<Self> {
        let mut steps = vec![
            Self::Interact(MapSprite::BILLSHOUSE_BILL_POKEMON),
            Self::UsePc { map: Map::BillsHouse },
        ];
        steps.extend(std::iter::repeat(Self::Interact(MapSprite::BILLSHOUSE_BILL1)).take(8));
        steps
    }

    /// Heal the party at the Vermilion Pokémon Center and return to Vermilion City.
    fn heal_at_vermilion() -> Vec<Self> {
        vec![
            Self::enter(Map::VermilionPokecenter),
            Self::Interact(MapSprite::VERMILIONPOKECENTER_NURSE),
            Self::enter(Map::VermilionCity),
        ]
    }

    /// Board the S.S. Anne, beat every cabin trainer and the rival, and take HM01 from the captain.
    pub fn ss_anne_steps() -> Vec<Self> {
        let mut s = vec![];

        // ── 1F cabins (4 trainers) ──
        s.extend(Self::heal_at_vermilion());
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::SSAnne1F)]);
        s.extend([
            Self::enter_at(Map::SSAnne1FRooms, 0, 0),   Self::Interact(MapSprite::SSANNE1FROOMS_GENTLEMAN1), Self::enter(Map::SSAnne1F),
            Self::enter_at(Map::SSAnne1FRooms, 10, 0),  Self::Interact(MapSprite::SSANNE1FROOMS_GENTLEMAN2), Self::enter(Map::SSAnne1F),
            Self::enter_at(Map::SSAnne1FRooms, 10, 10), Self::Interact(MapSprite::SSANNE1FROOMS_YOUNGSTER),
                                                        Self::Interact(MapSprite::SSANNE1FROOMS_COOLTRAINER_F), Self::enter(Map::SSAnne1F),
        ]);
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity), Self::enter(Map::VermilionPokecenter), Self::Interact(MapSprite::VERMILIONPOKECENTER_NURSE), Self::enter(Map::VermilionCity), ]); // disembark + heal

        // ── B1F cabins (6 trainers) ──
        s.extend(Self::heal_at_vermilion());
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::SSAnne1F), Self::enter(Map::SSAnneB1F)]);
        s.extend([
            Self::enter_at(Map::SSAnneB1FRooms, 2, 5),  Self::Interact(MapSprite::SSANNEB1FROOMS_SAILOR5),
                                                        Self::Interact(MapSprite::SSANNEB1FROOMS_FISHER), Self::enter(Map::SSAnneB1F),
            Self::enter_at(Map::SSAnneB1FRooms, 12, 5), Self::Interact(MapSprite::SSANNEB1FROOMS_SAILOR3), Self::enter(Map::SSAnneB1F),
            Self::enter_at(Map::SSAnneB1FRooms, 22, 5), Self::Interact(MapSprite::SSANNEB1FROOMS_SAILOR4), Self::enter(Map::SSAnneB1F),
            Self::enter_at(Map::SSAnneB1FRooms, 2, 15), Self::Interact(MapSprite::SSANNEB1FROOMS_SAILOR1),
                                                        Self::Interact(MapSprite::SSANNEB1FROOMS_SAILOR2), Self::enter(Map::SSAnneB1F),
        ]);
        s.extend([Self::enter(Map::SSAnne1F), Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity), Self::enter(Map::VermilionPokecenter), Self::Interact(MapSprite::VERMILIONPOKECENTER_NURSE), Self::enter(Map::VermilionCity), ]); // disembark + heal

        // ── 2F cabins (4 trainers) + Bow (2 trainers, via 3F) ──
        s.extend(Self::heal_at_vermilion());
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::SSAnne1F), Self::enter(Map::SSAnne2F)]);
        s.extend([
            Self::enter_at(Map::SSAnne2FRooms, 12, 5), Self::Interact(MapSprite::SSANNE2FROOMS_GENTLEMAN1),
                                                       Self::Interact(MapSprite::SSANNE2FROOMS_FISHER), Self::enter(Map::SSAnne2F),
            Self::enter_at(Map::SSAnne2FRooms, 2, 15), Self::Interact(MapSprite::SSANNE2FROOMS_GENTLEMAN2),
                                                       Self::Interact(MapSprite::SSANNE2FROOMS_COOLTRAINER_F), Self::enter(Map::SSAnne2F),
        ]);
        s.extend([
            Self::enter(Map::SSAnne3F), Self::enter(Map::SSAnneBow),
            Self::Interact(MapSprite::SSANNEBOW_SAILOR2), Self::Interact(MapSprite::SSANNEBOW_SAILOR3),
            Self::enter(Map::SSAnne3F), Self::enter(Map::SSAnne2F),
        ]);
        s.extend([Self::enter(Map::SSAnne1F), Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity), Self::enter(Map::VermilionPokecenter), Self::Interact(MapSprite::VERMILIONPOKECENTER_NURSE), Self::enter(Map::VermilionCity), ]); // disembark + heal

        // ── Rival + Captain (HM01) ──
        s.extend(Self::heal_at_vermilion());
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::SSAnne1F), Self::enter(Map::SSAnne2F)]);
        s.push(Self::enter(Map::SSAnneCaptainsRoom)); // rival battle triggers on approach to the (36,4) warp
        s.extend(std::iter::repeat(Self::Interact(MapSprite::SSANNECAPTAINSROOM_CAPTAIN)).take(4));
        // ── Disembark: the ship leaves once HM01 is taken ──
        s.extend([
            Self::enter(Map::SSAnne2F), Self::enter(Map::SSAnne1F),
            Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity),
            Self::enter(Map::VermilionPokecenter), Self::Interact(MapSprite::VERMILIONPOKECENTER_NURSE), Self::enter(Map::VermilionCity), 
        ]);
        s
    }

    /// Cerulean → Nugget Bridge → Bill → Misty → Vermilion by Route 5–6, catching the Cut carrier.
    /// The trashed house's back door is the only way into Cerulean's Route 5 terrace.
    pub fn cerulean_to_vermilion_steps() -> Vec<Self> {
        let mut steps = vec![
            Self::enter(Map::CeruleanCity),
            // Poké Balls for the Cut carrier on Route 25.
            Self::enter(Map::CeruleanMart),
            Self::BuyFromMart { item: BagItem::new(ItemId::PokeBall, 6), map: Map::CeruleanMart },
            // Potions before the nine bridge trainers, at the last counter before them.
            Self::BuyFromMart { item: BagItem::new(ItemId::Potion, 10), map: Map::CeruleanMart },
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::Route24),
            Self::enter(Map::Route25),
            // Back across the bridge to a Centre before Bill, because what runs out here is PP.
            Self::enter(Map::Route24),
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::Route24),
            Self::enter(Map::Route25),
            Self::enter(Map::BillsHouse),
        ];
        steps.extend(Self::bill_ss_ticket_steps());
        steps.extend([
            Self::enter(Map::Route25),
            // ── The Cut carrier.
            Self::CatchPokemon { species: PokemonSpecies::Oddish, on_map: Map::Route25,
                                 ball: Some(ItemId::PokeBall) },
            Self::enter(Map::Route24),
            // Misty comes after the bridge and this grind.
            Self::GrindUntilLevel { target_level: 24, on_map: Map::Route24, target: Self::STARTER_LINE },
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
            Self::enter(Map::CeruleanCity),
            Self::DefeatGymLeader { leader: MapSprite::CERULEANGYM_MISTY, badge: Badge::CascadeBadge },
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::CeruleanTrashedHouse),   // front door (main terrace, ~27,11)
            Self::enter_at(Map::CeruleanCity, 27, 9), // back door lands in the Route-5 terrace
            Self::enter(Map::Route5),
            Self::enter(Map::UndergroundPathRoute5),
            Self::enter(Map::UndergroundPathNorthSouth),
            Self::enter(Map::UndergroundPathRoute6),
            Self::enter(Map::Route6),
            Self::enter(Map::VermilionCity),
        ]);
        steps
    }

    /// Thunder Badge: teach Cut and Dig, cut the gym tree, solve the trash cans, beat Lt. Surge.
    pub fn thunder_badge_steps() -> Vec<Self> {
        let mut s = Self::heal_at_vermilion();
        s.extend([
            // The carrier, not the lead.
            Self::TeachMove { item: ItemId::Hm01Cut, target: Self::CUT_SLAVE },
            Self::TeachMove { item: ItemId::Tm28Dig, target: Self::STARTER_LINE },
            Self::CutTree { map: Map::VermilionCity },
            Self::enter(Map::VermilionGym),
            Self::SolveTrashCans,
            Self::DefeatGymLeader { leader: MapSprite::VERMILIONGYM_LT_SURGE, badge: Badge::ThunderBadge },
        ]);
        s
    }

    /// Back to Cerulean by the Underground Path, the only way north while Saffron is shut.
    pub fn back_to_cerulean_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::VermilionCity), // exit the gym into the Cut-tree enclosure
            Self::CutTree { map: Map::VermilionCity },
        ];
        s.extend(Self::heal_at_vermilion());
        s.extend(Self::heal_at_vermilion());
        s.extend([
            Self::enter(Map::VermilionMart),
            Self::BuyFromMart { item: BagItem::new(ItemId::SuperPotion, 10), map: Map::VermilionMart },
            Self::enter(Map::VermilionCity),
            Self::enter(Map::Route6),
            Self::enter(Map::UndergroundPathRoute6),
            Self::enter(Map::UndergroundPathNorthSouth),
            Self::enter(Map::UndergroundPathRoute5),
            Self::enter(Map::Route5),
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
            Self::enter(Map::CeruleanCity),
        ]);
        s
    }

    /// Rock Tunnel's warp maze, Route 10 north to south; starts on Route 10 from Route 9.
    pub fn rock_tunnel_traversal() -> Vec<Self> { vec![
        Self::enter_at(Map::RockTunnel1F, 15, 3),   // Route 10 north entrance
        Self::enter_at(Map::RockTunnelB1F, 33, 25),
        Self::enter_at(Map::RockTunnel1F, 5, 3),
        Self::enter_at(Map::RockTunnelB1F, 23, 11),
        Self::enter_at(Map::RockTunnel1F, 37, 17),
        Self::enter_at(Map::Route10, 8, 53),        // south exit (→ Lavender)
    ] }

    /// Cerulean City (main terrace, post-Thunder) → Lavender Town.
    pub fn cerulean_to_lavender_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::CeruleanTrashedHouse),   // main terrace front door
            Self::enter_at(Map::CeruleanCity, 27, 9), // back door → Route-9 terrace
            Self::enter(Map::Route9),
            Self::CutTree { map: Map::Route9 },        // cut the (5,8) tree boxing the west pocket
            Self::enter(Map::Route10),
            // Heal at the mouth: a black-out mid-tunnel cannot resume the warp chain.
            Self::enter(Map::RockTunnelPokecenter),
            Self::Interact(MapSprite::ROCKTUNNELPOKECENTER_NURSE),
            Self::enter(Map::Route10),
        ];
        s.extend(Self::rock_tunnel_traversal());
        s.extend([
            Self::enter(Map::LavenderTown),
            Self::enter(Map::LavenderPokecenter),
            Self::Interact(MapSprite::LAVENDERPOKECENTER_NURSE),
            Self::enter(Map::LavenderTown),
        ]);
        s
    }

    /// Lavender → Celadon under Route 7–8: every Saffron gate wants a drink sold only in Celadon.
    pub fn lavender_to_celadon_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::Route8),
            Self::enter(Map::UndergroundPathRoute8),
            Self::enter(Map::UndergroundPathWestEast),
            Self::enter(Map::UndergroundPathRoute7),
            Self::enter(Map::Route7),
            Self::enter(Map::CeladonCity),
            Self::enter(Map::CeladonPokecenter),
            Self::Interact(MapSprite::CELADONPOKECENTER_NURSE),
            Self::enter(Map::CeladonCity),
        ]
    }

    /// Rainbow Badge: cut the trees sealing the gym and beat Erika.
    pub fn celadon_rainbow_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::CeladonPokecenter),
            Self::Interact(MapSprite::CELADONPOKECENTER_NURSE),
            Self::enter(Map::CeladonCity),
            Self::CutTree { map: Map::CeladonCity },   // cut the trees sealing the gym entrance
            Self::enter(Map::CeladonGym),
            // The garden paths are blocked by cuttable trees (GYM tile $50, pokered `cut.asm`).
            Self::CutTree { map: Map::CeladonGym },
            Self::DefeatGymLeader { leader: MapSprite::CELADONGYM_ERIKA, badge: Badge::RainbowBadge },
        ]
    }

    /// Erika's gym → Rocket Hideout B1F: re-cut the regrown trees, beat the poster's Rocket.
    pub fn rocket_hideout_entrance_steps() -> Vec<Self> {
        let mut s = vec![
            // Beating Erika reloads the map, which regrows the garden trees around the player.
            Self::CutTree { map: Map::CeladonGym },
            Self::enter(Map::CeladonCity),          // exit the gym into the (regrown) tree enclosure
            Self::CutTree { map: Map::CeladonCity }, // re-cut to reach the rest of the city
            Self::enter(Map::CeladonPokecenter),
            Self::Interact(MapSprite::CELADONPOKECENTER_NURSE),
            Self::enter(Map::CeladonCity),
            Self::enter(Map::GameCorner),
        ];
        // The Rocket on (9,5) guards the poster at (9,4) and vanishes when beaten.
        s.extend([
            Self::Interact(MapSprite::GAMECORNER_ROCKET),
            Self::FlipSwitch { map: Map::GameCorner, at: Point8 { x: 9, y: 4 }, reveals: Map::RocketHideoutB1F },
            Self::enter(Map::RocketHideoutB1F),
        ]);
        s
    }

    /// Down the spinner floors B2F/B3F to B4F for the Lift Key (`MetaTileMap::spinners`).
    pub fn lift_key_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::RocketHideoutB2F),
            Self::enter(Map::RocketHideoutB3F),
            Self::enter(Map::RocketHideoutB4F),
        ];
        s.extend(std::iter::repeat(Self::Interact(MapSprite::ROCKETHIDEOUTB4F_ROCKET3)).take(3));
        s.push(Self::CollectItem(MapSprite::ROCKETHIDEOUTB4F_LIFT_KEY));
        s
    }

    /// The Lift Key, then the elevator to Giovanni's B4F room for the Silph Scope.
    pub fn silph_scope_steps() -> Vec<Self> {
        let mut s = Self::lift_key_steps();
        s.extend([
            // B2F's elevator warp is not gated by the Rocket door, unlike B1F's.
            Self::enter(Map::RocketHideoutB3F),
            Self::enter(Map::RocketHideoutB2F),
            Self::enter(Map::RocketHideoutElevator),
            // Panel bg-event at (1,1); floors are B1F(0)/B2F(1)/B4F(2) — pick B4F.
            Self::UseElevator { panel: Point8 { x: 1, y: 1 }, floor: 2 },
            // Trainers stay put when beaten, so one talk each suffices.
            Self::Interact(MapSprite::ROCKETHIDEOUTB4F_ROCKET1),
            Self::Interact(MapSprite::ROCKETHIDEOUTB4F_ROCKET2),
            // Giovanni vanishes when beaten, revealing the Scope.
            Self::Interact(MapSprite::ROCKETHIDEOUTB4F_GIOVANNI),
            Self::CollectItem(MapSprite::ROCKETHIDEOUTB4F_SILPH_SCOPE),
        ]);
        s
    }

    /// Silph Scope in hand: out of the hideout to Lavender, up Pokémon Tower to rescue Mr. Fuji.
    pub fn poke_flute_steps() -> Vec<Self> {
        let mut s = vec![
            // Out: elevator to B2F, up to B1F, the Game Corner, Celadon; then heal.
            Self::enter(Map::RocketHideoutElevator),
            Self::UseElevator { panel: Point8 { x: 1, y: 1 }, floor: 1 }, // B2F = menu index 1
            // B1F is two halves split by the wall at row 16; B2F (27,8) leads to the north one.
            Self::EnterMap { to_map: Map::RocketHideoutB1F, to_position: Some(Point8 { x: 23, y: 2 }) },
            Self::enter(Map::GameCorner),
            Self::enter(Map::CeladonCity),
            Self::enter(Map::CeladonPokecenter),
            Self::Interact(MapSprite::CELADONPOKECENTER_NURSE),
            Self::enter(Map::CeladonCity),
        ];
        // Celadon → Lavender via the Route 7–8 Underground Path (reverse of lavender_to_celadon).
        s.extend([
            Self::enter(Map::Route7),
            Self::enter(Map::UndergroundPathRoute7),
            Self::enter(Map::UndergroundPathWestEast),
            Self::enter(Map::UndergroundPathRoute8),
            Self::enter(Map::Route8),
            Self::enter(Map::LavenderTown),
            // Heal: no Centre in the tower, and a black-out cannot resume the Mr. Fuji rescue.
            Self::enter(Map::LavenderPokecenter),
            Self::Interact(MapSprite::LAVENDERPOKECENTER_NURSE),
            // Super Potions too: full HP at the door is not enough.
            Self::enter(Map::LavenderMart),
            Self::BuyFromMart { item: BagItem::new(ItemId::SuperPotion, 10), map: Map::LavenderMart },
            Self::enter(Map::LavenderTown),
        ]);
        // Climb the tower.
        s.extend([
            Self::enter(Map::PokemonTower1F),
            Self::enter(Map::PokemonTower2F),
            Self::enter(Map::PokemonTower3F),
            Self::enter(Map::PokemonTower4F),
            // The only PP in Kanto the route can reach before the Elite Four.
            Self::CollectItem(MapSprite::POKEMONTOWER4F_ELIXER),
            Self::enter(Map::PokemonTower5F),
            Self::enter(Map::PokemonTower6F),
            // The Rare Candy ball at (6,8) is the only way to the Marowak trigger and 7F stairs.
            Self::CollectItem(MapSprite::POKEMONTOWER6F_RARE_CANDY),
            Self::enter(Map::PokemonTower7F),
        ]);
        // 7F: beat the three Rockets, then Mr. Fuji, whose script warps the player home.
        s.extend([
            Self::Interact(MapSprite::POKEMONTOWER7F_ROCKET1),
            Self::Interact(MapSprite::POKEMONTOWER7F_ROCKET2),
            Self::Interact(MapSprite::POKEMONTOWER7F_ROCKET3),
            // Re-assert the floor before the rescue, so a black-out is survivable.
            Self::goto(Map::PokemonTower7F),
            Self::Interact(MapSprite::POKEMONTOWER7F_MR_FUJI),
        ]);
        for _ in 0..4 { s.push(Self::Interact(MapSprite::MRFUJISHOUSE_MR_FUJI)); }
        s
    }

    /// Wake Route 12's Snorlax with the Poké Flute; the step pops when its sprite goes.
    pub fn snorlax_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::LavenderTown), // leave Mr. Fuji's house
            // Heal: the tower wore the party down and the Route 12–15 trainers are next.
            Self::enter(Map::LavenderPokecenter),
            Self::Interact(MapSprite::LAVENDERPOKECENTER_NURSE),
            Self::enter(Map::LavenderTown),
            Self::enter(Map::Route12),      // south connection off Lavender (lands at the north tip)
            // The Route 12 gate blocks the road: in by the north door, out by the south.
            Self::enter(Map::Route12Gate1F),
            Self::EnterMap { to_map: Map::Route12, to_position: Some(Point8 { x: 10, y: 21 }) },
            Self::UseFieldItem { item: ItemId::PokeFlute, target: MapSprite::ROUTE12_SNORLAX },
        ]
    }

    /// Soul Badge: Route 12 → 15 → Fuchsia, fighting line-of-sight trainers on the way, then Koga.
    pub fn soul_badge_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::Route13),
            // (19,8): the nearest crossing lands at (19,6), a pocket a Bird Keeper seals.
            Self::EnterMap { to_map: Map::Route14, to_position: Some(Point8 { x: 19, y: 8 }) },
            Self::enter(Map::Route15),
            // Route 15 also has a gate building walling off the Fuchsia (west) connection.
            Self::enter(Map::Route15Gate1F),
            Self::EnterMap { to_map: Map::Route15, to_position: Some(Point8 { x: 7, y: 8 }) },
            Self::enter(Map::FuchsiaCity),
            Self::enter(Map::FuchsiaPokecenter),
            Self::Interact(MapSprite::FUCHSIAPOKECENTER_NURSE),
            Self::enter(Map::FuchsiaCity),
            Self::enter(Map::FuchsiaGym),
            Self::DefeatGymLeader { leader: MapSprite::FUCHSIAGYM_KOGA, badge: Badge::SoulBadge },
        ]
    }

    pub fn safari_zone_surf_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::FuchsiaCity),       // out of Koga's gym
            Self::enter(Map::SafariZoneGate),
            Self::enter(Map::SafariZoneCenter),  // pays 500 via the join prompt, auto-walks in
            // West is reached Center → East → North → West, the only land route round the water.
            Self::enter(Map::SafariZoneEast),
            Self::enter(Map::SafariZoneNorth),
            // The eastern North→West pair lands on a shelf ledged off from the Gold Teeth.
            Self::enter_at(Map::SafariZoneWest, 21, 0),
            Self::CollectItem(MapSprite::SAFARIZONEWEST_GOLD_TEETH),
            Self::enter(Map::SafariZoneSecretHouse),
            Self::Interact(MapSprite::SAFARIZONESECRETHOUSE_FISHING_GURU), // hands over HM03 Surf
            // Surf goes on the starter: Blastoise learns Surf and Strength itself.
            Self::TeachMove { item: ItemId::Hm03Surf, target: Self::STARTER_LINE },
        ]
    }

    /// The Gold Teeth to the Warden for HM04; a trip that times out lands at the gate all the same.
    pub fn safari_zone_strength_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::SafariZoneWest),    // out of the secret house
            // Center is split by water: the North entrance lands in a pocket cut off from the gate.
            Self::enter(Map::SafariZoneNorth),
            Self::enter(Map::SafariZoneEast),
            Self::enter(Map::SafariZoneCenter),
            Self::enter(Map::SafariZoneGate),
            Self::enter(Map::FuchsiaCity),
            Self::enter(Map::WardensHouse),
            Self::Interact(MapSprite::WARDENSHOUSE_WARDEN), // give Gold Teeth → HM04 Strength
            // HM04 stays in the bag.
        ]
    }

    /// Into Saffron: Fresh Water from Celadon's roof machine for the Route 7 gate guard.
    pub fn saffron_entry_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::FuchsiaCity), // out of the Warden's house
            Self::enter(Map::Route15),      // from Fuchsia: lands on the west side of the Route-15 gate
            // Through the Route 15 gate west to east.
            Self::enter(Map::Route15Gate1F),
            Self::EnterMap { to_map: Map::Route15, to_position: Some(Point8 { x: 14, y: 8 }) },
            Self::enter(Map::Route14),
            Self::enter(Map::Route13),
            Self::enter(Map::Route12),      // from Route 13: lands south of the Route-12 gate
            // Through the Route 12 gate south to north.
            Self::enter(Map::Route12Gate1F),
            Self::EnterMap { to_map: Map::Route12, to_position: Some(Point8 { x: 10, y: 15 }) },
            Self::enter(Map::LavenderTown),
            // Lavender's nearest Route 8 crossing (0,11) jams; take (0,9).
            Self::EnterMap { to_map: Map::Route8, to_position: Some(Point8 { x: 59, y: 8 }) },
        ];
        s.extend(Self::lavender_to_celadon_steps());
        // Into the Mart, up to the roof, buy a Fresh Water from the vending machine.
        s.extend([
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonMart2F),
            Self::enter(Map::CeladonMart3F),
            Self::enter(Map::CeladonMart4F),
            Self::enter(Map::CeladonMart5F),
            Self::enter(Map::CeladonMartRoof),
            // The bag first binds here, so the toss goes here.
            Self::TossItem { item: ItemId::Tm24Thunderbolt },
            Self::UseVendingMachine { at: Point8 { x: 10, y: 1 }, drink: ItemId::FreshWater },
            // Back down and out to Celadon, then east through the Route-7 gate into Saffron.
            Self::enter(Map::CeladonMart5F),
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonCity),
            Self::enter(Map::Route7),
            Self::enter(Map::Route7Gate),        // west door
            Self::EnterMap { to_map: Map::Route7, to_position: Some(Point8 { x: 18, y: 10 }) },
            Self::enter(Map::SaffronCity),
        ]);
        s
    }

    /// Silph Co: the elevator to 5F for the Card Key. Panel at (3,0), menu 1F=0 … 11F=10.
    pub fn silph_co_card_key_steps() -> Vec<Self> {
        vec![
            // Heal: Silph Co has no Centre and what runs out in it is PP.
            Self::enter(Map::SaffronPokecenter),
            Self::Interact(MapSprite::SAFFRONPOKECENTER_NURSE),
            Self::enter(Map::SaffronCity),
            Self::enter(Map::SaffronMart),
            Self::BuyFromMart { item: BagItem::new(ItemId::HyperPotion, 15), map: Map::SaffronMart },
            Self::enter(Map::SaffronCity),
            Self::enter(Map::SilphCo1F),
            Self::enter(Map::SilphCoElevator),                          // step onto the (20,0) $58 warp tile
            Self::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 4 }, // 5F = menu index 4
            // The Card Key pocket is reached only by arriving on 5F's (9,15) pad.
            Self::enter(Map::SilphCo9F),                                // via 5F (9,15) → 9F (17,15)
            Self::enter(Map::SilphCo5F),                                // via 9F (17,15) → arrive at 5F (9,15)
            // Free two bag slots before reaching for the Card Key.
            Self::TossItem { item: ItemId::Nugget },
            Self::TossItem { item: ItemId::Tm34Bide },
            Self::CollectItem(MapSprite::SILPHCO5F_CARD_KEY),
        ]
    }

    /// From the Card Key: pads up to Giovanni on 11F, the President's Master Ball, and out. Pads:
    /// 5F(9,15)→9F(17,15); 9F→elevator→3F; 3F(11,11)→7F(5,3); 7F(5,7)→11F(3,2).
    pub fn silph_giovanni_steps() -> Vec<Self> {
        use crate::pokemon::map::MapSprite as MS;
        let mut s = vec![
            Self::enter(Map::SilphCo9F),                                 // 5F(9,15) pad → 9F(17,15)
            Self::enter(Map::SilphCoElevator),
            Self::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 2 }, // 3F = menu index 2
            Self::EnterMap { to_map: Map::SilphCo7F, to_position: Some(Point8 { x: 5, y: 3 }) },   // 3F(11,11) pad
            // Fight the 7F rival head-on: routing past trips his sight and desyncs the 11F warp.
            Self::Interact(MS::SILPHCO7F_RIVAL),
            // ── Out, heal and back in: no Centre, and the rival and Giovanni are one pad apart ──
            Self::EnterMapIfReachable { to_map: Map::SilphCo3F },       // 7F(5,3) pad, back the way we came
            Self::EnterMapIfReachable { to_map: Map::SilphCoElevator },
            Self::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 0 }, // 1F
            Self::EnterMapIfReachable { to_map: Map::SaffronCity },
            Self::EnterMapIfReachable { to_map: Map::SaffronPokecenter },
            Self::InteractIfReachable(MS::SAFFRONPOKECENTER_NURSE),
            Self::EnterMapIfReachable { to_map: Map::SaffronCity },
            Self::EnterMapIfReachable { to_map: Map::SilphCo1F },
            Self::EnterMapIfReachable { to_map: Map::SilphCoElevator },
            Self::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 2 }, // 3F
            Self::EnterMapIfReachable { to_map: Map::SilphCo7F },        // 3F(11,11) pad
            Self::EnterMap { to_map: Map::SilphCo11F, to_position: Some(Point8 { x: 3, y: 2 }) },  // 7F(5,7) pad
            Self::InteractIfReachable(MS::SILPHCO11F_ROCKET1),
        ];
        // Once Giovanni is beaten and gone, `InteractIfReachable` pops where `Interact` would hang.
        for _ in 0..14 { s.push(Self::InteractIfReachable(MS::SILPHCO11F_GIOVANNI)); }
        s.extend([
            // Free a bag slot before the President reaches into his pocket.
            Self::TossItem { item: ItemId::Tm11Bubblebeam },
            Self::Interact(MS::SILPHCO11F_SILPH_PRESIDENT),             // Master Ball + Rockets leave Saffron
            // The way out gives up rather than stalling, because a black-out has already taken it.
            Self::EnterMapIfReachable { to_map: Map::SilphCo7F },   // 11F(3,2) pad
            Self::EnterMapIfReachable { to_map: Map::SilphCo3F },   // 7F(5,3) pad
            Self::EnterMapIfReachable { to_map: Map::SilphCoElevator },
            Self::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 0 }, // 1F
            Self::EnterMapIfReachable { to_map: Map::SaffronCity },
            Self::enter(Map::SaffronPokecenter),
            Self::Interact(MS::SAFFRONPOKECENTER_NURSE),
            Self::enter(Map::SaffronCity),
        ]);
        s
    }

    /// Marsh Badge: `DefeatGymLeader` threads Saffron Gym's pads to Sabrina; needs Saffron freed.
    pub fn marsh_badge_steps() -> Vec<Self> {
        use crate::pokemon::map::MapSprite as MS;
        vec![
            Self::enter(Map::SaffronGym),
            Self::DefeatGymLeader { leader: MS::SAFFRONGYM_SABRINA, badge: Badge::MarshBadge },
        ]
    }

    /// Saffron → Cinnabar: Diglett's Cave, Route 2's two trees, Pallet, then Surf down Route 21.
    pub fn saffron_to_cinnabar_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::SaffronCity),
            Self::enter(Map::Route6),
            Self::enter(Map::Route6Gate),
            Self::EnterMap { to_map: Map::Route6, to_position: Some(Point8 { x: 10, y: 7 }) },
            Self::enter(Map::VermilionCity),
            Self::enter(Map::Route11),
            Self::enter(Map::DiglettsCaveRoute11),
            Self::enter(Map::DiglettsCave),
            Self::enter(Map::DiglettsCaveRoute2),
            Self::enter(Map::Route2),
            Self::CutTree { map: Map::Route2 },
            Self::enter(Map::Route2Gate),
            Self::EnterMap { to_map: Map::Route2, to_position: Some(Point8 { x: 15, y: 39 }) },
            Self::CutTree { map: Map::Route2 },
            Self::enter(Map::ViridianCity),
            Self::enter(Map::Route1),
            Self::enter(Map::PalletTown),
            Self::enter(Map::Route21),
            Self::enter(Map::CinnabarIsland),
        ]
    }

    /// Pokémon Mansion → Secret Key: one global switch toggles every floor's doors, and B1F is
    /// reached by falling through a 3F hole (`apply_mansion_holes`).
    pub fn mansion_secret_key_steps() -> Vec<Self> {
        vec![
            // Heal to full PP first: no Centre inside, and an empty mon Struggles itself out.
            Self::enter(Map::CinnabarPokecenter),
            Self::Interact(MapSprite::CINNABARPOKECENTER_NURSE),
            Self::enter(Map::CinnabarIsland),
            Self::UseRareCandy { slot: 0 },
            Self::enter(Map::PokemonMansion1F),
            Self::enter(Map::PokemonMansion2F),
            Self::EnterMap { to_map: Map::PokemonMansion3F, to_position: Some(Point8 { x: 6, y: 1 }) },
            Self::FlipSwitch { map: Map::PokemonMansion3F, at: Point8 { x: 10, y: 5 }, reveals: Map::PokemonMansion1F },
            Self::enter(Map::PokemonMansion1F),   // fall through a hole → 1F (16,14)
            Self::enter(Map::PokemonMansionB1F),  // (21,23) staircase down
            Self::FlipSwitch { map: Map::PokemonMansionB1F, at: Point8 { x: 18, y: 25 }, reveals: Map::PokemonMansion1F },
            Self::CollectItem(MapSprite::POKEMONMANSIONB1F_TM_BLIZZARD),
            // Taught here: a TM is consumed on use and the bag has no room for both.
            Self::TeachMove { item: ItemId::Tm14Blizzard, target: Self::STARTER_LINE },
            Self::FlipSwitch { map: Map::PokemonMansionB1F, at: Point8 { x: 20, y: 3 }, reveals: Map::PokemonMansion1F },
            Self::CollectItem(MapSprite::POKEMONMANSIONB1F_SECRET_KEY),
        ]
    }

    /// Volcano Badge: undo the two B1F flips to reopen the stairs, heal, and beat Blaine.
    pub fn volcano_badge_steps() -> Vec<Self> {
        vec![
            Self::FlipSwitch { map: Map::PokemonMansionB1F, at: Point8 { x: 20, y: 3 }, reveals: Map::PokemonMansion1F },
            Self::FlipSwitch { map: Map::PokemonMansionB1F, at: Point8 { x: 18, y: 25 }, reveals: Map::PokemonMansion1F },
            Self::enter(Map::PokemonMansion1F),   // (23,22) staircase up → 1F right side
            Self::enter(Map::CinnabarIsland),
            Self::enter(Map::CinnabarPokecenter), // heal before the gym gauntlet
            Self::Interact(MapSprite::CINNABARPOKECENTER_NURSE),
            Self::enter(Map::CinnabarIsland),
            Self::MovePokemonToFront { target: Self::STARTER_LINE },
            Self::enter(Map::CinnabarGym),
            Self::DefeatGymLeader { leader: MapSprite::CINNABARGYM_BLAINE, badge: Badge::VolcanoBadge },
        ]
    }

    /// Articuno in Seafoam B4F: Surf down Route 20, solve both boulder floors, Master Ball it.
    pub fn seafoam_articuno_steps() -> Vec<Self> {
        // Strength is armed per floor: `BIT_STRENGTH_ACTIVE` clears on every map change.
        vec![
            // Heal and stock balls: no Centre inside Seafoam, whose wilds are fled.
            Self::enter(Map::CinnabarIsland),
            Self::enter(Map::CinnabarPokecenter),
            Self::Interact(MapSprite::CINNABARPOKECENTER_NURSE),
            Self::enter(Map::CinnabarIsland),
            // A full bag refuses the purchase in silence, so the unsold Nugget goes first.
            Self::TossItem { item: ItemId::Nugget },
            Self::BuyFromMart { item: BagItem::new(ItemId::GreatBall, 10), map: Map::CinnabarMart },
            // Top the Hyper Potions back up while at the last mart on the route that sells them.
            Self::BuyFromMart { item: BagItem::new(ItemId::HyperPotion, 20), map: Map::CinnabarMart },
            Self::enter(Map::CinnabarIsland),
            // Surf east across Route 20 to the east Seafoam entrance.
            Self::enter(Map::Route20),
            Self::enter_at(Map::SeafoamIslands1F, 26, 17),
            // This leg catches its own Strength carrier.
            Self::CatchPokemon { species: PokemonSpecies::Slowpoke, on_map: Map::SeafoamIslands1F,
                                 ball: Some(ItemId::PokeBall) },
            Self::TeachMove { item: ItemId::Hm04Strength, target: PartyRef::Species(PokemonSpecies::Slowpoke) },
            Self::TeachMove { item: ItemId::Tm28Dig, target: PartyRef::Species(PokemonSpecies::Slowpoke) },
            // Down the east side, one walled pocket at a time, then across to B3F's west half.
            Self::enter_at(Map::SeafoamIslandsB1F, 23, 15),
            Self::enter_at(Map::SeafoamIslandsB2F, 25, 11),
            Self::enter_at(Map::SeafoamIslandsB3F, 25, 14),
            Self::enter_at(Map::SeafoamIslandsB4F, 20, 17),
            Self::enter_at(Map::SeafoamIslandsB3F, 8, 6),
            // ── SEAFOAM4: two of B3F's four boulders into its two holes.
            Self::UseStrength { target: PartyRef::Species(PokemonSpecies::Slowpoke) },
            Self::DropBoulderInHole { hole: Point8 { x: 3, y: 16 },
                                      boulder: Some(Point8 { x: 3, y: 15 }) },
            Self::DropBoulderInHole { hole: Point8 { x: 6, y: 16 },
                                      boulder: Some(Point8 { x: 8, y: 14 }) },
            // Fall through (6,16) into the west lake, already surfing.
            Self::enter_at(Map::SeafoamIslandsB4F, 5, 14),
            Self::CatchPokemon { species: PokemonSpecies::Articuno, on_map: Map::SeafoamIslandsB4F,
                                 ball: Some(ItemId::MasterBall) },
            // The bird arrives with Peck and Ice Beam — 10 PP of Ice for five Elite Four rooms.
            Self::TeachMove { item: ItemId::Tm14Blizzard, target: PartyRef::Species(PokemonSpecies::Articuno) },
            // Dig out: no walk back east, and Cinnabar's Centre above set `wLastBlackoutMap`.
            Self::Dig { target: PartyRef::Species(PokemonSpecies::Slowpoke) },
            Self::enter(Map::CinnabarIsland),
        ]
    }

    /// Earth Badge: Surf back to Pallet, up to Viridian, heal, cross the spinner gym to Giovanni.
    pub fn earth_badge_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::CinnabarIsland),   // out of Blaine's gym
            Self::enter(Map::Route21),
            Self::enter(Map::PalletTown),
            Self::enter(Map::Route1),
            Self::enter(Map::ViridianCity),
            Self::enter(Map::ViridianPokecenter),
            Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
            Self::enter(Map::ViridianCity),
            Self::enter(Map::ViridianGym),
            Self::DefeatGymLeader { leader: MapSprite::VIRIDIANGYM_GIOVANNI, badge: Badge::EarthBadge },
        ]
    }

    /// The level the lone fighter is ground to before the Elite Four.
    pub const GAUNTLET_LEVEL: u8 = 85;

    /// The starter this route plays, named once rather than in six legs.
    const STARTER_BALL: MapSprite = MapSprite::OAKSLAB_SQUIRTLE_POKE_BALL;
    /// The starter, named as its whole line.
    const STARTER_LINE: PartyRef = PartyRef::Line(&[
        PokemonSpecies::Squirtle, PokemonSpecies::Wartortle, PokemonSpecies::Blastoise]);

    /// The Cut carrier.
    const CUT_SLAVE: PartyRef = PartyRef::Species(PokemonSpecies::Oddish);

    const MACHOP: PartyRef = PartyRef::Species(PokemonSpecies::Machop);

    /// Viridian → the Route 22 rival → Route 23 → VR1F, ending with a Machop taught Strength.
    pub fn victory_road_1f_approach_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::ViridianCity),          // out of the gym
            Self::enter(Map::ViridianPokecenter),
            Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
            Self::enter(Map::ViridianCity),
            Self::MovePokemonToFront { target: Self::STARTER_LINE },
            Self::enter(Map::Route22),
            Self::enter(Map::Route22Gate),           // walk west → rival ambush → gate to Route 23
            Self::Interact(MapSprite::ROUTE22GATE_GUARD), // walk to (5,2): badge check + flips the dynamic warp
            Self::enter(Map::Route23),
            Self::goto(Map::VictoryRoad1F),
            // The boulder slave, caught two tiles from the boulder it is for.
            Self::CatchPokemon { species: PokemonSpecies::Machop, on_map: Map::VictoryRoad1F, ball: None },
            Self::TeachMove { item: ItemId::Hm04Strength, target: Self::MACHOP },
            // The catch leaves the Machop leading, and the VR trainers are not its fight.
            Self::MovePokemonToFront { target: Self::STARTER_LINE },
        ]
    }

    /// The boulder onto (17, 13) and the climb to VR2F.
    pub fn victory_road_1f_climb_steps() -> Vec<Self> {
        vec![
            Self::UseStrength { target: Self::MACHOP },
            Self::SolveBoulders { switch: Point8 { x: 17, y: 13 }, boulder: None },
            // `goto`, not `enter`, because a black-out on this last walk is otherwise terminal.
            Self::goto(Map::VictoryRoad2F),
        ]
    }

    /// VR2F and VR3F's hole-drop puzzle, up to the Indigo Plateau lobby.
    pub fn victory_road_2f_3f_steps() -> Vec<Self> {
        vec![
            // VR2F: switch1 (1,16) → up the (23,7) stairs to VR3F.
            Self::UseStrength { target: Self::MACHOP },
            Self::SolveBoulders { switch: Point8 { x: 1, y: 16 }, boulder: None },
            Self::enter(Map::VictoryRoad3F),
            // VR3F: switch (3,5) opens the hole; a boulder into (23,15) reveals 2F's hidden one.
            Self::UseStrength { target: Self::MACHOP },
            Self::SolveBoulders { switch: Point8 { x: 3, y: 5 }, boulder: None },
            Self::DropBoulderInHole { hole: Point8 { x: 23, y: 15 }, boulder: None },
            Self::enter_at(Map::VictoryRoad2F, 22, 16),
            // VR2F east: the revealed boulder onto (9,16), which leaves the player in the west.
            Self::UseStrength { target: Self::MACHOP },
            Self::SolveBoulders { switch: Point8 { x: 9, y: 16 }, boulder: None },
            // Return trip: climb back to VR3F and come down on the exit side.
            Self::enter(Map::VictoryRoad3F),
            // (27,7), not (22,16): only this landing reaches the exit.
            Self::enter_at(Map::VictoryRoad2F, 27, 7),
            // Out the exit beside it → Route 23 → Indigo Plateau → the Elite Four lobby.
            Self::enter(Map::Route23),
            Self::enter(Map::IndigoPlateau),
            Self::enter(Map::IndigoPlateauLobby),
        ]
    }

    /// The grind before the Elite Four: heal, stock up, and level the fighter in the Mansion.
    pub fn gauntlet_grind_steps() -> Vec<Self> {

        vec![
            Self::enter(Map::CinnabarIsland),
            Self::enter(Map::CinnabarPokecenter),
            Self::Interact(MapSprite::CINNABARPOKECENTER_NURSE),
            Self::enter(Map::CinnabarIsland),
            // Healing items, so the grind is not a string of walks back to the Centre.
            Self::BuyFromMart { item: BagItem::new(ItemId::FullHeal, 40), map: Map::CinnabarMart },
            Self::BuyFromMart { item: BagItem::new(ItemId::HyperPotion, 10), map: Map::CinnabarMart },
            Self::enter(Map::CinnabarIsland),
            Self::enter(Map::PokemonMansion1F),
            // One fighter: experience is cubic, so one at lv85 costs less than three at lv75.
            Self::GrindUntilLevel { target_level: Self::GAUNTLET_LEVEL, on_map: Map::PokemonMansion1F,
                target: Self::STARTER_LINE },
            Self::enter(Map::CinnabarIsland),
        ]
    }

    pub fn elite_four_steps() -> Vec<Self> {
        vec![
            // ¥42,000 of Full Restores and Revives, affordable only after the gauntlet grind.
            Self::TossItem { item: ItemId::Tm21MegaDrain },
            Self::BuyFromMart { item: BagItem::new(ItemId::FullRestore, 12), map: Map::IndigoPlateauLobby },
            Self::BuyFromMart { item: BagItem::new(ItemId::Revive, 4), map: Map::IndigoPlateauLobby },
            Self::Interact(MapSprite::INDIGOPLATEAULOBBY_NURSE),   // revive + restore all PP
            // Blastoise leads every room, because it is the only thing in the party that fights.
            Self::MovePokemonToFront { target: Self::STARTER_LINE },
            Self::enter(Map::LoreleisRoom),
            Self::BattleTrainer { trainer: MapSprite::LORELEISROOM_LORELEI },
            Self::enter(Map::BrunosRoom),
            Self::BattleTrainer { trainer: MapSprite::BRUNOSROOM_BRUNO },
            Self::enter(Map::AgathasRoom),
            Self::BattleTrainer { trainer: MapSprite::AGATHASROOM_AGATHA },
            Self::enter(Map::LancesRoom),
            Self::BattleTrainer { trainer: MapSprite::LANCESROOM_LANCE },
            // 26 Pokémon against 35 PP: the Elixer is the margin.
            Self::use_pp_restore(ItemId::Elixer, 0, 0),
            Self::enter(Map::ChampionsRoom),
            Self::BattleTrainer { trainer: MapSprite::CHAMPIONSROOM_RIVAL },
        ]
    }

    /// The whole scripted playthrough, every map transition an explicit `EnterMap`.
    pub fn complete_game_steps() -> Vec<Self> {
        Self::game_steps(true)
    }

    /// The same route stopped at Victory Road 2F: no gauntlet grind, no Elite Four.
    pub fn eight_badge_steps() -> Vec<Self> {
        Self::game_steps(false)
    }

    /// Pallet to Mt Moon 1F: the starter, Brock and the Route 3 grind.
    pub fn pallet_to_cerulean_steps() -> Vec<Self> {
        vec![
            // ── Pallet Town: fetch a starter ──
            Self::enter(Map::RedsHouse1F),
            Self::enter(Map::PalletTown),
            Self::soft_goto(Map::Route1),                        // Oak stops you → OaksLab
            Self::Interact(Self::STARTER_BALL),                   // pick the starter (+ rival battle)

            // ── Viridian Mart: pick up Oak's Parcel ──
            Self::enter(Map::PalletTown),
            Self::enter(Map::Route1),
            Self::enter(Map::ViridianCity),
            Self::enter(Map::ViridianMart),
            Self::Interact(MapSprite::VIRIDIANMART_CLERK),       // clerk hands over Oak's Parcel

            // ── Deliver the Parcel to Oak → Pokédex ──
            Self::enter(Map::ViridianCity),
            Self::enter(Map::Route1),
            Self::enter(Map::PalletTown),
            Self::enter(Map::OaksLab),
            Self::Interact(MapSprite::OAKSLAB_OAK1),

            // ── Town Map from Daisy ──
            Self::enter(Map::PalletTown),
            Self::enter(Map::BluesHouse),
            Self::Interact(MapSprite::BLUESHOUSE_DAISY1),

            // ── Stock up + heal in Viridian City ──
            Self::enter(Map::PalletTown),
            Self::enter(Map::Route1),
            Self::enter(Map::ViridianCity),
            Self::enter(Map::ViridianPokecenter),
            Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
            Self::enter(Map::ViridianCity),

            // ── Grind the starter on Route 1 ──
            Self::enter(Map::Route1),
            // Twelve: the level is bounded by what there is to fight.
            Self::GrindUntilLevel { target_level: 12, on_map: Map::Route1, target: PartyRef::Slot(0) },
            Self::enter(Map::ViridianCity),
            Self::enter(Map::ViridianPokecenter),
            Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
            Self::enter(Map::ViridianCity),

            // ── Viridian Forest → Pewter City ──
            Self::enter(Map::Route2),
            Self::enter(Map::ViridianForestSouthGate),
            Self::enter(Map::ViridianForest),
            Self::enter(Map::ViridianForestNorthGate),
            Self::enter(Map::Route2),
            Self::enter(Map::PewterCity),
            Self::enter(Map::PewterPokecenter),
            Self::Interact(MapSprite::PEWTERPOKECENTER_NURSE),
            Self::enter(Map::PewterCity),

            // ── Defeat Brock (Boulder Badge) ──
            Self::DefeatGymLeader { leader: MapSprite::PEWTERGYM_BROCK, badge: Badge::BoulderBadge },
            // Every forward `enter` must be one direct transition, so out of the gym first.
            Self::enter(Map::PewterCity),
            Self::enter(Map::PewterPokecenter),
            Self::Interact(MapSprite::PEWTERPOKECENTER_NURSE),
            Self::enter(Map::PewterCity),
            // The first medicine the route can buy.
            Self::enter(Map::PewterMart),
            Self::BuyFromMart { item: BagItem::new(ItemId::Potion, 10), map: Map::PewterMart },
            Self::enter(Map::PewterCity),

            // ── Route 3 grind → heal at the Mt Moon Pokécenter ──
            Self::enter(Map::Route3),
            // Twenty-two, because the next fight after Mt Moon is the rival.
            Self::GrindUntilLevel { target_level: 22, on_map: Map::Route3, target: PartyRef::Slot(0) },
            Self::enter(Map::Route4),
            Self::enter(Map::MtMoonPokecenter),
            Self::Interact(MapSprite::MTMOONPOKECENTER_NURSE),
            Self::enter(Map::Route4),
            Self::enter(Map::MtMoon1F),
        ]
    }

    /// `finish` adds the gauntlet grind and everything past the eighth badge.
    fn game_steps(finish: bool) -> Vec<Self> {
        let mut steps = Self::pallet_to_cerulean_steps();
        steps.extend(Self::mt_moon_traversal());

        steps.extend([
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
        ]);

        steps.extend(Self::cerulean_to_vermilion_steps());
        steps.extend(Self::ss_anne_steps());
        steps.extend(Self::thunder_badge_steps());
        steps.extend(Self::back_to_cerulean_steps());
        steps.extend(Self::cerulean_to_lavender_steps());
        steps.extend(Self::lavender_to_celadon_steps());
        steps.extend(Self::celadon_rainbow_steps());
        steps.extend(Self::rocket_hideout_entrance_steps());
        steps.extend(Self::silph_scope_steps());
        steps.extend(Self::poke_flute_steps());
        steps.extend(Self::snorlax_steps());
        steps.extend(Self::soul_badge_steps());
        steps.extend(Self::safari_zone_surf_steps());
        steps.extend(Self::safari_zone_strength_steps());
        steps.extend(Self::saffron_entry_steps());
        steps.extend(Self::silph_co_card_key_steps());
        steps.extend(Self::silph_giovanni_steps());
        steps.extend(Self::marsh_badge_steps());
        steps.extend(Self::saffron_to_cinnabar_steps());
        steps.extend(Self::mansion_secret_key_steps());
        steps.extend(Self::volcano_badge_steps());
        if finish {
            steps.extend(Self::gauntlet_grind_steps());
        }
        steps.extend(Self::earth_badge_steps());
        steps.extend(Self::victory_road_1f_approach_steps());
        steps.extend(Self::victory_road_1f_climb_steps());
        if finish {
            steps.extend(Self::victory_road_2f_3f_steps());
            steps.extend(Self::elite_four_steps());
        }

        steps
    }
}

/// The scripted route's own cursor, kept beside the save it belongs to.
mod scripted_progress {
    use std::path::Path;

    pub const FILE: &str = "scripted-progress.json";

    /// FNV-1a over each step's `Debug`, which is what makes the cursor safe to trust.
    pub fn fingerprint(steps: &[super::PolicyStep]) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for step in steps {
            for byte in format!("{step:?}").bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x1000_0000_01b3);
            }
        }
        hash
    }

    /// `(completed, total, route)`, or `None` for a run that never wrote one: a new run.
    pub fn load(dir: &Path) -> Option<(usize, usize, u64)> {
        let text = std::fs::read_to_string(dir.join(FILE)).ok()?;
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        let field = |name: &str| value.get(name)?.as_u64();
        Some((field("completed")? as usize, field("total")? as usize, field("route")?))
    }

    pub fn save(dir: &Path, completed: usize, total: usize, route: u64) {
        let body = serde_json::json!({ "completed": completed, "total": total, "route": route });
        if let Err(failure) = crate::run::write_atomically(
            &dir.join(FILE), body.to_string().as_bytes()) {
            // Loud but not fatal, like a failed checkpoint.
            println!("[policy] could not record scripted progress: {failure}");
        }
    }

    pub fn clear(dir: &Path) {
        let _ = std::fs::remove_file(dir.join(FILE));
    }
}

/// Where a scripted run records its place, and what it last recorded there.
struct ScriptedCursor {
    dir: std::path::PathBuf,
    /// The route entire, so `restart` can put it back.
    full_route: Vec<PolicyStep>,
    /// The route length the cursor counts against; with `route`, what makes it safe to trust.
    total: usize,
    route: u64,
    written: usize,
}

pub struct DeterministicPolicy {
    rng: StdRng,
    /// What `rng` and `name_picker` were seeded from, so [`Policy::restart`] rebuilds exactly.
    seed: u64,
    queue: VecDeque<PolicyStep>,
    /// Where to record how far along the route this run is, and what it last recorded.
    progress: Option<ScriptedCursor>,
    name_picker: PokemonNamePicker,
    /// The last Pokémon Center where the player was healed.
    pub last_pokemon_center: Option<Map>,
    /// The Centre a heal detour is walking to.
    heal_return: Option<Map>,
    /// Shop visits this `BuyFromMart` step has made without the purchase landing.
    mart_attempts: u32,
    /// `(money, quantity held)` as the last `BuyFromMart` shop visit was opened.
    mart_baseline: Option<(u32, u8)>,
    /// Consecutive polls a heal detour has been unable to move.
    heal_route_stuck: u32,
    /// Set when a heal detour gives up on routing; cleared by the next heal.
    heal_unreachable: bool,
    /// Where a heal detour set off from, so it can put the run back.
    heal_came_from: Option<Map>,
    /// Consecutive polls spent waiting for a nurse to finish.
    heal_waits: u32,
    /// Consecutive polls a `DefeatGymLeader` step has found no route to its gym.
    gym_route_stuck: u32,
    /// The map a `Dig` was issued from, so the step pops once Dig has actually warped away.
    dig_from_map: Option<Map>,
    /// Whether the current `CollectItem` or `UseFieldItem` sprite has been seen present.
    collect_item_seen: bool,
    /// Polls a `CollectItem` has waited, so a refused pickup gives up with a reason.
    collect_item_waits: u32,
    /// Consecutive polls a catch has found no encounter source.
    catch_wander_stuck: u32,
    /// Species a `CatchPokemon` step gave up on, so steps waiting for them stop.
    catch_abandoned: Vec<PokemonSpecies>,
    /// The ball's quantity when this catch battle began, telling a missed throw from none yet.
    catch_ball_baseline: Option<u8>,
    /// A slot switched in at the start of every battle so it earns the XP.
    train_slot: Option<u8>,
    /// Set once the grind trainee has taken part in the current battle.
    trainee_participated: bool,
    /// Polls an `InteractIfReachable` has waited for its sprite.
    interact_skip_waits: u32,
    /// `mansion_switch_on` when the current Mansion `FlipSwitch` began.
    mansion_flip_baseline: Option<bool>,
    /// Visible-boulder count captured when the current `DropBoulderInHole` step began.
    boulder_drop_baseline: Option<usize>,
    /// Which target the evolve baseline belongs to, so a step at another mon starts afresh.
    evolve_baseline: Option<(PartyRef, PokemonSpecies)>,
    /// Trainers beaten this `DefeatGymLeader` step: a beaten trainer keeps its sprite, so
    /// standing in its sight with no battle marks it here.
    gym_beaten: HashSet<Point8>,
    /// Casts the current `Fish` step has issued.
    fish_casts: u32,
    /// Safari trips paid, and whether the last tick was inside: an ejection is an edge and
    /// `EVENT_IN_SAFARI_ZONE` only a level.
    safari: crate::pokemon::postgame::safari::HuntProgress,
    /// `(trainer, player position, stuck ticks)` for a trainer approach.
    gym_engage: Option<(Point8, Point8, u32)>,
    /// Polls an `EnterMapIfReachable` has waited.
    enter_stuck: u32,
    /// The bag count of a `UseBagItem`'s item at the start, so exactly one is spent.
    item_use_baseline: Option<u8>,
    /// How many times the current `UseBagItem` step has been handed to the driver.
    item_use_attempts: u32,
    /// Bag quantities of a `UseItemsInBattle` step's items when it began, parallel to its list.
    battle_item_baseline: Option<Vec<u8>>,
    /// Black-outs this run has had, and whether the newest one is still waiting to be reported.
    blackouts: u32,
    blackout_pending: bool,
    /// Actions the current heal-return detour has issued.
    heal_hops: u32,
    /// Round trips a `GrindUntilLevel` has made to a Pokémon Centre because its trainee fainted.
    grind_heal_trips: u32,
    /// The map of the last battle, which a black-out is reported against.
    last_battle_map: Option<Map>,
}

/// Whether a map sprite is a static encounter of `species`.
fn sprite_is_species(name: &str, species: PokemonSpecies) -> bool {
    name.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ') == species.to_string()
}

impl DeterministicPolicy {
    /// Whether the step at the queue front wants `enemy` caught, and with which ball.
    fn catch_target(&self, state: &GameState, enemy: PokemonSpecies) -> Option<Option<ItemId>> {
        match self.queue.front() {
            Some(&PolicyStep::CatchPokemon { species, ball, .. }) if species == enemy => Some(ball),
            Some(&PolicyStep::SweepDex { ball, .. })
                if crate::pokemon::postgame::aides::sweep_wants(state, enemy) => Some(ball),
            _ => None,
        }
    }

    /// Give up on `species` and record it, so steps waiting for it stop.
    fn abandon_catch(&mut self, species: PokemonSpecies, why: &str) {
        println!("[policy] giving up on catching {species}: {why}");
        if !self.catch_abandoned.contains(&species) {
            self.catch_abandoned.push(species);
        }
    }

    /// Whether `target` names a species a catch gave up on.
    fn target_was_abandoned(&self, target: PartyRef) -> bool {
        match target {
            PartyRef::Species(species) => self.catch_abandoned.contains(&species),
            PartyRef::Line(line) => line.iter().any(|s| self.catch_abandoned.contains(s)),
            // A slot is a position, not a promise about a species.
            PartyRef::Slot(_) => false,
        }
    }

    /// Polls a `DefeatGymLeader` waits for a route to its gym, e.g. just after a black-out.
    const MAX_GYM_ROUTE_WAIT: u32 = 400;
    /// Polls a heal detour waits for a route before handing back to the queue.
    const MAX_HEAL_ROUTE_WAIT: u32 = 400;
    /// Hops the heal-return detour may take before it concludes it is going round in circles.
    const MAX_HEAL_HOPS: u32 = 60;
    /// Shop visits one `BuyFromMart` makes before giving up.
    const MAX_MART_ATTEMPTS: u32 = 4;
    /// Times one `UseBagItem` is handed to the driver before giving up.
    const MAX_ITEM_USE_ATTEMPTS: u32 = 4;

    /// How many polls a `CollectItem` may spend on an item that will not be picked up.
    const MAX_COLLECT_ITEM_WAITS: u32 = 3000;
    /// Polls a nurse gets to finish healing before the route carries on without her.
    const MAX_HEAL_WAITS: u32 = 1500;
    /// Policy polls, not ticks, that one `EnterMapIfReachable` spends before giving up.
    const MAX_ENTER_WAIT: u32 = 60;

    /// Resume this route where the last process left it, and keep recording as it advances.
    pub fn resuming_in(mut self, run_dir: &std::path::Path, from_the_beginning: bool) -> Self {
        let total = self.queue.len();
        let route = scripted_progress::fingerprint(self.queue.make_contiguous());
        let full_route: Vec<PolicyStep> = self.queue.iter().cloned().collect();
        match scripted_progress::load(run_dir) {
            None if from_the_beginning =>
                println!("[policy] no scripted progress on disk — starting the route from the beginning"),
            None => {
                println!(
                    "[policy] this run is being resumed but recorded no scripted progress, so \
                     there is no telling how much of the {total}-step route its save has already \
                     played. Parking rather than replaying the route over a game that may be \
                     part-way through it. Start a new run to play this one.",
                );
                self.queue.clear();
            }
            Some((completed, saved_total, saved_route)) if saved_total == total && saved_route == route => {
                println!("[policy] resuming the scripted route at step {completed}/{total}");
                self.queue.drain(..completed.min(total));
            }
            Some((completed, saved_total, saved_route)) => {
                println!(
                    "[policy] the scripted route has changed under this run ({saved_total} steps \
                     / {saved_route:016x} recorded, {total} / {route:016x} now), so step \
                     {completed} means nothing here. Parking rather than replaying a different \
                     route over a game that is already part-way through it. Start a new run to play \
                     this one.",
                );
                self.queue.clear();
            }
        }
        self.progress = Some(ScriptedCursor {
            dir: run_dir.to_path_buf(),
            full_route,
            total,
            route,
            written: usize::MAX,
        });
        self.record_progress();
        self
    }

    /// Write the cursor when it has moved.
    fn record_progress(&mut self) {
        let remaining = self.queue.len();
        let Some(cursor) = self.progress.as_mut() else { return };
        let completed = cursor.total.saturating_sub(remaining);
        if cursor.written == completed { return }
        cursor.written = completed;
        scripted_progress::save(&cursor.dir, completed, cursor.total, cursor.route);
    }

    pub fn new(seed: u64, steps: impl IntoIterator<Item = PolicyStep>) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
            seed,
            queue: steps.into_iter().collect(),
            progress: None,
            name_picker: PokemonNamePicker::seed_from_u64(seed),
            last_pokemon_center: None,
            heal_return: None,
            heal_route_stuck: 0,
            heal_unreachable: false,
            heal_came_from: None,
            heal_waits: 0,
            mart_attempts: 0,
            mart_baseline: None,
            gym_route_stuck: 0,
            dig_from_map: None,
            collect_item_seen: false,
            collect_item_waits: 0,
            catch_wander_stuck: 0,
            catch_abandoned: Vec::new(),
            catch_ball_baseline: None,
            mansion_flip_baseline: None,
            boulder_drop_baseline: None,
            evolve_baseline: None,
            gym_beaten: HashSet::new(),
            gym_engage: None,
            train_slot: None,
            trainee_participated: false,
            interact_skip_waits: 0,
            fish_casts: 0,
            safari: Default::default(),
            enter_stuck: 0,
            item_use_baseline: None,
            item_use_attempts: 0,
            battle_item_baseline: None,
            blackouts: 0,
            blackout_pending: false,
            heal_hops: 0,
            grind_heal_trips: 0,
            last_battle_map: None,
        }
    }

    /// Route one hop toward `target` over the incremental world graph.
    pub(crate) fn route_toward(world_graph: &WorldGraph, map: &MetaTileMap, actions: &[OverworldAction],
                               target: Map) -> Option<OverworldAction> {
        // A transition on this map is the shortest path, and the graph could miss it.
        Self::enter_map_action(actions, target, None)
            .or_else(|| world_graph.pick_shortest_path_action(&Self::with_every_crossing(map, actions), target))
    }

    /// `actions`, then every other reachable crossing into the same neighbours: out of a pocket the
    /// nearest crossing is the pocket's own. Offered rows stay first so a tie keeps them.
    fn with_every_crossing(map: &MetaTileMap, actions: &[OverworldAction]) -> Vec<OverworldAction> {
        let mut candidates = actions.to_vec();
        let mut neighbours: Vec<Map> = vec![];
        for action in actions {
            if let MetaTile::Connection { to_map, .. } = action.tile
                && !neighbours.contains(&to_map)
            {
                neighbours.push(to_map);
            }
        }
        for to_map in neighbours {
            for crossing in map.crossings(to_map).into_iter().filter(|crossing| crossing.reachable) {
                let tile = MetaTile::Connection { to_map, to_position: crossing.to_position };
                if !candidates.iter().any(|action| action.tile == tile)
                    && let Some(action) = map.connection_action(to_map, crossing.to_position)
                {
                    candidates.push(action);
                }
            }
        }
        candidates
    }

    /// The row that puts a boulder on switch or hole `target`, or `None` while there is none.
    fn boulder_goal_action(state: &GameState, actions: &[OverworldAction], target: Point8,
                           boulder: Option<Point8>, hole: bool) -> Option<OverworldAction> {
        match boulder {
            Some(which) => state.map.boulder_goal_action(which, target, hole),
            None => actions.iter()
                .find(|action| matches!(action.tile, MetaTile::BoulderGoal { at, .. } if at == target))
                .cloned(),
        }
    }

    /// The nearest action taking a transition to `to_map` (at raw `to_position` if given).
    fn enter_map_action(actions: &[OverworldAction], to_map: Map, to_position: Option<Point8>) -> Option<OverworldAction> {
        actions.iter().filter(|a| match a.tile {
            MetaTile::Warp { to_map: m, to_position: p }
            | MetaTile::Connection { to_map: m, to_position: p } => {
                m == to_map && to_position.map_or(true, |want| want == p)
            }
            // A water edge carries no landing, so it matches on the map alone.
            MetaTile::ConnectionWater(m) => m == to_map,
            _ => false,
        }).min_by_key(|a| a.route.len()).cloned()
    }

}

impl Policy for DeterministicPolicy {
    fn name(&self) -> &'static str { "scripted" }

    /// Count black-outs.
    fn on_event(&mut self, event: &crate::pokemon::agent::AgentEvent) {
        if let crate::pokemon::agent::AgentEvent::TextBox { message } = event
            && message.contains("blacked out")
        {
            self.blackouts += 1;
            self.blackout_pending = true;
        }
    }

    fn pick_overworld_action(&mut self, state: &GameState, world_graph: &WorldGraph) -> Option<OverworldAction> {
        // The cursor is written here rather than at every `pop_front`.
        self.record_progress();
        // Back in the overworld, so the last battle is over.
        self.trainee_participated = false;
        // The next catch battle is a fresh target, so the last one's spend says nothing.
        self.catch_ball_baseline = None;
        if self.blackout_pending {
            self.blackout_pending = false;
            println!("[policy] BLACKOUT #{} — lost on {}; queue at {} with {:?}; party {:?}",
                self.blackouts,
                self.last_battle_map.map_or_else(|| "an unrecorded map".to_string(), |m| m.to_string()),
                self.queue.len(),
                self.queue.front(),
                state.pokemon.iter().map(|p| (p.species, p.level)).collect::<Vec<_>>());
        }
        if state.map.map.is_pokemon_center() {
            self.last_pokemon_center = Some(state.map.map);
            // Standing in a Centre is the one place the give-up above is certainly stale.
            self.heal_unreachable = false;
        }

        let actions = state.map.actions();

        // ── Heal before the next fight, not after it (`needs_a_centre`) ──
        if self.heal_return.is_none()
            && !self.heal_unreachable
            && (state.map.map.is_overworld() || self.queue.front().is_some_and(step_finds_its_own_way_back))
            && let Some(centre) = self.last_pokemon_center
            && needs_a_centre(state,
                matches!(self.queue.front(), Some(PolicyStep::GrindUntilLevel { .. })))
        {
            println!("[policy] the lead cannot fight another battle — detouring to {centre} first");
            self.heal_return = Some(centre);
            self.heal_came_from = Some(state.map.map);
            self.heal_route_stuck = 0;
            self.heal_hops = 0;
        }

        // ── Heal detour ──
        if let Some(pokecenter) = self.heal_return {
            if state.map.map == pokecenter {
                // Arrived — find and interact with the Nurse.
                if let Some(action) = actions.iter().find(|a| a.tile == MetaTile::Sprite("Nurse")) {
                    self.heal_return = None;
                    self.heal_route_stuck = 0;
                    return Some(action.clone());
                }
                // No Nurse yet: the arrival settling, bounded.
                self.heal_route_stuck += 1;
                if self.heal_route_stuck < Self::MAX_HEAL_ROUTE_WAIT {
                    return None;
                }
                println!("[policy] no Nurse in sight on {pokecenter} — carrying on without the heal");
                self.heal_return = None;
                self.heal_route_stuck = 0;
            } else if self.heal_hops >= Self::MAX_HEAL_HOPS {
                // Routing and arriving are different things, and only one of them was bounded.
                println!("[policy] the heal detour to {pokecenter} has taken {} hops without \
                          arriving — carrying on with the route", self.heal_hops);
                self.heal_return = None;
                self.heal_route_stuck = 0;
                self.heal_hops = 0;
                // Latched, or the low-PP check re-arms the same circle on the next encounter.
                self.heal_unreachable = true;
            } else if let Some(action) = Self::route_toward(world_graph, &state.map, &actions, pokecenter)
                // Failing that, the Centre's town: towns join by walkable connections.
                .or_else(|| pokecenter.pokemon_center_town()
                    .filter(|&town| town != state.map.map)
                    .and_then(|town| Self::route_toward(world_graph, &state.map, &actions, town)))
            {
                self.heal_route_stuck = 0;
                self.heal_hops += 1;
                return Some(action);
            } else {
                // Right after a black-out the map is briefly unsettled, so wait before giving up.
                self.heal_route_stuck += 1;
                if self.heal_route_stuck < Self::MAX_HEAL_ROUTE_WAIT {
                    return None;
                }
                println!("[policy] no route from {} to {} to heal — carrying on with the route",
                    state.map.map, pokecenter);
                self.heal_return = None;
                self.heal_route_stuck = 0;
                // Latch it, or the give-up is undone by the very next battle.
                self.heal_unreachable = true;
            }
        }

        // ── Walk back: a detour that does not return strands the step it interrupted ──
        if let Some(from) = self.heal_came_from {
            let front_reroutes = self.queue.front().is_some_and(step_finds_its_own_way_back);
            if self.heal_return.is_some() || from == state.map.map || front_reroutes {
                if self.heal_return.is_none() { self.heal_came_from = None; }
            } else if let Some(action) = Self::route_toward(world_graph, &state.map, &actions, from) {
                self.heal_route_stuck = 0;
                return Some(action);
            } else {
                self.heal_route_stuck += 1;
                if self.heal_route_stuck < Self::MAX_HEAL_ROUTE_WAIT { return None; }
                println!("[policy] healed, but no route back to {from} — carrying on with the route");
                self.heal_came_from = None;
                self.heal_route_stuck = 0;
            }
        }

        println!("[policy] map={} pos={} front={:?} queue_len={}",
            state.map.map, state.map.player_position, self.queue.front(), self.queue.len());
        loop {
            let step = self.queue.front()?.clone();
            return match step {
                PolicyStep::EnterMap { to_map, to_position } => {
                    if state.map.map == to_map {
                        self.queue.pop_front();
                        continue;
                    }
                    if let Some(action) = Self::enter_map_action(&actions, to_map, to_position) {
                        return Some(action);
                    }
                    // A landing `actions()` does not offer, as it offers only the nearest.
                    if let Some(pos) = to_position {
                        if let Some(action) = state.map.connection_action(to_map, pos) {
                            return Some(action);
                        }
                        // No *land* crossing lands there.
                        if let Some(action) = state.map.water_connection_action(to_map) {
                            return Some(action);
                        }
                    }
                    // Recovery: the direct transition isn't on the current map.
                    Self::route_toward(world_graph, &state.map, &actions, to_map)
                },
                PolicyStep::EnterMapIfReachable { to_map } => {
                    if state.map.map == to_map {
                        self.enter_stuck = 0;
                        self.queue.pop_front();
                        continue;
                    }
                    // Counted on every poll: the failure this survives is not always nowhere to go.
                    self.enter_stuck += 1;
                    if self.enter_stuck >= Self::MAX_ENTER_WAIT {
                        println!("[policy] TOUR: gave up entering {to_map} from {} after {} ticks",
                            state.map.map, self.enter_stuck);
                        self.enter_stuck = 0;
                        self.queue.pop_front();
                        continue;
                    }
                    Self::enter_map_action(&actions, to_map, None)
                        .or_else(|| Self::route_toward(world_graph, &state.map, &actions, to_map))
                },
                PolicyStep::Goto { map: target, strict } => {
                    if state.map.map == target {
                        self.queue.pop_front();
                        continue;
                    }
                    let action = Self::route_toward(world_graph, &state.map, &actions, target);
                    if !strict && action.is_some() {
                        // A non-strict goto action can be interrupted
                        self.queue.pop_front();
                    }
                    action
                },
                PolicyStep::CatchPokemon { species, on_map, .. } => {
                    if state.map.map != on_map {
                        let action = Self::route_toward(world_graph, &state.map, &actions, on_map);
                        if action.is_none() {
                            self.abandon_catch(species, &format!("no path to {on_map}"));
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if state.pokedex_owned.contains(&species) {
                        // Caught the pokemon (note this only works once for each species)
                        self.queue.pop_front();
                        continue;
                    } else if state.bag.best_pokeball().is_none() {
                        self.abandon_catch(species, "no Pokéballs left in the bag");
                        self.queue.pop_front();
                        continue;
                    } else if on_map.sprites().iter().any(|s| sprite_is_species(s.name, species)) {
                        // A static encounter (a legendary) is a map sprite named after its species.
                        match actions.iter().find(|a| matches!(a.tile, MetaTile::Sprite(n) if sprite_is_species(n, species))) {
                            Some(action) => {
                                println!("[policy] static encounter: routing to {species} at {} ({} steps)",
                                    action.destination, action.route.len());
                                self.catch_wander_stuck = 0;
                                Some(action.clone())
                            }
                            None => {
                                self.catch_wander_stuck += 1;
                                let spent = state.map.sprites.iter()
                                    .any(|s| s.hidden && sprite_is_species(s.name, species));
                                if self.catch_wander_stuck < if spent { 50 } else { 400 } {
                                    None
                                } else {
                                    self.abandon_catch(species, &format!("its sprite on {on_map} is unreachable"));
                                    self.catch_wander_stuck = 0;
                                    self.queue.pop_front();
                                    continue;
                                }
                            }
                        }
                    } else if let Some(action) = actions.iter().find(|a| a.tile == MetaTile::Grass) {
                        self.catch_wander_stuck = 0;
                        Some(action.clone()) // walk in grass to trigger encounters
                    } else if let Some(action) = actions.iter()
                        .filter(|a| matches!(a.tile, MetaTile::Sprite(_)))
                        .max_by_key(|a| a.route.len()) {
                        // No grass (a cave): walk to the farthest reachable object.
                        self.catch_wander_stuck = 0;
                        Some(action.clone())
                    } else if let Some(action) = state.map.wander_action() {
                        // Pace to the farthest walkable tile; Surfing rolls too.
                        self.catch_wander_stuck = 0;
                        Some(action)
                    } else {
                        self.catch_wander_stuck += 1;
                        if self.catch_wander_stuck < 400 {
                            None // wait
                        } else {
                            self.abandon_catch(species, "nowhere on this map to trigger an encounter");
                            self.catch_wander_stuck = 0;
                            self.queue.pop_front();
                            continue;
                        }
                    }
                },
                PolicyStep::SweepDex { on_map, min_share, .. } => {
                    use crate::pokemon::postgame::aides;
                    if state.map.map != on_map {
                        let action = Self::route_toward(world_graph, &state.map, &actions, on_map);
                        if action.is_none() {
                            println!("[policy] want to sweep {on_map}, but no path there!");
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if aides::sweep_remaining(&state, on_map, min_share).is_empty() {
                        println!("[policy] SweepDex {on_map}: every target owned — done ({} in the dex)",
                            state.pokedex_owned.species().len());
                        self.catch_wander_stuck = 0;
                        self.queue.pop_front();
                        continue;
                    } else if state.bag.best_pokeball().is_none() {
                        println!("[policy] SweepDex {on_map}: out of Pokéballs, {:?} still missing!",
                            aides::sweep_remaining(&state, on_map, min_share));
                        self.catch_wander_stuck = 0;
                        self.queue.pop_front();
                        continue;
                    } else if let Some(action) = actions.iter().find(|a| a.tile == MetaTile::Grass) {
                        self.catch_wander_stuck = 0;
                        Some(action.clone())
                    } else if let Some(action) = actions.iter()
                        .filter(|a| matches!(a.tile, MetaTile::Sprite(_)))
                        .max_by_key(|a| a.route.len()) {
                        // A cave: pace between the farthest objects.
                        self.catch_wander_stuck = 0;
                        Some(action.clone())
                    } else if let Some(action) = state.map.wander_action() {
                        self.catch_wander_stuck = 0;
                        Some(action)
                    } else {
                        self.catch_wander_stuck += 1;
                        if self.catch_wander_stuck < 400 {
                            None
                        } else {
                            println!("[policy] SweepDex {on_map}: nowhere to trigger an encounter (gave up)!");
                            self.catch_wander_stuck = 0;
                            self.queue.pop_front();
                            continue;
                        }
                    }
                },
                PolicyStep::GrindUntilLevel { target_level, on_map, target } => {
                    let Some(slot) = target.resolve(state) else {
                        println!("[policy] nothing matching {target:?} to level up");
                        self.queue.pop_front();
                        continue;
                    };
                    if let Some(pokemon) = state.pokemon.get(slot as usize) {
                        if pokemon.level >= target_level {
                            self.queue.pop_front();
                            continue;
                        }
                        if pokemon.current_hp == 0 {
                            // Honour the detour's give-up here, or it is not one.
                            if self.heal_unreachable {
                                println!("[policy] grind mon (slot {slot}) is fainted and no Pokémon \
                                    Centre can be routed to from {} — giving up on the grind",
                                    state.map.map);
                                self.heal_unreachable = false;
                                self.queue.pop_front();
                                continue;
                            }
                            if let Some(center) = self.last_pokemon_center {
                                // Counted: the round trip is the grind's other cost.
                                self.grind_heal_trips += 1;
                                println!("[policy] grind mon (slot {slot}) fainted — routing to {center} to heal (trip #{})",
                                    self.grind_heal_trips);
                                self.heal_return = Some(center);
                                return Self::route_toward(world_graph, &state.map, &actions, center);
                            }
                        }
                    } else {
                        println!("[policy] no Pokemon in slot {slot} to level up");
                        self.queue.pop_front();
                        continue;
                    }
                    if state.map.map != on_map {
                        let action = Self::route_toward(world_graph, &state.map, &actions, on_map);
                        if action.is_none() {
                            println!("[policy] want to grind until level {} in {}, but no path there!", target_level, on_map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if let Some(action) = actions.iter()
                        .filter(|a| a.tile == MetaTile::Grass)
                        .min_by_key(|a| a.route.len())
                    {
                        // The nearest grass, so the grind ping-pongs locally.
                        Some(action.clone())
                    } else if !state.map.has_grass_tiles() && let Some(action) = actions.iter()
                        .filter(|a| match a.tile {
                            // Every cave step rolls: pace between non-ball, non-boulder objects.
                            MetaTile::Sprite(name) => !state.map.sprites.iter().any(|s| s.name == name
                                && matches!(s.picture_id, crate::pokemon::sprite::PictureId::PokeBall
                                                        | crate::pokemon::sprite::PictureId::Boulder)),
                            _ => false,
                        })
                        .max_by_key(|a| a.route.len())
                        .cloned()
                        // When the floor offers nothing to pace between, wander — never a warp.
                        .or_else(|| state.map.wander_action())
                    {
                        Some(action)
                    } else {
                        // Pacing is only for a grassless map; elsewhere it loops.
                        println!(
                            "[policy] cannot level up a Pokemon on {}: {}",
                            state.map.map,
                            match state.map.has_grass_tiles() {
                                true => "its grass is not reachable from here",
                                false => "no grass and nothing to pace between",
                            },
                        );
                        self.queue.pop_front();
                        continue;
                    }
                },
                PolicyStep::DefeatGymLeader { leader, badge } => {
                    if state.badges.contains(badge) {
                        self.gym_beaten.clear();
                        self.gym_engage = None;
                        self.gym_route_stuck = 0;
                        self.queue.pop_front();
                        continue;
                    } else if state.map.map != leader.map() {
                        // A loss blacks out to a Centre, so this never pops on a defeat.
                        match Self::route_toward(world_graph, &state.map, &actions, leader.map()) {
                            Some(action) => { self.gym_route_stuck = 0; Some(action) }
                            None if actions.iter().any(|a| matches!(a.tile, MetaTile::Cut { .. })) => {
                                println!("[policy] no route to {} — cutting the regrown trees on {}",
                                    leader.map(), state.map.map);
                                self.gym_route_stuck = 0;
                                self.queue.push_front(PolicyStep::CutTree { map: state.map.map });
                                continue;
                            }
                            None => {
                                // Just after a black-out the map is unsettled: wait, bounded.
                                self.gym_route_stuck += 1;
                                if self.gym_route_stuck < Self::MAX_GYM_ROUTE_WAIT {
                                    None
                                } else {
                                    println!("[policy] want to defeat {} to obtain the {}, but no path there!", leader, badge);
                                    self.gym_route_stuck = 0;
                                    self.queue.pop_front();
                                    continue;
                                }
                            }
                        }
                    } else if let Some(a) = actions.iter().find(|a| a.tile == MetaTile::Sprite(leader.name)) {
                        self.gym_route_stuck = 0;
                        // Stay on this step until the badge is obtained — do not pop here.
                        Some(a.clone())
                    } else if actions.iter().any(|a| matches!(a.tile, MetaTile::Cut { .. })) {
                        // The leader is walled off behind cuttable trees.
                        println!("[policy] {} is walled off — cutting the regrown trees in {}",
                            leader, state.map.map);
                        self.queue.push_front(PolicyStep::CutTree { map: state.map.map });
                        continue;
                    } else {
                        // In a gated gym (Cinnabar) each junior trainer beaten opens a gate.
                        use crate::pokemon::map_metadata::PlayerFacingDirection;
                        let mut cands: Vec<_> = state.map.sprites.iter()
                            .filter(|s| !s.hidden && s.name != leader.name && !s.name.contains("Guide")
                                && !self.gym_beaten.contains(&s.position))
                            .filter_map(|s| state.map.route_to_face_dir(s.position, Some(PlayerFacingDirection::Up))
                                .map(|r| (s.position, s.name, r)))
                            .collect();
                        cands.sort_by_key(|(_, _, r)| r.len());
                        let cur = state.map.player_position;
                        let mut chosen = None;
                        for (pos, name, route) in cands {
                            if route.is_empty() {
                                // In its sight with no battle: already beaten.
                                self.gym_beaten.insert(pos);
                                continue;
                            }
                            // Re-targeted from one spot 40 times: after-battle text, so beaten.
                            match self.gym_engage {
                                Some((t, p, w)) if t == pos && p == cur => {
                                    if w + 1 > 40 {
                                        self.gym_beaten.insert(pos);
                                        self.gym_engage = None;
                                        continue;
                                    }
                                    self.gym_engage = Some((pos, cur, w + 1));
                                }
                                _ => self.gym_engage = Some((pos, cur, 0)),
                            }
                            chosen = Some(OverworldAction { map: state.map.map, origin: state.map.player_position,
                                destination: Point8 { x: pos.x, y: pos.y + 1 }, tile: MetaTile::Sprite(name), route });
                            break;
                        }
                        chosen
                    }
                },
                PolicyStep::BattleTrainer { trainer } => {
                    use crate::pokemon::map_metadata::PlayerFacingDirection;
                    if state.map.map != trainer.map() {
                        let action = Self::route_toward(world_graph, &state.map, &actions, trainer.map());
                        if action.is_none() { self.queue.pop_front(); continue; }
                        action
                    } else if let Some(sprite) = state.map.sprites.iter().find(|s| !s.hidden && s.name == trainer.name) {
                        let pos = sprite.position;
                        let cur = state.map.player_position;
                        // Approach from the side the trainer faces.
                        let approach = match sprite.facing {
                            crate::pokemon::sprite::SpriteFacing::Down => PlayerFacingDirection::Up,
                            crate::pokemon::sprite::SpriteFacing::Up => PlayerFacingDirection::Down,
                            crate::pokemon::sprite::SpriteFacing::Left => PlayerFacingDirection::Right,
                            crate::pokemon::sprite::SpriteFacing::Right => PlayerFacingDirection::Left,
                        };
                        match state.map.route_to_face_dir(pos, Some(approach)) {
                            // Standing in the trainer's LOS with no battle → it's beaten.
                            Some(route) if route.is_empty() => {
                                self.gym_engage = None;
                                self.queue.pop_front();
                                continue;
                            }
                            Some(route) => {
                                // The same frozen-spot test as the gym arm.
                                match self.gym_engage {
                                    Some((t, p, w)) if t == pos && p == cur => {
                                        if w + 1 > 40 { self.gym_engage = None; self.queue.pop_front(); continue; }
                                        self.gym_engage = Some((pos, cur, w + 1));
                                    }
                                    _ => self.gym_engage = Some((pos, cur, 0)),
                                }
                                Some(OverworldAction { map: state.map.map, origin: cur,
                                    destination: Point8 { x: pos.x, y: pos.y + 1 },
                                    tile: MetaTile::Sprite(trainer.name), route })
                            }
                            None => { self.queue.pop_front(); continue; }
                        }
                    } else {
                        // Trainer sprite absent (hidden/gone) — treat as done.
                        self.queue.pop_front();
                        continue;
                    }
                },
                PolicyStep::Interact(sprite) => {
                    // Prefer the sprite visible on the CURRENT map.
                    if let Some(action) = actions.iter().find(|a| a.tile == MetaTile::Sprite(sprite.name)) {
                        // A heal ends when the party is full, not when the talk lands.
                        if sprite.name == "Nurse" && !party_is_fresh(state)
                            && self.heal_waits < Self::MAX_HEAL_WAITS {
                            self.heal_waits += 1;
                            return Some(action.clone());
                        }
                        if self.heal_waits >= Self::MAX_HEAL_WAITS {
                            println!("[policy] the nurse on {} never finished healing — carrying on",
                                state.map.map);
                        }
                        self.heal_waits = 0;
                        self.queue.pop_front();
                        return Some(action.clone());
                    }
                    let map = sprite.map();
                    if state.map.map == map {
                        // Not actionable yet (walking on, or hidden by a script): wait.
                        None
                    } else {
                        let action = Self::route_toward(world_graph, &state.map, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to interact with {} on {}, but no path there!", sprite, map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    }
                }
                PolicyStep::InteractIfReachable(sprite) => {
                    // Reachable on the current map → walk to it (single-shot, like Interact).
                    if let Some(action) = actions.iter().find(|a| a.tile == MetaTile::Sprite(sprite.name)) {
                        self.interact_skip_waits = 0;
                        self.queue.pop_front();
                        return Some(action.clone());
                    }
                    let map = sprite.map();
                    if state.map.map == map {
                        // Loading, or walled off by the maze.
                        self.interact_skip_waits += 1;
                        if self.interact_skip_waits > 250 {
                            println!("[policy] {} unreachable after waiting — skipping", sprite);
                            self.interact_skip_waits = 0;
                            self.queue.pop_front();
                            continue;
                        }
                        None
                    } else {
                        let action = Self::route_toward(world_graph, &state.map, &actions, map);
                        if action.is_none() {
                            self.interact_skip_waits = 0;
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    }
                }
                PolicyStep::UsePc { map } => {
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &state.map, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to use the PC on {}, but no path there!", map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if let Some(action) = actions.iter().find(|a| a.tile == MetaTile::Pc) {
                        self.queue.pop_front();
                        return Some(action.clone());
                    } else {
                        // A script may still be running: wait.
                        None
                    }
                }
                PolicyStep::UseFlash { .. } => None, // on the map — `pick_field_move` drives the menu
                PolicyStep::Fish { map, .. } => {
                    // Routing only; `pick_field_move` hands each cast to the driver.
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &state.map, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to fish on {map}, but no path there!");
                            self.fish_casts = 0;
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        None
                    }
                }
                PolicyStep::SafariHunt { targets, map, max_trips } => {
                    use crate::pokemon::postgame::safari::Hunt;
                    match crate::pokemon::postgame::safari::pick(
                        &mut self.safari, state, world_graph, &actions, targets, map, max_trips)
                    {
                        Hunt::Walk(action) => Some(action),
                        Hunt::Wait => None,
                        Hunt::Done => { self.safari.reset(); self.queue.pop_front(); continue }
                    }
                }
                PolicyStep::SafariExit => {
                    use crate::pokemon::postgame::safari::Hunt;
                    match crate::pokemon::postgame::safari::exit(&mut self.safari, state, world_graph, &actions) {
                        Hunt::Walk(action) => Some(action),
                        Hunt::Wait => None,
                        Hunt::Done => { self.safari.reset(); self.queue.pop_front(); continue }
                    }
                }
                PolicyStep::BuyGameCoins { target } => match crate::pokemon::postgame::game_corner::buy_coins_action(state, &actions, world_graph, target) {
                    Some(action) => action,
                    None => { self.queue.pop_front(); continue }
                },
                PolicyStep::RedeemPrize { .. } if state.map.map != Map::GameCornerPrizeRoom => {
                    let action = Self::route_toward(world_graph, &state.map, &actions, Map::GameCornerPrizeRoom);
                    if action.is_none() {
                        println!("[policy] want a prize, but no path to the prize room!");
                        self.queue.pop_front();
                        continue;
                    }
                    action
                }
                PolicyStep::RedeemPrize { .. } => None, // on the map — handed to `pick_field_move`
                PolicyStep::PartyScript { .. } => None, // already routed by a preceding `enter` step
                PolicyStep::UseBagItem { .. } => None,  // `pick_field_move` owns the menus
                PolicyStep::UseItemsInBattle { on_map, items } => {
                    if state.map.map != on_map {
                        let action = Self::route_toward(world_graph, &state.map, &actions, on_map);
                        if action.is_none() {
                            println!("[policy] want a battle on {on_map} to use items in, but no path there!");
                            self.battle_item_baseline = None;
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        let baseline = self.battle_item_baseline.get_or_insert_with(||
                            items.iter().map(|&i| crate::pokemon::postgame::items::bag_quantity(&state, i)).collect());
                        let left: Vec<ItemId> = items.iter().zip(baseline.iter())
                            .filter(|&(&item, &was)| crate::pokemon::postgame::items::bag_quantity(&state, item) >= was)
                            .map(|(&item, _)| item)
                            .collect();
                        if left.is_empty() {
                            println!("[policy] UseItemsInBattle {on_map}: every item spent — done");
                            self.battle_item_baseline = None;
                            self.catch_wander_stuck = 0;
                            self.queue.pop_front();
                            continue;
                        }
                        // Nothing left in the bag to spend — say so rather than pace for ever.
                        if left.iter().all(|&i| crate::pokemon::postgame::items::bag_quantity(&state, i) == 0) {
                            println!("[policy] UseItemsInBattle {on_map}: none of {left:?} are in the bag — skipping");
                            self.battle_item_baseline = None;
                            self.catch_wander_stuck = 0;
                            self.queue.pop_front();
                            continue;
                        }
                        match actions.iter().find(|a| a.tile == MetaTile::Grass)
                            .cloned()
                            .or_else(|| state.map.wander_action())
                        {
                            Some(action) => { self.catch_wander_stuck = 0; Some(action) }
                            None => {
                                self.catch_wander_stuck += 1;
                                if self.catch_wander_stuck < 400 { None } else {
                                    println!("[policy] UseItemsInBattle {on_map}: nowhere to trigger an encounter!");
                                    self.battle_item_baseline = None;
                                    self.catch_wander_stuck = 0;
                                    self.queue.pop_front();
                                    continue;
                                }
                            }
                        }
                    }
                }
                PolicyStep::SellToMart { map, .. } | PolicyStep::UseItemPc { map, .. } | PolicyStep::UsePcBox { map, .. } => {
                    // Routing only.
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &state.map, &actions, map);
                        if action.is_none() {
                            println!("[policy] want the PC on {}, but no path there!", map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        None
                    }
                }
                PolicyStep::CollectItem(sprite) => {
                    let map = sprite.map();
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &state.map, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to collect {} on {}, but no path there!", sprite, map);
                            self.collect_item_waits = 0;
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        let present = state.map.sprites.iter().any(|s| !s.hidden && s.name == sprite.name);
                        if present { self.collect_item_seen = true; }
                        if !present && self.collect_item_seen {
                            // Seen and now gone: picked up, or removed by a script.
                            self.collect_item_seen = false;
                            self.collect_item_waits = 0;
                            self.queue.pop_front();
                            continue;
                        }
                        // Bounded: a refused pickup never makes the sprite go.
                        self.collect_item_waits += 1;
                        if self.collect_item_waits >= Self::MAX_COLLECT_ITEM_WAITS {
                            println!("[policy] gave up collecting {sprite} on {map} after {} polls{}",
                                self.collect_item_waits,
                                match state.bag.len() >= crate::pokemon::bag::Bag::MAX_ITEMS {
                                    true => format!(" — the bag is full ({} entries), which refuses \
                                        every pickup in the game", state.bag.len()),
                                    false => String::new(),
                                });
                            self.collect_item_waits = 0;
                            self.collect_item_seen = false;
                            self.queue.pop_front();
                            continue;
                        }
                        if !present {
                            // Not yet revealed (a ball hidden until its guard is beaten).
                            None
                        } else {
                            // Stay until the sprite goes, so a battle mid-walk drops nothing.
                            actions.iter()
                                .find(|a| a.tile == MetaTile::Sprite(sprite.name))
                                .cloned()
                        }
                    }
                }
                PolicyStep::BuyFromMart { item, map } => {
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &state.map, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to buy {} from {} but no path there!", item, map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if state.bag.iter().any(|i| i.id == item.id && i.quantity >= item.quantity) {
                        // Purchase registered (bag now holds ≥ the target quantity) — done.
                        self.mart_attempts = 0;
                        self.mart_baseline = None;
                        self.queue.pop_front();
                        continue;
                    } else if let Some((money, held)) = self.mart_baseline
                        && self.mart_attempts > 0
                        && state.money == money
                        && held == state.bag.iter().find(|i| i.id == item.id).map_or(0, |i| i.quantity)
                    {
                        // Neither money nor bag moved: the wallet, not a dropped confirm.
                        let bag_full = state.bag.len() >= crate::pokemon::bag::Bag::MAX_ITEMS
                            && !state.bag.iter().any(|i| i.id == item.id);
                        println!(
                            "[policy] bought {} of {} from {} — {}",
                            held, item, map,
                            if bag_full { format!("the bag is full at {} entries, and ¥{} was not the problem",
                                                  state.bag.len(), state.money) }
                            else { "the wallet covers no more".to_string() },
                        );
                        self.mart_attempts = 0;
                        self.mart_baseline = None;
                        self.queue.pop_front();
                        continue;
                    } else if self.mart_attempts >= Self::MAX_MART_ATTEMPTS {
                        println!("[policy] gave up buying {} from {} after {} attempts", item, map, self.mart_attempts);
                        self.mart_attempts = 0;
                        self.mart_baseline = None;
                        self.queue.pop_front();
                        continue;
                    } else {
                        // Talk to the clerk to (re)open the mart.
                        let action = actions.iter()
                            .find(|a| matches!(a.tile, MetaTile::Sprite(sprite) if sprite == "Clerk" || sprite == "Clerk 1"));

                        if action.is_none() {
                            println!("[policy] BuyFromMart step encountered in pick_overworld_action and no clerk available — skipping");
                            self.mart_attempts = 0;
                            self.queue.pop_front();
                            continue;
                        }

                        action.cloned()
                    }
                }
                PolicyStep::TeachMove { .. } => {
                    // Handled by `pick_field_move` (the agent calls it first).
                    None
                }
                PolicyStep::EvolveWithStone { .. } => {
                    // Handled by `pick_field_move` (bag menu chain); wait without advancing.
                    None
                }
                PolicyStep::UseRareCandy { .. } | PolicyStep::Dig { .. } | PolicyStep::TossItem { .. } => {
                    // Handled by `pick_field_move` (bag menu chain); wait without advancing.
                    None
                }
                PolicyStep::MovePokemonToFront { .. } | PolicyStep::Fly { .. } => {
                    // Handled by `pick_field_move`.
                    None
                }
                PolicyStep::UseStrength { .. } => {
                    // Handled by `pick_field_move`.
                    None
                }
                // One goal row carries the whole puzzle, as it does for a model.
                PolicyStep::SolveBoulders { switch, boulder } =>
                    Self::boulder_goal_action(state, &actions, switch, boulder, false),
                PolicyStep::DropBoulderInHole { hole, boulder } =>
                    Self::boulder_goal_action(state, &actions, hole, boulder, true),
                PolicyStep::CutTree { map } => {
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &state.map, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to cut a tree on {map} but no path there!");
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if matches!(state.map.tile_in_front(), Some((_, MetaTile::CutTree))) {
                        // Facing a tree — `pick_field_move` performs the cut; just wait.
                        None
                    } else {
                        // Face a reachable tree; when none remain, done.
                        match actions.iter().find(|a| matches!(a.tile, MetaTile::Cut { .. })).cloned() {
                            Some(action) => Some(action),
                            None => { self.queue.pop_front(); continue; }
                        }
                    }
                }
                PolicyStep::SolveTrashCans => {
                    if state.map.map != Map::VermilionGym {
                        let action = Self::route_toward(world_graph, &state.map, &actions, Map::VermilionGym);
                        if action.is_none() {
                            println!("[policy] want to solve trash cans but can't reach Vermilion Gym!");
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        // On the gym floor — `pick_field_move` drives checking the switch cans.
                        None
                    }
                }
                PolicyStep::FlipSwitch { map, .. } => {
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &state.map, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to flip a switch on {map} but can't reach it!");
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        // On the map — `pick_field_move` drives facing + pressing the switch.
                        None
                    }
                }
                PolicyStep::UseElevator { .. } => {
                    // Handled by `pick_field_move`; an `enter` put the agent here.
                    let in_elevator = matches!(state.map.map,
                        Map::RocketHideoutElevator | Map::SilphCoElevator | Map::CeladonMartElevator);
                    if !in_elevator {
                        println!("[policy] UseElevator but not in the elevator room ({});", state.map.map);
                        self.queue.pop_front();
                        continue;
                    }
                    None
                }
                PolicyStep::UseFieldItem { .. } => {
                    // Handled by `pick_field_move` once the target sprite is on this map.
                    None
                }
                PolicyStep::UseVendingMachine { .. } => None, // driven by `pick_field_move`
            }
        }
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        let battle_state = state.battle.as_ref()?;
        let actions = battle_options(state)?;
        // Where a black-out will be reported against — see `last_battle_map`.
        self.last_battle_map = Some(state.map.map);

        // No Centre detours in a Centre-less dungeon: fleeing the puzzle abandons it.
        let in_center_less_dungeon = matches!(state.map.map,
            Map::VictoryRoad2F | Map::VictoryRoad3F
            | Map::SeafoamIslands1F | Map::SeafoamIslandsB1F | Map::SeafoamIslandsB2F
            | Map::SeafoamIslandsB3F | Map::SeafoamIslandsB4F);

        // A ghost battle offers only `Run` (`battle::is_ghost_battle`).
        if is_ghost_battle(state.map.map, &state.bag, battle_state.battle_type) {
            return Some(BattleAction::Run);
        }

        if battle_state.battle_type == BattleType::Safari {
            if let Some(&PolicyStep::SafariHunt { targets, .. }) = self.queue.front() {
                if let Some(action) = crate::pokemon::postgame::safari::pick_battle_action(state, targets, &actions) {
                    return Some(action);
                }
            }
            return Some(BattleAction::Run);
        }

        if let Some(&PolicyStep::Fish { goal, .. }) = self.queue.front() {
            if let Some(action) = crate::pokemon::postgame::fishing::pick_battle_action(state, goal, &actions) {
                return Some(action);
            }
        }

        if let Some(&PolicyStep::UseItemsInBattle { items, .. }) = self.queue.front() {
            use crate::pokemon::postgame::items;
            if battle_state.battle_type == BattleType::Wild {
                if let Some(baseline) = self.battle_item_baseline.clone() {
                    let next = items.iter().zip(baseline.iter())
                        .find(|&(&item, &was)| items::bag_quantity(state, item) >= was && was > 0)
                        .map(|(&item, _)| item);
                    if let Some(item) = next {
                        if let Some(action) = actions.iter().find(|a|
                            matches!(a, BattleAction::UseItem { item: b, .. } if b.id == item)) {
                            println!("[policy] UseItemsInBattle: using {item:?}");
                            return Some(action.clone());
                        }
                    } else {
                        if let Some(run) = actions.iter().find(|a| matches!(a, BattleAction::Run)) {
                            return Some(*run);
                        }
                    }
                }
            }
        }

        if self.heal_return.is_some() && battle_state.battle_type == BattleType::Wild {
            // Returning to the pokemon center, run from battles.
            if let Some(center) = self.last_pokemon_center {
                println!("[policy] PP critically low — fleeing and routing to {center} to heal");
            }
            return Some(BattleAction::Run);
        }

        // ── Low-PP flee: run from wild battles and detour to the last Centre ──
        if battle_state.battle_type == BattleType::Wild
            && self.heal_return.is_none()
            && !self.heal_unreachable
            && !in_center_less_dungeon
            && all_damaging_moves_low_pp(&actions)
        {
            if let Some(center) = self.last_pokemon_center {
                println!("[policy] PP critically low — fleeing and routing to {center} to heal");
                self.heal_return = Some(center);
                return Some(BattleAction::Run);
            } else {
                println!("[policy] PP critically low but no known Pokémon Center to return to — fighting on");
            }
        }

        // Fainted: send the healthiest member.
        if battle_state.player.current_hp == 0 {
            return actions.iter()
                .filter(|a| matches!(a, BattleAction::SwitchPokemon { .. }))
                .max_by_key(|a| match a {
                    BattleAction::SwitchPokemon { pokemon, .. } => pokemon.current_hp,
                    _ => 0,
                })
                .cloned();
        }

        // ── Flee obstacle wilds: in a Centre-less dungeon each one drains PP toward a black-out ──
        if battle_state.battle_type == BattleType::Wild
            && actions.iter().any(|a| matches!(a, BattleAction::Run))
        {
            let flee = match self.queue.front() {
                Some(PolicyStep::CatchPokemon { .. }) | Some(PolicyStep::SweepDex { .. }) =>
                    self.catch_target(state, battle_state.enemy.species).is_none(),
                // A grind is for the encounter, so the obstacle rule is inverted for it.
                Some(PolicyStep::GrindUntilLevel { .. }) => false,
                _ => in_center_less_dungeon,
            };
            if flee {
                return Some(BattleAction::Run);
            }
        }

        // Train a bench mon by switching it in so it — not the lead — earns the XP.
        let is_grinding = matches!(self.queue.front(), Some(&PolicyStep::GrindUntilLevel { .. }));
        let train_slot = match self.queue.front() {
            Some(&PolicyStep::GrindUntilLevel { target, .. }) if battle_state.battle_type == BattleType::Wild =>
                target.resolve(state),
            _ => self.train_slot,
        };
        if let Some(slot) = train_slot {
            // ── Grind hand-off: an underlevelled trainee is one-shot before a low-HP reaction,
            // and a faint far from a Centre stalls the run ──
            let enemy_threatens = battle_state.enemy.level + 6 >= battle_state.player.level;
            if is_grinding && battle_state.active_party_slot == slot && !self.trainee_participated
                && enemy_threatens
            {
                if let Some(sw) = actions.iter()
                    // The tank out-levels the trainee and has over 25% HP, so it stays a hand-off.
                    .filter(|a| matches!(a, BattleAction::SwitchPokemon { pokemon, .. }
                        if pokemon.current_hp as u32 * 4 > pokemon.stats.hp as u32 && pokemon.level > battle_state.player.level))
                    .max_by_key(|a| match a { BattleAction::SwitchPokemon { pokemon, .. } => pokemon.level, _ => 0 })
                {
                    println!("[policy] grind: trainee (slot {slot}) participated — handing off to a tank");
                    self.trainee_participated = true;
                    return Some(sw.clone());
                }
            }
            if battle_state.active_party_slot != slot && !(is_grinding && self.trainee_participated) {
                if let Some(sw) = actions.iter().find(|a| matches!(a,
                    BattleAction::SwitchPokemon { slot: s, pokemon }
                        if *s == slot && pokemon.current_hp > 0
                        && battle_state.enemy.level <= pokemon.level + 6)) {
                    println!("[policy] training slot {slot} — switching it in to take the XP");
                    self.trainee_participated = true;
                    return Some(sw.clone());
                }
            }
        }

        // Above the catch throw, or unreachable for a whole hunt: a throw is a turn not defending.
        if battle_state.player.remaining_hp() < 0.25 {
            let potion_rank = |id: ItemId| match id {
                ItemId::FullRestore => 4, ItemId::MaxPotion => 3, ItemId::HyperPotion => 2,
                ItemId::SuperPotion => 1, ItemId::Potion => 0, _ => -1,
            };
            let heal = actions.iter()
                .filter(|a| matches!(a, BattleAction::UseItem { item, .. } if potion_rank(item.id) >= 0))
                .max_by_key(|a| match a { BattleAction::UseItem { item, .. } => potion_rank(item.id), _ => -1 });
            if let Some(heal_action) = heal {
                println!("[policy] HP critical ({:.0}%) — using healing item", battle_state.player.remaining_hp() * 100.0);
                return Some(heal_action.clone());
            }
        }

        // When catching, throw a Pokéball immediately if one is available.
        if let Some(ball) = self.catch_target(state, battle_state.enemy.species) {
            let species = &battle_state.enemy.species;
            if battle_state.battle_type == BattleType::Wild {
                // A step may pin its ball so an incidental catch spares the Master Ball.
                let chosen = ball
                    .and_then(|id| state.bag.iter().find(|i| i.id == id && i.quantity > 0))
                    .or_else(|| state.bag.best_pokeball());
                if let Some(best_pokeball) = chosen {
                    if let Some(use_pokeball_action) = actions.iter()
                        .find(|a| matches!(a, BattleAction::UseItem { item, .. } if item.id == best_pokeball.id )) {

                        // Catch-rate-3 targets (legendaries) are paralysed, then only thrown at.
                        if let Some(action) = crate::pokemon::postgame::legendaries::pre_catch_action(state, *species, &actions, Some(use_pokeball_action)) {
                            return Some(action);
                        }

                        // Weaken above half HP without knocking out, except for a Master Ball or a
                        // first throw at a much weaker target.
                        let thrown = self.catch_ball_baseline.get_or_insert(best_pokeball.quantity)
                            .saturating_sub(best_pokeball.quantity);
                        if battle_state.enemy.remaining_hp() > 0.5
                            && best_pokeball.id != ItemId::MasterBall
                            && (thrown > 0 || battle_state.player.level < battle_state.enemy.level + 12) {
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

        // Switch to the healthiest party member if below 15% HP and a better option exists.
        let grinding = self.train_slot.is_some()
            || matches!(self.queue.front(), Some(PolicyStep::GrindUntilLevel { .. }));
        if !grinding && battle_state.player.remaining_hp() < 0.15 {
            if let Some(switch) = actions.iter()
                .filter(|a| matches!(a, BattleAction::SwitchPokemon { .. }))
                .max_by_key(|a| match a {
                    BattleAction::SwitchPokemon { pokemon, .. } => pokemon.current_hp,
                    _ => 0,
                })
            {
                if let BattleAction::SwitchPokemon { pokemon, .. } = switch {
                    // Only a real alternative: over half its HP and at least the active level.
                    let healthy_enough = pokemon.stats.hp > 0
                        && pokemon.current_hp as u32 * 2 > pokemon.stats.hp as u32;
                    let strong_enough = pokemon.level >= battle_state.player.level;
                    if healthy_enough && strong_enough && pokemon.current_hp > battle_state.player.current_hp {
                        println!("[policy] HP critical — switching to {} (lv{} {}/{}hp)",
                            pokemon.species, pokemon.level, pokemon.current_hp, pokemon.stats.hp);
                        return Some(*switch);
                    }
                }
            }
        }

        // No potion and no healthy team-mate: flee and heal rather than fight to a faint.
        let flee_below = match state.bag.iter().any(|i| matches!(i.id,
            ItemId::Potion | ItemId::SuperPotion | ItemId::HyperPotion | ItemId::MaxPotion
            | ItemId::FullRestore) && i.quantity > 0) {
            true => 0.15,
            false => 0.34,
        };
        if !grinding
            && !in_center_less_dungeon
            && battle_state.battle_type == BattleType::Wild
            && self.heal_return.is_none()
            && battle_state.player.remaining_hp() < flee_below
        {
            match self.last_pokemon_center {
                Some(center) => {
                    println!("[policy] HP critical, no heal/switch — fleeing to {center} to heal");
                    self.heal_return = Some(center);
                }
                None => println!("[policy] HP critical and no Centre known yet — fleeing anyway"),
            }
            return Some(BattleAction::Run);
        }

        // Elite Four: when the active mon's best move does under a third of the enemy's HP and a
        // healthy bench mon hits much harder, switch.
        if battle_state.battle_type == BattleType::Trainer {
            // `pp > 0`: a 0-PP move scored here livelocks the switch.
            let move_dmg = |mon: &crate::pokemon::pokemon::PokemonSummary| -> u32 { mon.moves.iter().flatten()
                .filter(|m| m.pp > 0)
                .filter_map(|m| expected_damage(mon, m.name, &battle_state.enemy).map(|d| d as u32))
                .max().unwrap_or(0) };
            let active_best = move_dmg(&battle_state.player);
            if (active_best * 3) < battle_state.enemy.stats.hp as u32 {
                let best_switch = actions.iter()
                    .filter(|a| matches!(a, BattleAction::SwitchPokemon { pokemon, .. }
                        if pokemon.current_hp as u32 * 2 > pokemon.stats.hp as u32))
                    .filter_map(|a| match a {
                        BattleAction::SwitchPokemon { pokemon, .. } => Some((move_dmg(pokemon), a)),
                        _ => None,
                    })
                    .max_by_key(|(d, _)| *d);
                if let Some((bench_dmg, sw)) = best_switch {
                    if bench_dmg * 2 >= active_best * 3 && (bench_dmg * 3) >= battle_state.enemy.stats.hp as u32 {
                        println!("[policy] active out of strong moves (dmg {active_best}) — switching to a fresher attacker (dmg {bench_dmg})");
                        return Some(sw.clone());
                    }
                }
            }
        }

        let result = pick_best_move(&battle_state, &actions, false);
        if result.is_some() {
            return result;
        }

        // No damaging move on the active Pokémon (out of PP, or all resisted to 0 damage).
        if let Some(switch) = actions.iter()
            .filter_map(|a| match a {
                BattleAction::SwitchPokemon { pokemon, .. } => {
                    let best = pokemon.moves.iter().flatten()
                        .filter(|m| m.pp > 0)
                        .filter_map(|m| expected_damage(pokemon, m.name, &battle_state.enemy))
                        .max().unwrap_or(0);
                    (best > 0).then_some((best, a))
                }
                _ => None,
            })
            .max_by_key(|(dmg, _)| *dmg)
            .map(|(_, a)| a)
        {
            println!("[policy] no damaging move available — switching to an attacker");
            return Some(switch.clone());
        }

        // No party member can damage the enemy.
        if let Some(a) = actions.iter()
            .filter(|a| matches!(a,
                BattleAction::Fight { battle_move, .. } if battle_move.name != PokemonMoveName::LeechSeed))
            .choose(&mut self.rng)
        {
            return Some(a.clone());
        }

        // Last resort: a Fight move (Struggle), else any non-item action.
        let last_resort = actions.iter().find(|a| matches!(a, BattleAction::Fight { .. }))
            .or_else(|| actions.iter().find(|a| !matches!(a, BattleAction::UseItem { .. })))
            .or_else(|| actions.iter().find(|a| matches!(a, BattleAction::Fight { .. })))
            .cloned();
        if last_resort.is_none() {
            // A `None` for ever looks like thinking and the watchdog never fires, so say so.
            println!("[policy] no battle action to take against {} — options {:?}",
                battle_state.enemy.species, actions);
        }
        last_resort
    }

    fn pick_nickname(&mut self, _species: PokemonSpecies) -> Option<Option<String>> {
        let name = self.name_picker.pick().to_string();
        println!("[policy] pick name={}", name);
        Some(Some(name))
    }

    fn pick_move_to_forget(
        &mut self,
        _party_slot: usize,
        current_moves: &[PokemonMove],
        new_move: PokemonMoveName,
    ) -> Option<Option<usize>> {
        let is_hm = |m: PokemonMoveName| matches!(m, PokemonMoveName::Cut | PokemonMoveName::Fly
            | PokemonMoveName::Surf | PokemonMoveName::Strength | PokemonMoveName::Flash);
        let value = |m: PokemonMoveName| if is_hm(m) { u16::MAX }
            else { m.metadata().power.unwrap_or(0) as u16 + if is_damaging_move(m) { 1 } else { 0 } };

        let slot = current_moves.iter().enumerate()
            .min_by_key(|(_, m)| value(m.name))
            .map(|(i, _)| i)?;
        println!("[policy] learning {new_move:?} — forgetting slot {slot} ({:?})",
            current_moves.get(slot).map(|m| m.name));
        Some(Some(slot))
    }

    fn pick_field_move(&mut self, state: &GameState) -> Option<FieldMove> {
        // Cut a tree the player is already facing (routed there by the CutTree overworld action).
        if let Some(&PolicyStep::CutTree { map }) = self.queue.front() {
            if state.map.map == map
                && matches!(state.map.tile_in_front(), Some((_, MetaTile::CutTree)))
            {
                return Some(FieldMove::CutTree);
            }
        }
        if let Some(&PolicyStep::UseItemPc { op, item, qty, map }) = self.queue.front() {
            // Wait until we are on the right map — `pick_overworld_action` is doing the routing.
            if state.map.map == map {
                match crate::pokemon::tile_map::pc_locations_for(map).first() {
                    // Popped on issue: the driver owns it to completion.
                    Some(&pc) => {
                        self.queue.pop_front();
                        return Some(FieldMove::UseItemPc { op, item, qty, pc });
                    }
                    None => {
                        println!("[policy] UseItemPc: {map} has no PC — skipping");
                        self.queue.pop_front();
                        return None;
                    }
                }
            }
        }
        if let Some(&PolicyStep::UsePcBox { op, map }) = self.queue.front() {
            // Popped on issue, as `UseItemPc`.
            if state.map.map == map {
                self.queue.pop_front();
                return match crate::pokemon::tile_map::pc_locations_for(map).first() {
                    Some(&pc) => Some(FieldMove::UsePcBox { op, pc }),
                    None => { println!("[policy] UsePcBox: {map} has no PC — skipping"); None }
                };
            }
        }
        if let Some(&PolicyStep::PartyScript { script, slot }) = self.queue.front() {
            if state.map.map == script.map() {
                self.queue.pop_front();
                return crate::pokemon::postgame::gifts::pick(state, script, slot);
            }
        }
        if let Some(&PolicyStep::SellToMart { map, item }) = self.queue.front() {
            if state.map.map == map {
                self.queue.pop_front();
                return crate::pokemon::postgame::game_corner::pick_sale(state, item);
            }
        }
        if let Some(&PolicyStep::RedeemPrize { prize }) = self.queue.front() {
            if state.map.map == Map::GameCornerPrizeRoom {
                self.queue.pop_front();
                return crate::pokemon::postgame::game_corner::pick_prize(state, prize);
            }
        }
        if let Some(&PolicyStep::Fish { rod, map, goal }) = self.queue.front() {
            if state.map.map == map {
                use crate::pokemon::postgame::fishing;
                if fishing::goal_met(state, goal, self.fish_casts) {
                    println!("[policy] Fish: {goal:?} met after {} casts — done", self.fish_casts);
                    self.fish_casts = 0;
                    self.queue.pop_front();
                    return None;
                }
                match fishing::pick(state, rod) {
                    Some(field_move) => { self.fish_casts += 1; return Some(field_move); }
                    None => {
                        println!("[policy] Fish: no water on {map} the player can stand next to — skipping");
                        self.fish_casts = 0;
                        self.queue.pop_front();
                        return None;
                    }
                }
            }
        }
        if let Some(&PolicyStep::UseBagItem { item, target }) = self.queue.front() {
            use crate::pokemon::postgame::items;
            let baseline = *self.item_use_baseline
                .get_or_insert_with(|| items::baseline(state, item));
            match items::pick(state, item, target, baseline, self.item_use_attempts) {
                Ok(field_move) => {
                    if self.item_use_attempts >= Self::MAX_ITEM_USE_ATTEMPTS {
                        println!("[policy] UseBagItem: gave up on {item:?} after {} attempts",
                            self.item_use_attempts);
                        self.item_use_attempts = 0;
                        self.item_use_baseline = None;
                        self.queue.pop_front();
                        return None;
                    }
                    self.item_use_attempts += 1;
                    return Some(field_move);
                }
                Err(why) => {
                    println!("[policy] UseBagItem: {why} — done");
                    self.item_use_attempts = 0;
                    self.item_use_baseline = None;
                    self.queue.pop_front();
                    return None;
                }
            }
        }
        if let Some(&PolicyStep::Fly { to }) = self.queue.front() {
            self.queue.pop_front();
            return Some(FieldMove::Fly { to });
        }
        if let Some(&PolicyStep::MovePokemonToFront { target }) = self.queue.front() {
            let Some(slot) = target.resolve(state) else {
                println!("[policy] MovePokemonToFront: {target:?} is not in the party — skipping");
                self.queue.pop_front();
                return None;
            };
            self.queue.pop_front();
            return Some(FieldMove::ReorderParty { slot });
        }
        if let Some(&PolicyStep::GrindUntilLevel { on_map, target, .. }) = self.queue.front() {
            // The trainee leads the grind, so it takes the whole battle and the whole XP.
            if state.map.map == on_map
                && let Some(slot) = target.resolve(state)
                && slot != 0
                && state.pokemon.get(usize::from(slot)).is_some_and(|mon| mon.current_hp > 0)
            {
                println!("[policy] grind: leading with slot {slot} so it takes the whole battle and the whole XP");
                return Some(FieldMove::ReorderParty { slot });
            }
            // Cure the tick here, or pay for it as a four-warp round trip to a Pokémon Centre.
            if state.map.map == on_map
                && let Some(slot) = target.resolve(state)
                && let Some(mon) = state.pokemon.get(usize::from(slot))
                && mon.current_hp > 0
                && matches!(mon.status, crate::pokemon::status::PokemonStatus::Poisoned
                                      | crate::pokemon::status::PokemonStatus::Burned)
                // On sight, and *not* on a low-HP threshold — a Full Heal restores no HP.
                && let Some(cure) = [ItemId::FullHeal, ItemId::Antidote].into_iter()
                    .find(|&id| state.bag.iter().any(|i| i.id == id && i.quantity > 0))
                // An Antidote answers only poison; a Full Heal both.
                && (cure == ItemId::FullHeal
                    || mon.status == crate::pokemon::status::PokemonStatus::Poisoned)
            {
                println!("[policy] grind: {:?} is {:?} — curing it with a {cure:?} rather than walking to a Centre",
                    mon.species, mon.status);
                return Some(FieldMove::UseBagItem { item: cure,
                    target: crate::pokemon::postgame::items::UseTarget::Party { slot } });
            }
        }
        if let Some(&PolicyStep::UseFlash { slot }) = self.queue.front() {
            if !state.map_is_dark {
                println!("[policy] UseFlash: {} is lit — done", state.map.map);
                self.queue.pop_front();
                return None;
            }
            let (slot, move_index) = field_move_carrier(state, PokemonMoveName::Flash)
                .unwrap_or((slot, field_move_index(state, slot, PokemonMoveName::Flash)));
            return Some(FieldMove::UseFieldMove { slot, move_index });
        }
        if let Some(&PolicyStep::UseStrength { target }) = self.queue.front() {
            if state.strength_active {
                println!("[policy] UseStrength: BIT_STRENGTH_ACTIVE set — done");
                self.queue.pop_front();
                return None;
            }
            // "Not in the party" has two meanings and only one of them is worth waiting for.
            if self.target_was_abandoned(target) {
                println!("[policy] UseStrength: {target:?} was never caught — skipping");
                self.queue.pop_front();
                return None;
            }
            let Some(slot) = target.resolve(state) else {
                println!("[policy] UseStrength: {target:?} is not in the party — waiting");
                return None;
            };
            let (slot, move_index) = field_move_carrier(state, PokemonMoveName::Strength)
                .unwrap_or((slot, field_move_index(state, slot, PokemonMoveName::Strength)));
            return Some(FieldMove::UseFieldMove { slot, move_index });
        }
        if let Some(&PolicyStep::SolveBoulders { switch, .. }) = self.queue.front() {
            // Done once a boulder sits on the switch (the map script then opens the barrier).
            let boulder_on_switch = state.map.sprites.iter()
                .any(|s| s.name.starts_with("Boulder") && !s.hidden && s.position == switch);
            if boulder_on_switch {
                println!("[policy] SolveBoulders: boulder on switch {switch} — done");
                self.queue.pop_front();
                return None;
            }
            // Handed to the agent as one goal, not dripped one shove at a time.
            return None;
        }
        if let Some(&PolicyStep::DropBoulderInHole { hole, boulder }) = self.queue.front() {
            // Exact when the step names its boulder, and it has to be.
            let done = match boulder {
                Some(which) => !state.map.boulders().contains(&which),
                None => {
                    let visible = state.map.sprites.iter()
                        .filter(|s| s.name.starts_with("Boulder") && !s.hidden).count();
                    let baseline = *self.boulder_drop_baseline.get_or_insert(visible);
                    visible < baseline
                }
            };
            if done {
                println!("[policy] DropBoulderInHole: a boulder fell into {hole} — done");
                self.boulder_drop_baseline = None;
                self.queue.pop_front();
                return None;
            }
            // Same as the switch above: the goal row carries the whole thing.
            return None;
        }
        if let Some(&PolicyStep::TeachMove { item, target }) = self.queue.front() {
            // Resolved every tick: a `Species` target may still be a ball on the floor.
            let resolved = target.resolve(state);
            let already_knows = hm_move(item).map_or(false, |mv| {
                resolved.and_then(|slot| state.pokemon.get(slot as usize))
                    .map_or(false, |p| p.moves.iter().flatten().any(|m| m.name == mv))
            });
            if already_knows {
                println!("[policy] TeachMove: {target:?} already knows the move — done");
                self.queue.pop_front();
                return None;
            }
            // A TM never picked up cannot be taught, and the driver would loop looking for it.
            if !state.bag.iter().any(|b| b.id == item) {
                println!("[policy] TeachMove: {item:?} is not in the bag — skipping");
                self.queue.pop_front();
                return None;
            }
            // "Not in the party" has two meanings and only one of them is worth waiting for.
            if self.target_was_abandoned(target) {
                println!("[policy] TeachMove: {target:?} was never caught — skipping");
                self.queue.pop_front();
                return None;
            }
            let Some(target_slot) = resolved else {
                println!("[policy] TeachMove: {target:?} is not in the party — waiting");
                return None;
            };
            // The cartridge refuses a machine outside the learnset back into the party menu,
            // which the driver cannot leave.
            if state.pokemon.get(target_slot as usize)
                .is_some_and(|mon| !crate::pokemon::learnset::can_learn(mon.species, item)) {
                println!("[policy] TeachMove: {target:?} cannot learn {item:?} — skipping");
                self.queue.pop_front();
                return None;
            }
            return Some(FieldMove::TeachMove { item, target_slot });
        }
        if let Some(&PolicyStep::UseRareCandy { slot }) = self.queue.front() {
            // Done once the Rare Candy is gone (consumed).
            if !state.bag.iter().any(|b| b.id == ItemId::RareCandy) {
                println!("[policy] UseRareCandy: consumed — done");
                self.queue.pop_front();
                return None;
            }
            return Some(FieldMove::TeachMove { item: ItemId::RareCandy, target_slot: slot });
        }
        if let Some(&PolicyStep::TossItem { item }) = self.queue.front() {
            if !state.bag.iter().any(|b| b.id == item) {
                println!("[policy] TossItem: no {item:?} in the bag — done");
                self.queue.pop_front();
                return None;
            }
            return Some(FieldMove::TossItem { item });
        }
        if let Some(&PolicyStep::Dig { target }) = self.queue.front() {
            // Done when Dig has warped us off this map.
            match self.dig_from_map {
                Some(from) if from != state.map.map => {
                    println!("[policy] Dig: out of {from} → {} — done", state.map.map);
                    self.dig_from_map = None;
                    self.queue.pop_front();
                    return None;
                }
                None => self.dig_from_map = Some(state.map.map),
                _ => {}
            }
            // "Not in the party" has two meanings and only one of them is worth waiting for.
            if self.target_was_abandoned(target) {
                println!("[policy] Dig: {target:?} was never caught — skipping");
                self.queue.pop_front();
                return None;
            }
            let Some(slot) = target.resolve(state) else {
                println!("[policy] Dig: {target:?} is not in the party — waiting");
                return None;
            };
            let (slot, move_index) = field_move_carrier(state, PokemonMoveName::Dig)
                .unwrap_or((slot, field_move_index(state, slot, PokemonMoveName::Dig)));
            return Some(FieldMove::UseFieldMove { slot, move_index });
        }
        if let Some(&PolicyStep::EvolveWithStone { stone, target }) = self.queue.front() {
            // Evolved once the species at the target differs from the one the step started with.
            let current = target.resolve(state).and_then(|slot| state.pokemon.get(slot as usize))
                .map(|p| p.species);
            let Some(current) = current else {
                // A `Species` target gone has evolved away; a `Slot` one is off the end.
                println!("[policy] EvolveWithStone: {target:?} is not in the party — done");
                self.evolve_baseline = None;
                self.queue.pop_front();
                return None;
            };
            if self.evolve_baseline.map_or(true, |(who, _)| who != target) {
                self.evolve_baseline = Some((target, current));
            }
            let evolve_from = self.evolve_baseline.expect("just set").1;
            if current != evolve_from {
                println!("[policy] EvolveWithStone: {target:?} is now {current:?} — done");
                self.evolve_baseline = None;
                self.queue.pop_front();
                return None;
            }
            let target_slot = target.resolve(state).expect("resolved just above");
            return Some(FieldMove::EvolveWithStone { stone, target_slot, evolve_from });
        }
        if let Some(&PolicyStep::SolveTrashCans) = self.queue.front() {
            if let Some(puzzle) = &state.trash_cans {
                if puzzle.second_opened {
                    println!("[policy] SolveTrashCans: both locks open — door unlocked");
                    self.queue.pop_front();
                    return None;
                }
                let target = if puzzle.first_opened { puzzle.second_target } else { puzzle.first_target };
                return Some(FieldMove::CheckTrashCan { target, facing: None });
            }
        }
        if let Some(&PolicyStep::FlipSwitch { map, at, reveals }) = self.queue.front() {
            if state.map.map == map {
                if is_mansion_floor(map) {
                    // Pokémon Mansion: one global switch toggles every floor's gates.
                    let baseline = *self.mansion_flip_baseline.get_or_insert(state.mansion_switch_on);
                    if state.mansion_switch_on != baseline {
                        println!("[policy] FlipSwitch: Mansion switch toggled to {} — done", state.mansion_switch_on);
                        self.mansion_flip_baseline = None;
                        self.queue.pop_front();
                        return None;
                    }
                    // Statue switches only trigger when faced from directly below (facing Up).
                    return Some(FieldMove::CheckTrashCan { target: at,
                        facing: Some(crate::pokemon::map_metadata::PlayerFacingDirection::Up) });
                }
                // Non-Mansion (Rocket Hideout poster): done once the passage to `reveals` opens.
                let done = match reveals {
                    Map::RocketHideoutB1F => state.found_rocket_hideout,
                    _ => state.map.actions().iter().any(|a| matches!(a.tile,
                        MetaTile::Warp { to_map, .. } if to_map == reveals)),
                };
                if done {
                    println!("[policy] FlipSwitch: {reveals} passage revealed — done");
                    self.queue.pop_front();
                    return None;
                }
                return Some(FieldMove::CheckTrashCan { target: at, facing: None });
            }
        }
        if let Some(&PolicyStep::UseElevator { panel, floor }) = self.queue.front() {
            // Done once ridden out of the elevator room.
            let in_elevator = matches!(state.map.map,
                Map::RocketHideoutElevator | Map::SilphCoElevator | Map::CeladonMartElevator);
            if !in_elevator {
                self.queue.pop_front();
                return None;
            }
            return Some(FieldMove::UseElevator { panel, floor });
        }
        if let Some(&PolicyStep::UseFieldItem { item, target }) = self.queue.front() {
            let present = state.map.sprites.iter().any(|s| !s.hidden && s.name == target.name);
            if present { self.collect_item_seen = true; }
            // Seen and now gone: the item's effect removed it (the Snorlax).
            if !present && self.collect_item_seen {
                self.collect_item_seen = false;
                println!("[policy] UseFieldItem: {} gone — done", target.name);
                self.queue.pop_front();
                return None;
            }
            if !present { return None; } // target not yet observed on this map — keep walking/waiting
            let pos = state.map.sprites.iter()
                .find(|s| !s.hidden && s.name == target.name)
                .map(|s| s.position)?;
            return Some(FieldMove::UseFieldItem { item, target: pos });
        }
        if let Some(&PolicyStep::UseVendingMachine { at, drink }) = self.queue.front() {
            if state.bag.contains(&drink) {
                println!("[policy] UseVendingMachine: bought {drink:?} — done");
                self.queue.pop_front();
                return None;
            }
            // The vending menu opens on the cheapest drink, so A buys it.
            return Some(FieldMove::CheckTrashCan { target: at, facing: None });
        }
        None
    }

    fn pick_mart_purchase(&mut self, state: &GameState) -> Option<Option<BagItem>> {
        let result = match self.queue.front() {
            Some(PolicyStep::BuyFromMart { item, .. }) => {
                self.mart_attempts += 1;
                // So `pick_overworld_action` can tell an empty wallet from a dropped confirm.
                self.mart_baseline = Some((
                    state.money,
                    state.bag.iter().find(|entry| entry.id == item.id).map_or(0, |entry| entry.quantity),
                ));
                println!("[policy] BuyFromMart: {:?} (attempt {})", item, self.mart_attempts);
                Some(*item)
            }
            _ => {
                println!("[policy] pick_mart_purchase called but no BuyFromMart step queued — returning None");
                None
            },
        };

        Some(result)
    }

    fn is_exhausted(&self) -> bool {
        self.queue.is_empty()
    }

    fn restart(&mut self, run_dir: Option<&std::path::Path>) {
        let Some(mut cursor) = self.progress.take() else { return };
        scripted_progress::clear(&cursor.dir);
        if let Some(dir) = run_dir { cursor.dir = dir.to_path_buf(); }
        cursor.written = usize::MAX;
        *self = Self::new(self.seed, cursor.full_route.iter().cloned());
        self.progress = Some(cursor);
        println!("[policy] new run — the scripted route starts again at step 0");
        self.record_progress();
    }

    fn steps_remaining(&self) -> Option<usize> {
        Some(self.queue.len())
    }

    fn current_step_is_long_running(&self) -> bool {
        matches!(
            self.queue.front(),
            Some(PolicyStep::GrindUntilLevel { .. })
                | Some(PolicyStep::CatchPokemon { .. })
                | Some(PolicyStep::SweepDex { .. })
                | Some(PolicyStep::CollectItem(_))
                | Some(PolicyStep::DefeatGymLeader { .. })
                | Some(PolicyStep::BattleTrainer { .. })
                | Some(PolicyStep::FlipSwitch { .. })
                | Some(PolicyStep::Fish { .. })
                | Some(PolicyStep::SafariHunt { .. })
                | Some(PolicyStep::SafariExit)
                | Some(PolicyStep::UseItemsInBattle { .. })
                // Carries its own bound (`MAX_ENTER_WAIT`) that can outlast the stall window.
                | Some(PolicyStep::EnterMapIfReachable { .. })
        )
    }
}
#[cfg(test)]
mod move_learn_tests {
    use super::*;
    use crate::pokemon::move_name::PokemonMoveName::*;

    fn mv(name: crate::pokemon::move_name::PokemonMoveName) -> PokemonMove {
        PokemonMove::with_max_pp(name)
    }

    #[test]
    fn keeps_damaging_moves_when_learning_status() {
        let mut p = DeterministicPolicy::new(0, Vec::<PolicyStep>::new());
        // Ivysaur learning Poisonpowder.
        let moves = [mv(Tackle), mv(Growl), mv(LeechSeed), mv(VineWhip)];
        let slot = p.pick_move_to_forget(0, &moves, Poisonpowder).flatten().expect("should pick a slot");
        assert!(slot == 1 || slot == 2,
            "forgot slot {slot} ({:?}) — must forget a status move, not Tackle/Vine Whip", moves[slot].name);
    }

    #[test]
    fn learns_strong_move_over_status() {
        let mut p = DeterministicPolicy::new(0, Vec::<PolicyStep>::new());
        let moves = [mv(Tackle), mv(Growl), mv(LeechSeed), mv(VineWhip)];
        let slot = p.pick_move_to_forget(0, &moves, RazorLeaf).flatten().unwrap();
        assert!(slot == 1 || slot == 2, "should forget a status move to learn Razor Leaf");
    }

    #[test]
    fn never_forgets_hm() {
        let mut p = DeterministicPolicy::new(0, Vec::<PolicyStep>::new());
        let moves = [mv(Cut), mv(Growl), mv(LeechSeed), mv(Poisonpowder)];
        let slot = p.pick_move_to_forget(0, &moves, Poisonpowder).flatten().unwrap();
        assert_ne!(moves[slot].name, Cut, "must never forget an HM move (Cut)");
    }
}

#[cfg(test)]
mod policy_helper_tests {
    use super::*;

    /// Every name a policy can choose is one the game takes: non-empty, within
    /// `MAX_PLAYER_NAME`, every character in the charmap.
    #[test]
    fn every_name_a_policy_can_choose_is_one_the_game_will_take() {
        let mut names: Vec<String> = RANDOM_NAMES.iter().map(|n| n.to_string()).collect();
        names.push("HUMAN".to_string());
        names.push(crate::pokemon::llm_policy::PLAYER_NAME.to_string());

        for name in names {
            assert!(!name.is_empty(), "a policy offered an empty name");
            assert!(
                name.len() <= crate::pokemon::MAX_PLAYER_NAME,
                "{name:?} is longer than the game's field"
            );
            assert_ne!(name, "NINTEN", "{name:?} is the new-game sentinel");
            let encoded = crate::pokemon::strings::PokemonString::from_string(&name).0;
            assert!(!encoded.contains(&0x00), "{name:?} has a character with no glyph");
        }
    }

    /// The Power Plant's numbered static-encounter sprites match their species.
    #[test]
    fn numbered_static_encounters_match_their_species() {
        assert!(sprite_is_species("Electrode 1", PokemonSpecies::Electrode));
        assert!(sprite_is_species("Electrode 2", PokemonSpecies::Electrode));
        assert!(sprite_is_species("Voltorb 6", PokemonSpecies::Voltorb));
        assert!(sprite_is_species("Moltres", PokemonSpecies::Moltres));
        assert!(sprite_is_species("Zapdos", PokemonSpecies::Zapdos));
        assert!(!sprite_is_species("Electrode 1", PokemonSpecies::Voltorb));
        assert!(!sprite_is_species("Rare Candy", PokemonSpecies::Electrode));
    }
}

#[cfg(test)]
mod heal_detour_tests {
    use super::*;
    use crate::pokemon::integration_tests::fixture::TestFixture;

    #[test]
    fn a_heal_detour_that_cannot_route_hands_back_to_the_route() {
        let mut fixture = TestFixture::new(
            include_bytes!("data/mt-moon.bin"), std::time::Duration::from_secs(1), vec![]);
        let state = fixture.game_state();
        let centre = Map::CinnabarPokecenter;
        assert_ne!(state.map.map, centre, "the fixture must be somewhere the detour has to travel to");

        // `enter` the current map pops on sight, so a shrinking queue proves the detour let go.
        let mut policy = DeterministicPolicy::new(42, vec![PolicyStep::enter(state.map.map)]);
        policy.last_pokemon_center = Some(centre);
        policy.heal_return = Some(centre);
        let graph = WorldGraph::new();

        for poll in 1..DeterministicPolicy::MAX_HEAL_ROUTE_WAIT {
            assert!(policy.pick_overworld_action(&state, &graph).is_none(), "poll {poll}");
            assert_eq!(policy.heal_return, Some(centre), "still trying at poll {poll}");
            assert_eq!(policy.steps_remaining(), Some(1), "the queue is untouched at poll {poll}");
        }

        policy.pick_overworld_action(&state, &graph);
        assert_eq!(policy.heal_return, None, "the detour let go");
        assert_eq!(policy.steps_remaining(), Some(0), "and the route is being played again");
    }

    /// The bound never fires on a detour that is moving.
    #[test]
    fn a_heal_detour_that_is_moving_is_never_abandoned() {
        let mut fixture = TestFixture::new(
            include_bytes!("data/back-in-cerulean.bin"), std::time::Duration::from_secs(1), vec![]);
        let state = fixture.game_state();
        let centre = Map::CeruleanPokecenter;
        assert_eq!(state.map.map, Map::CeruleanCity, "the fixture moved out from under this test");

        let mut policy = DeterministicPolicy::new(42, vec![PolicyStep::enter(state.map.map)]);
        policy.heal_return = Some(centre);
        // One observed node here, carrying the exits in sight, the Centre's door among them.
        let mut graph = WorldGraph::new();
        graph.observe(state.map.map, state.map.player_position, &state.map);

        // One poll short of the bound, so a counter that was not reset would give up on the next.
        policy.heal_route_stuck = DeterministicPolicy::MAX_HEAL_ROUTE_WAIT - 1;
        let action = policy.pick_overworld_action(&state, &graph)
            .expect("the centre is one warp away and the graph has been shown it");
        assert!(matches!(action.tile, MetaTile::Warp { to_map, .. } if to_map == centre),
            "the detour walks to the centre's door, not {:?}", action.tile);
        assert_eq!(policy.heal_route_stuck, 0, "a detour that moved was still being counted out");
        assert_eq!(policy.heal_return, Some(centre), "and it is still going");
        assert_eq!(policy.steps_remaining(), Some(1), "the main queue waits its turn");
    }

    /// Out of Route 14's pocket, the route on is scored across every crossing, not only the menu's.
    #[test]
    fn a_route_is_scored_across_every_crossing_and_not_only_the_nearest() {
        use crate::pokemon::world_graph::EdgeKind::Connection;
        let p = |x, y| Point8 { x, y };
        let mut fixture = TestFixture::new(include_bytes!("data/pocket-route14.bin"),
            std::time::Duration::from_secs(60), vec![PolicyStep::enter(Map::Route13)]);
        let state = fixture.run_until(|s| s.map.map == Map::Route13 && s.map.position_settled);
        let offered: Vec<MetaTile> = state.map.actions().into_iter().map(|a| a.tile)
            .filter(|tile| matches!(tile, MetaTile::Connection { to_map: Map::Route14, .. })).collect();
        let (pocket, main) = (p(19, 6), p(19, 8));
        assert_eq!(offered, [MetaTile::Connection { to_map: Map::Route14, to_position: pocket }],
                   "the fixture no longer lands beside the pocket's crossing");

        let mut graph = WorldGraph::new();
        graph.observe_edges(Map::Route14, pocket, &[(p(21, 6), Map::Route13, p(0, 6), Connection)]);
        graph.observe_edges(Map::Route14, main, &[(p(0, 44), Map::Route15, p(59, 8), Connection)]);
        graph.observe_edges(Map::Route13, p(0, 6), &[
            (p(0, 7), Map::Route14, pocket, Connection),
            (p(0, 9), Map::Route14, main, Connection),
        ]);
        let action = DeterministicPolicy::route_toward(&graph, &state.map, &state.map.actions(), Map::Route15)
            .expect("Route 15 is two crossings away");
        assert_eq!(action.tile, MetaTile::Connection { to_map: Map::Route14, to_position: main });
    }
}

#[cfg(test)]
mod scripted_progress_tests {
    use super::*;
    use crate::run::Scratch;

    fn route() -> Vec<PolicyStep> {
        vec![
            PolicyStep::enter(Map::PalletTown),
            PolicyStep::enter(Map::Route1),
            PolicyStep::enter(Map::ViridianCity),
            PolicyStep::enter(Map::Route2),
            PolicyStep::enter(Map::PewterCity),
        ]
    }

    /// A process starting this run from the beginning of the game — `Origin::Fresh`.
    fn policy_in(dir: &std::path::Path, steps: Vec<PolicyStep>) -> DeterministicPolicy {
        DeterministicPolicy::new(42, steps).resuming_in(dir, true)
    }

    /// A process resuming this run from a checkpoint — `Origin::Resumed`, the rollout case.
    fn resumed_policy_in(dir: &std::path::Path, steps: Vec<PolicyStep>) -> DeterministicPolicy {
        DeterministicPolicy::new(42, steps).resuming_in(dir, false)
    }

    #[test]
    fn a_restarted_process_resumes_the_route_where_it_left_off() {
        let scratch = Scratch::new("scripted-progress");

        let mut first = policy_in(&scratch.0, route());
        assert_eq!(first.steps_remaining(), Some(5));
        first.queue.drain(..3);
        first.record_progress();

        let second = resumed_policy_in(&scratch.0, route());
        assert_eq!(second.steps_remaining(), Some(2), "the route resumes at step 3 of 5");
        assert_eq!(second.queue.front(), Some(&PolicyStep::enter(Map::Route2)));
    }

    /// A run with no cursor is a new game and starts at the beginning.
    #[test]
    fn a_run_with_no_cursor_starts_at_the_beginning() {
        let scratch = Scratch::new("scripted-progress-fresh");
        let policy = policy_in(&scratch.0, route());
        assert_eq!(policy.steps_remaining(), Some(5));
        // And it records one immediately, so the *next* restart resumes rather than guessing.
        assert!(scratch.0.join(scripted_progress::FILE).exists(), "step 0 is still a cursor");
    }

    #[test]
    fn a_resumed_run_with_no_cursor_parks_rather_than_replaying_the_route() {
        let scratch = Scratch::new("scripted-progress-cursorless");
        let parked = resumed_policy_in(&scratch.0, route());
        assert_eq!(parked.steps_remaining(), Some(0), "parked");
        assert!(parked.is_exhausted(), "a parked policy stops answering, which is what parks the run");
    }

    /// A changed route parks the run; it must never replay from 0.
    #[test]
    fn a_route_that_changed_under_a_run_parks_it_rather_than_replaying_it() {
        let scratch = Scratch::new("scripted-progress-changed");

        let mut first = policy_in(&scratch.0, route());
        first.queue.drain(..3);
        first.record_progress();

        // Same length, different steps — so the length alone would have accepted this.
        let mut changed = route();
        changed[4] = PolicyStep::enter(Map::CeruleanCity);
        let parked = resumed_policy_in(&scratch.0, changed);
        assert_eq!(parked.steps_remaining(), Some(0), "parked");
        assert!(parked.is_exhausted(), "a parked policy stops answering, which is what parks the run");

        // And a route of a different length is caught too, by the cheaper half of the same check.
        let mut shorter = route();
        shorter.pop();
        assert_eq!(resumed_policy_in(&scratch.0, shorter).steps_remaining(), Some(0));
    }

    /// `POST /api/new-run` is the same desync from the other side.
    #[test]
    fn a_new_run_starts_the_route_again() {
        let scratch = Scratch::new("scripted-progress-restart");
        let next = Scratch::new("scripted-progress-restart-2");

        let mut policy = policy_in(&scratch.0, route());
        policy.queue.drain(..4);
        policy.record_progress();
        assert_eq!(policy.steps_remaining(), Some(1));

        policy.restart(Some(&next.0));
        assert_eq!(policy.steps_remaining(), Some(5), "the whole route is back");
        assert_eq!(policy.queue.front(), Some(&PolicyStep::enter(Map::PalletTown)));
        assert!(!scratch.0.join(scripted_progress::FILE).exists(), "the old run's cursor is not left behind");
        assert!(next.0.join(scripted_progress::FILE).exists(), "the new run records its own");
    }

    #[test]
    fn a_new_run_forgets_everything_the_old_one_learned() {
        let scratch = Scratch::new("scripted-progress-taint");
        let next = Scratch::new("scripted-progress-taint-2");

        let mut policy = policy_in(&scratch.0, route());
        policy.queue.drain(..2);
        policy.record_progress();
        policy.last_pokemon_center = Some(Map::MtMoonPokecenter);
        policy.heal_return = Some(Map::MtMoonPokecenter);
        policy.heal_route_stuck = 7;
        policy.gym_beaten.insert(Point8 { x: 4, y: 13 });
        policy.train_slot = Some(3);
        policy.collect_item_seen = true;

        policy.restart(Some(&next.0));

        assert_eq!(policy.heal_return, None, "a fresh game does not owe the old run a heal");
        assert_eq!(policy.heal_route_stuck, 0);
        assert_eq!(policy.last_pokemon_center, None);
        assert!(policy.gym_beaten.is_empty(), "a fresh save has beaten no gyms");
        assert_eq!(policy.train_slot, None);
        assert!(!policy.collect_item_seen);
        // And the queue, which is the half that was already right.
        assert_eq!(policy.steps_remaining(), Some(5));
    }

    /// Stable across processes, which is why it is not `DefaultHasher`.
    #[test]
    fn the_route_fingerprint_is_about_the_route_and_nothing_else() {
        assert_eq!(scripted_progress::fingerprint(&route()), scripted_progress::fingerprint(&route()));
        let mut different = route();
        different[0] = PolicyStep::enter(Map::ViridianCity);
        assert_ne!(scripted_progress::fingerprint(&route()), scripted_progress::fingerprint(&different));
        // The real route, so a fingerprint collapsed to a constant shows here.
        assert_ne!(
            scripted_progress::fingerprint(&PolicyStep::complete_game_steps()),
            scripted_progress::fingerprint(&route()),
        );
    }
}

#[cfg(test)]
mod random_policy_tests {
    use super::*;

    /// A warp on `map` leading to `to`, which is all [`RandomPolicy::action_key`] reads.
    fn warp(map: Map, to: Map, x: u8, y: u8) -> OverworldAction {
        OverworldAction {
            map,
            origin: Point8 { x: 0, y: 0 },
            destination: Point8 { x, y },
            tile: MetaTile::Warp { to_map: to, to_position: Point8 { x: 0, y: 0 } },
            route: vec![],
        }
    }

    /// Warps at the same coordinates on different maps do not share a weight.
    #[test]
    fn the_key_of_an_action_names_its_map() {
        let a = warp(Map::PalletTown, Map::OaksLab, 5, 6);
        let b = warp(Map::OaksLab, Map::PalletTown, 5, 6);
        assert_ne!(RandomPolicy::action_key(&a), RandomPolicy::action_key(&b));
    }

    /// Each repeat in the window compounds; an untaken action keeps its full weight.
    #[test]
    fn each_repeat_compounds_the_penalty() {
        let taken = warp(Map::PalletTown, Map::OaksLab, 5, 6);
        let fresh = warp(Map::PalletTown, Map::Route1, 9, 0);
        let mut policy = RandomPolicy::exploring(1);
        assert_eq!(policy.novelty_weight(&taken), 1.0, "nothing has been taken yet");
        for expected in [EXPLORE_DECAY, EXPLORE_DECAY.powi(2), EXPLORE_DECAY.powi(3)] {
            policy.remember(&taken);
            assert!((policy.novelty_weight(&taken) - expected).abs() < 1e-12);
        }
        assert_eq!(policy.novelty_weight(&fresh), 1.0, "an untaken action is never penalised");
    }

    /// A window, not a tally.
    #[test]
    fn the_window_forgets() {
        let first = warp(Map::PalletTown, Map::OaksLab, 5, 6);
        let mut policy = RandomPolicy::exploring(1);
        policy.remember(&first);
        assert_eq!(policy.novelty_weight(&first), EXPLORE_DECAY);
        for i in 0..EXPLORE_MEMORY {
            policy.remember(&warp(Map::Route1, Map::ViridianCity, i as u8, 0));
        }
        assert_eq!(policy.recent.len(), EXPLORE_MEMORY, "the window is bounded");
        assert_eq!(policy.novelty_weight(&first), 1.0, "the first choice has aged out");
    }

    /// The exploring walker takes all of a hub's exits sooner than the uniform draw.
    #[test]
    fn a_walker_covers_a_hub_faster_than_a_uniform_one() {
        let exits: Vec<OverworldAction> = [Map::ViridianCity, Map::PalletTown, Map::OaksLab, Map::Route2]
            .iter().enumerate().map(|(i, to)| warp(Map::Route1, *to, i as u8, 0)).collect();

        // Picks until all four exits have been taken at least once, averaged over 400 attempts.
        let cover = |explore: bool| -> f64 {
            let mut total = 0usize;
            for seed in 0..400u64 {
                let mut policy = if explore { RandomPolicy::exploring(seed) }
                                 else { RandomPolicy::seeded(seed) };
                let mut seen = std::collections::BTreeSet::new();
                let mut picks = 0usize;
                while seen.len() < exits.len() {
                    let weights: Vec<f64> = exits.iter().map(|a| policy.novelty_weight(a)).collect();
                    let rng = policy.rng.as_mut().expect("seeded");
                    let chosen = if explore {
                        RandomPolicy::choose_weighted(rng, exits.clone(), &weights)
                    } else {
                        exits.clone().into_iter().choose(rng)
                    }.expect("a non-empty menu");
                    if explore { policy.remember(&chosen) }
                    seen.insert(RandomPolicy::action_key(&chosen));
                    picks += 1;
                }
                total += picks;
            }
            total as f64 / 400.0
        };

        let (weighted, uniform) = (cover(true), cover(false));
        println!("picks to take all four exits: weighted {weighted:.2}, uniform {uniform:.2}");
        assert!(uniform > 7.5, "the uniform baseline should be near 8.3, was {uniform:.2}");
        assert!(weighted < uniform * 0.7,
                "weighted covered the hub in {weighted:.2} picks against uniform's {uniform:.2} — \
                 the recency bias is not doing anything");
    }

    /// An empty menu is `None`, and an all-zero-weight menu still answers.
    #[test]
    fn a_degenerate_menu_still_answers() {
        let mut policy = RandomPolicy::exploring(1);
        let rng = policy.rng.as_mut().expect("seeded");
        assert!(RandomPolicy::choose_weighted(rng, vec![], &[]).is_none());
        let menu = vec![warp(Map::Route1, Map::ViridianCity, 9, 0)];
        assert!(RandomPolicy::choose_weighted(rng, menu, &[0.0]).is_some(),
                "a total of zero falls back to a uniform draw rather than answering nothing");
    }
}

#[cfg(test)]
mod abandoned_catch_tests {
    use super::*;
    use crate::pokemon::integration_tests::fixture::TestFixture;

    #[test]
    fn a_species_the_route_gave_up_catching_is_not_waited_for() {
        let mut fixture = TestFixture::new(
            include_bytes!("data/back-in-cerulean.bin"), std::time::Duration::from_secs(1), vec![]);
        let state = fixture.game_state();

        // An item the fixture is actually carrying.
        let item = state.bag.iter().find(|i| i.quantity > 0)
            .expect("the fixture carries something").id;
        let missing = PokemonSpecies::Machop;
        assert!(!state.pokemon.iter().any(|p| p.species == missing),
            "the fixture moved under this test — it must not already hold a {missing}");
        let step = PolicyStep::TeachMove { item, target: PartyRef::Species(missing) };

        // Nothing has given up: the step waits, and goes on waiting.
        let mut waiting = DeterministicPolicy::new(42, vec![step.clone()]);
        for poll in 1..200 {
            assert!(waiting.pick_field_move(&state).is_none(), "poll {poll}");
            assert_eq!(waiting.steps_remaining(), Some(1),
                "the wait on a species that may still turn up is unbounded (poll {poll})");
        }

        // The catch gave up: the step must let go on the very next poll.
        let mut giving_up = DeterministicPolicy::new(42, vec![step]);
        giving_up.abandon_catch(missing, "the test says the balls ran out");
        assert!(giving_up.pick_field_move(&state).is_none());
        assert_eq!(giving_up.steps_remaining(), Some(0),
            "the step must skip a species the route already gave up catching");
    }

    /// A failed catch does not make a step aimed at a slot or another species give up.
    #[test]
    fn giving_up_on_one_species_does_not_skip_steps_aimed_at_anything_else() {
        let mut policy = DeterministicPolicy::new(0, Vec::<PolicyStep>::new());
        policy.abandon_catch(PokemonSpecies::Oddish, "the test says so");

        assert!(policy.target_was_abandoned(PartyRef::Species(PokemonSpecies::Oddish)));
        assert!(policy.target_was_abandoned(
            PartyRef::Line(&[PokemonSpecies::Bulbasaur, PokemonSpecies::Oddish])));
        assert!(!policy.target_was_abandoned(PartyRef::Species(PokemonSpecies::Machop)));
        assert!(!policy.target_was_abandoned(PartyRef::Line(&[PokemonSpecies::Machop])));
        assert!(!policy.target_was_abandoned(PartyRef::Slot(0)));
    }
}
