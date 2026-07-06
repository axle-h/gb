use std::collections::VecDeque;
use std::fmt::Display;
use std::time::Duration;
use crate::cycles::MachineCycles;
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::{PokemonApi, PokemonApiTrait};
use crate::pokemon::bag::BagItem;
use crate::pokemon::delay::DelayContext;
use crate::pokemon::encoding::GameMode;
use crate::pokemon::map::Map;
use crate::pokemon::tile::MetaTile;
use crate::pokemon::menu::{is_forget_move_prompt, BattleMenuState};
use crate::pokemon::policy::{Policy, RandomPolicy};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::text::PokemonTextReader;
use crate::pokemon::world_graph::WorldGraph;

// too long and player veers off course on the overworld, too short and the game doesn't get chance to update values between turns
pub const AGENT_RESOLUTION: MachineCycles = MachineCycles::from_duration(Duration::from_millis(20));

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OverworldActionAbortedReason {
    Unknown,
    Script,
    Battle,
    Textbox,
    NamingScreen,
    WrongMap(Map),
    NoAdjacentGrass,
    NoRoute(MetaTile),
}

impl OverworldActionAbortedReason {
    pub fn from_game_mode(game_mode: GameMode) -> Self {
        match game_mode {
            GameMode::Overworld => Self::Unknown,
            GameMode::TrainerBattle | GameMode::WildBattle => Self::Battle,
            GameMode::TextBox => Self::Textbox,
            GameMode::Script => Self::Script,
            GameMode::NamingScreen => Self::NamingScreen,
        }
    }
}

#[derive(Debug)]
pub enum AgentEvent {
    StartedOverworldAction { destination: MetaTile },
    OverworldActionAborted { destination: MetaTile, reason: OverworldActionAbortedReason },
    OverworldActionCompleted { destination: MetaTile },
    BattleStarted,
    BattleActionStarted { action: BattleAction },
    BattleEnded,
    TextBox { message: String }
}

impl AgentEvent {
    pub fn text_box_from_reader(reader: &PokemonTextReader) -> Self {
        Self::TextBox { message: reader.to_string() }
    }
}

impl Display for AgentEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentEvent::StartedOverworldAction { destination } =>
                write!(f, "→ {destination}"),
            AgentEvent::OverworldActionAborted { destination, reason } =>
                write!(f, "✗ {destination} ({reason:?})"),
            AgentEvent::OverworldActionCompleted { destination } =>
                write!(f, "✓ {destination}"),
            AgentEvent::BattleStarted =>
                write!(f, "battle started"),
            AgentEvent::BattleActionStarted { action } =>
                write!(f, "battle: {action:?}"),
            AgentEvent::BattleEnded =>
                write!(f, "battle ended"),
            AgentEvent::TextBox { message } =>
                write!(f, "📖 {message}"),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum BattleState {
    /// Waiting for the battle menu (TextBoxID 0x0B/0x1B) to appear.
    WaitingForMenu { reader: PokemonTextReader, delay: DelayContext },

    /// Battle menu is up but policy hasn't returned an action yet.
    AwaitingPolicy { delay: DelayContext },

    /// Navigating the menus
    Navigating { action: BattleAction, delay: DelayContext },
}

impl Default for BattleState {
    fn default() -> Self {
        Self::WaitingForMenu {
            reader: PokemonTextReader::message_box_only(),
            delay: DelayContext::default(),
        }
    }
}

/// State machine for navigating a Pokémart purchase sequence.
#[derive(Debug, Clone, Eq, PartialEq)]
enum PokemartState {
    /// Buy/Sell/Quit menu visible. Keep pressing A on "Buy" (position 0) until item list appears.
    ChoosingBuyOption(BagItem),
    /// Item list visible. Navigate to the target item.
    ChoosingItem(BagItem),
    /// A pressed on the target item; waiting for wMaxItemQuantity==99 (qty selector opening).
    /// Buttons are released here to avoid the qty selector auto-confirming a spurious A press.
    AwaitingQtySelector(BagItem),
    /// Quantity selector active (wItemQuantity > 0). Adjust then press A.
    /// `qty_last` / `stall_ticks` track consecutive ticks where wItemQuantity didn't change,
    /// detecting the post-confirm price-text box that appears before the Yes/No prompt.
    ChoosingQuantity { item: BagItem, qty_last: u8, stall_ticks: u32 },
    /// Yes/No confirmation visible. Press A on "Yes" if wItemQuantity matches the target,
    /// otherwise press B to cancel and retry from the item list.
    ConfirmingPurchase(BagItem),
    /// YES was selected — purchase is being processed.  Pulse B (via `ticks` flip-flop) to
    /// advance post-purchase text and close the item list that buyMenuLoop re-opens.
    PurchasedItem { ticks: u32 },
    /// All items bought. Navigate cursor to "Quit" and press A.
    Quitting,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
enum AgentState {
    #[default]
    Idle,
    /// Policy returned None for an overworld action — waiting out a delay then re-polling.
    AwaitingOverworldAction { delay: DelayContext },
    OverworldMovement { destination: MetaTile, map: Map },
    ReadingTextBox { reader: PokemonTextReader },
    /// A map script or NPC scripted walk is running.  The player is frozen; the agent
    /// toggles A each tick to advance the script and any subsequent dialogue.
    /// To avoid triggering this on ledge jumps, if the delay is not triggerred before the game
    /// state changes, the agent restores to its previous state instead.
    RunningScript { rollback_deadline: DelayContext },
    /// The Pokémon nickname entry screen is active.
    /// `decided` is false while waiting for the policy; once true the name has been
    /// written to the naming buffer and START is toggled each tick until the screen exits.
    NamingPokemon { species: PokemonSpecies, decided: bool },

    /// Player alternates between two adjacent grass tiles until a wild battle triggers.
    WanderingInGrass { map: Map, tile_a: Point8, tile_b: Point8, heading_to_b: bool },

    Battle(BattleState),

    /// Navigating the Pokémart buy flow.
    PokemartShopping(PokemartState),

    /// Teaching an HM/TM (a legitimately-obtained bag item) to a party member, via the real menus:
    /// START→ITEM→(bag, navigate to the HM)→USE→(party, choose the mon). The move-replace menu (if the
    /// mon knows 4 moves) is handled by the shared global forget-move handler. Uses the same plain
    /// press/release "mashing" + cursor-navigate pattern as `CuttingTree`. `entered_menu` tracks that
    /// we left the overworld (so a return to it ends the attempt).
    TeachingMove { item: crate::pokemon::item::ItemId, target_slot: u8, press: bool, entered_menu: bool, settle: u8 },

    /// Using the Cut field move on the tree the player is facing: driving START→POKéMON→mon→CUT (the
    /// game's `UsedCut` then removes the tree). `press` alternates each tick — plain press/release
    /// "mashing", navigating each menu's cursor to its target index and then confirming with A (never
    /// carrying a held direction into the next menu). `entered_menu` tracks that we left the overworld
    /// (so returning to it means the cut animation finished). `tree_pos` is the tile being cut.
    CuttingTree { press: bool, entered_menu: bool, tree_pos: Point8 },

    /// Checking a Vermilion Gym trash can for a hidden switch: route to a tile adjacent to `target`
    /// and face it (recomputed each tick via `MetaTileMap::route_to_face`), then press A to trigger
    /// `GymTrashScript`. `press` alternates for plain press/release mashing. `checked` becomes true
    /// once the check text/menu has appeared, so returning to the overworld means the can was checked.
    CheckingTrashCan { target: Point8, checked: bool, press: bool },

    /// Using an elevator floor panel: face the panel + A to open the floor list-menu, navigate the
    /// cursor (`wCurrentMenuItem`) to `floor` + A to pick it, then ride the (runtime-redirected)
    /// elevator warp out. `selected` flips once the floor is confirmed; `press` alternates for plain
    /// press/release mashing. Finishes when the map changes (we've left the elevator).
    UsingElevator { panel: Point8, floor: u8, selected: bool, press: bool },

    /// Using a bag item on the field: route to face the sprite at `target`, then drive
    /// START→ITEM→(bag, navigate to the item)→USE. The item's field effect takes over (e.g. the Poké
    /// Flute wakes a Snorlax → a battle, which the battle handler wins). `press` alternates for plain
    /// press/release mashing; `entered_menu` tracks that we left the overworld into the bag menus.
    UsingFieldItem { item: crate::pokemon::item::ItemId, target: Point8, press: bool, entered_menu: bool },
}

impl AgentState {
    pub fn battle_state_mut(&mut self) -> Result<&mut BattleState, String> {
        if let AgentState::Battle(s) = self {
            Ok(s)
        } else {
            Err("Not in battle".to_string())
        }
    }
}

impl Display for PokemartState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PokemartState::ChoosingBuyOption(i)   => write!(f, "mart:buy({:?}×{})", i.id, i.quantity),
            PokemartState::ChoosingItem(i)         => write!(f, "mart:item({:?}×{})", i.id, i.quantity),
            PokemartState::AwaitingQtySelector(i)  => write!(f, "mart:await({:?}×{})", i.id, i.quantity),
            PokemartState::ChoosingQuantity { item, .. } => write!(f, "mart:qty({:?}×{})", item.id, item.quantity),
            PokemartState::ConfirmingPurchase(i)   => write!(f, "mart:confirm({:?}×{})", i.id, i.quantity),
            PokemartState::PurchasedItem { .. }    => write!(f, "mart:purchased"),
            PokemartState::Quitting                => write!(f, "mart:quit"),
        }
    }
}

impl Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::Idle                          => write!(f, "idle"),
            AgentState::AwaitingOverworldAction { .. } => write!(f, "wait"),
            AgentState::OverworldMovement { destination, map } => write!(f, "move→{destination}@{map:?}"),
            AgentState::ReadingTextBox { .. }         => write!(f, "text"),
            AgentState::RunningScript { .. }          => write!(f, "script"),
            AgentState::NamingPokemon { species, .. } => write!(f, "name:{:?}", species),
            AgentState::WanderingInGrass { .. }       => write!(f, "wander"),
            AgentState::Battle(s) => match s {
                BattleState::WaitingForMenu { .. } => write!(f, "battle:wait"),
                BattleState::AwaitingPolicy { .. } => write!(f, "battle:policy"),
                BattleState::Navigating { action, .. } => write!(f, "battle:{:?}", action),
            },
            AgentState::PokemartShopping(s)           => write!(f, "{s}"),
            AgentState::TeachingMove { item, .. } => write!(f, "teach:{item:?}"),
            AgentState::CuttingTree { .. } => write!(f, "cut"),
            AgentState::CheckingTrashCan { target, .. } => write!(f, "trash→{target}"),
            AgentState::UsingElevator { floor, selected, .. } => write!(f, "elevator→floor {floor} (sel={selected})"),
            AgentState::UsingFieldItem { item, .. } => write!(f, "use-item:{item:?}"),
        }
    }
}

pub struct PokemonAgent {
    state: AgentState,
    backup_state: Option<AgentState>,
    event_buffer: VecDeque<AgentEvent>,
    cycles: MachineCycles,
    policy: Box<dyn Policy>,
    /// Map graph built **incrementally** as the player traverses. Each time the agent lands on a
    /// new map it records that section's live, sprite-resolved reachable warps/connections
    /// (`WorldGraph::observe`). Provided to the policy each overworld decision for backtracking
    /// (e.g. heal-return); forward travel is scripted with explicit `EnterMap` steps.
    world_graph: WorldGraph,
    /// The map the agent was last on, to detect map changes (warp/connection landings).
    last_map: Option<Map>,
    /// Trees the agent has cut down, by `(map, expanded tile position)`. The `MetaTileMap` is decoded
    /// from the static ROM map, so it always shows a `CutTree` there even after the game removed it;
    /// the agent remembers what it cut and treats those tiles as `Empty` for routing/movement.
    cut_tiles: std::collections::HashSet<(Map, Point8)>,
}

impl Default for PokemonAgent {
    fn default() -> Self { Self::new(Box::new(RandomPolicy)) }
}

impl PokemonAgent {
    pub fn new(policy: Box<dyn Policy>) -> Self {
        Self {
            state: AgentState::default(),
            backup_state: None,
            event_buffer: VecDeque::new(),
            cycles: MachineCycles::default(),
            policy,
            world_graph: WorldGraph::new(),
            last_map: None,
            cut_tiles: std::collections::HashSet::new(),
        }
    }

    /// The incrementally-built world graph (exposed for tests/inspection).
    pub fn world_graph(&self) -> &WorldGraph {
        &self.world_graph
    }

    pub fn policy_exhausted(&self) -> bool {
        self.policy.is_exhausted()
    }

    pub fn policy_steps_remaining(&self) -> Option<usize> {
        self.policy.steps_remaining()
    }

    pub fn policy_current_step_is_long_running(&self) -> bool {
        self.policy.current_step_is_long_running()
    }

    /// Drains all buffered events and returns them.
    pub fn drain_events(&mut self) -> Vec<AgentEvent> {
        self.event_buffer.drain(..).collect()
    }

    /// Human-readable description of the agent's current state (for debugging/tests).
    pub fn state_debug(&self) -> String {
        format!("{}", self.state)
    }

    fn event(&mut self, event: AgentEvent) {
        println!("{:?}", event);
        self.event_buffer.push_back(event);
        while self.event_buffer.len() > 100 {
            self.event_buffer.pop_front();
        }
    }

    fn backup_current_state(&mut self, new_state: AgentState) {
        self.backup_state = Some(self.state.clone());
        self.state = new_state;
    }

    fn restore_state_from_backup(&mut self) {
        if self.backup_state.is_none() {
            return;
        }
        self.state = self.backup_state.clone().unwrap();
        self.backup_state = None;
    }

    /// Read the game state, overriding tiles the agent has cut down to `Empty` (the ROM-decoded map
    /// still shows them as `CutTree`). Use this wherever the map is used for routing/movement.
    fn observe_state(&self, api: &PokemonApi) -> Result<crate::pokemon::GameState, String> {
        let mut state = api.game_state()?;
        for &(map, pos) in &self.cut_tiles {
            if state.map.map == map {
                let idx = pos.x as usize + pos.y as usize * state.map.width;
                if state.map.meta_tiles.get(idx) == Some(&MetaTile::CutTree) {
                    state.map.meta_tiles[idx] = MetaTile::Empty;
                }
            }
        }
        Ok(state)
    }

    fn set_state(&mut self, state: AgentState) {
        if self.state != state {
            self.state = state;

            if self.state != AgentState::Idle {
                println!("{}", &self.state);
            }
        }
    }

    fn set_battle_state(&mut self, state: BattleState) {
        self.set_state(AgentState::Battle(state));
    }

    fn set_pokemart_state(&mut self, state: PokemartState) {
        self.set_state(AgentState::PokemartShopping(state));
    }

    /// Drive the "Which move should be forgotten?" replace-move menu (appears on a level-up learn or
    /// when teaching an HM to a 4-move mon). `cursor_index` is the current cursor slot. Asks the
    /// policy which slot to forget (it keeps the strongest moves) and navigates there / presses A.
    fn drive_forget_menu(&mut self, api: &mut PokemonApi, cursor_index: u8) -> Result<(), String> {
        let game_state = api.game_state()?;
        let which = api.learning_pokemon_index();
        let current_moves: Vec<_> = game_state.pokemon.get(which)
            .map(|p| p.moves.iter().flatten().copied().collect())
            .unwrap_or_default();
        match api.move_to_learn() {
            Some(new_move) => match self.policy.pick_move_to_forget(&current_moves, new_move) {
                None => {} // still deciding — wait
                Some(Some(slot)) => {
                    let slot = slot as u8;
                    if cursor_index < slot { api.toggle_button(JoypadButton::Down); }
                    else if cursor_index > slot { api.toggle_button(JoypadButton::Up); }
                    else { api.toggle_button(JoypadButton::A); }
                }
                Some(None) => api.toggle_button(JoypadButton::B), // decline (best effort)
            },
            None => api.toggle_button(JoypadButton::A),
        }
        Ok(())
    }

    fn abort_overworld(&mut self, destination: MetaTile, reason: OverworldActionAbortedReason) {
        self.event(AgentEvent::OverworldActionAborted { destination, reason });
        self.set_state(AgentState::Idle);
    }

    pub fn take_overworld_action(&mut self, action: OverworldAction) {
        self.event(AgentEvent::StartedOverworldAction { destination: action.tile.clone() });
        self.set_state(AgentState::OverworldMovement { destination: action.tile, map: action.map });
    }

    /// Checks if a battle has just started or finished
    fn assert_battle_state(&mut self, game_mode: GameMode) {
        if matches!(game_mode, GameMode::WildBattle | GameMode::TrainerBattle) {
            match self.state {
                AgentState::Battle(_) => {}
                // The nickname screen after a catch runs while wIsInBattle is still 1, so
                // game_mode stays WildBattle even though we're already in the naming flow.
                AgentState::NamingPokemon { .. } => {}
                AgentState::OverworldMovement { destination, .. } => {
                    // entering battle from the overworld
                    let d = destination;
                    self.abort_overworld(d, OverworldActionAbortedReason::Battle);
                    self.event(AgentEvent::BattleStarted);
                    self.set_battle_state(BattleState::default());
                }
                _ => {
                    // entering battle from somewhere else, maybe a textbox
                    self.event(AgentEvent::BattleStarted);
                    self.set_battle_state(BattleState::default());
                }
            }
        } else if let AgentState::Battle(battle_state) = &self.state {
            // Leaving battle.
            if let BattleState::WaitingForMenu { reader, .. } = battle_state {
                // dump remaining text
                self.event(AgentEvent::text_box_from_reader(reader));
            }

            self.event(AgentEvent::BattleEnded);
            self.set_state(AgentState::Idle);
        }
    }

    /// Checks if the naming screen has just opened or closed
    fn assert_naming_screen(&mut self, game_mode: GameMode, api: &mut PokemonApi) -> Result<(), String> {
        if game_mode == GameMode::NamingScreen {
            if !matches!(self.state, AgentState::NamingPokemon { .. }) {
                // the naming screen has just opened
                let species = api.naming_screen_species()?;
                api.release_all_buttons();
                self.set_state(AgentState::NamingPokemon { species, decided: false });
            }
        } else if matches!(self.state, AgentState::NamingPokemon { decided: false, .. }) {
            // The naming screen closed before the policy reached a decision (unexpected).
            self.set_state(AgentState::Idle);
        }
        // If decided=true the strict NamingScreen detection no longer fires (buf0 is no
        // longer 0x50 after the name is written), so game_mode is TextBox.  The
        // NamingPokemon match branch handles its own exit once the font unloads.

        Ok(())
    }

    /// If a map script triggers, start a PendingScript countdown before committing to
    /// RunningScript.  If Script mode ends before the delay fires the agent restores
    /// its previous state (e.g. OverworldMovement for a ledge jump).
    fn assert_script_state(&mut self, game_mode: GameMode) {
        // Checking a trash can runs GymTrashScript (Script mode); the CheckingTrashCan state drives
        // and completes it itself, so don't hand it off to the RunningScript machinery.
        if matches!(self.state, AgentState::CheckingTrashCan { .. } | AgentState::UsingElevator { .. }) {
            return;
        }
        if game_mode == GameMode::Script {
            if !matches!(self.state, AgentState::RunningScript { .. }) {
                // A south-facing ledge jump fires Script for up to ~660 ms; use >800 ms when
                // navigating so no ledge jump ever commits.  From any other state (Idle,
                // AwaitingOverworldAction…) the script is external — commit in 40 ms.
                let rollback_delay = match self.state {
                    AgentState::OverworldMovement { .. } | AgentState::WanderingInGrass { .. } => DelayContext::long(),
                    _ => DelayContext::short(),
                };

                self.backup_current_state(AgentState::RunningScript { rollback_deadline: rollback_delay });
            }
        } else if let AgentState::RunningScript { rollback_deadline: rollback_delay } = self.state {
            if rollback_delay.is_exhausted() {
                // The script committed (ran long enough to be genuine).
                self.backup_state = None;
                self.set_state(AgentState::AwaitingOverworldAction {
                    delay: DelayContext::default(),
                });
            } else {
                // Script mode ended before the deadline — this was transient (e.g. a ledge
                // jump).  Restore the state the agent was in before the script fired.
                self.restore_state_from_backup();
            }
        }
    }

    /// Detects the Buy/Sell/Quit menu and transitions into PokemartShopping.
    /// Must run before assert_text_box_state so the mart flow takes priority.
    fn assert_pokemart_state(&mut self, game_mode: GameMode, api: &mut PokemonApi) -> Result<(), String> {
        if game_mode != GameMode::TextBox {
            // Mart interaction ended (returned to Overworld/Script after purchase).
            if matches!(self.state, AgentState::PokemartShopping(_)) {
                api.release_all_buttons();
                self.set_state(AgentState::Idle);
            }
            return Ok(());
        }

        // When the Buy/Sell/Quit menu appears for the first time, ask the policy what to buy.
        if let Some(menu) = api.menu_state() {
            if menu.is_mart_buy_sell_menu() && !matches!(self.state, AgentState::PokemartShopping(_)) {
                let game_state = api.game_state()?;
                if let Some(item) = self.policy.pick_mart_purchase(&game_state) {
                    api.release_all_buttons();

                    if let Some(item) = item {
                        self.set_state(AgentState::PokemartShopping(PokemartState::ChoosingBuyOption(item)));
                    } else {
                        self.set_state(AgentState::PokemartShopping(PokemartState::Quitting));
                    }
                }
            }
        }

        Ok(())
    }

    fn assert_text_box_state(&mut self, game_mode: GameMode) {
        // wFontLoaded=1 while the naming screen is active too; NamingPokemon{decided:true}
        // handles its own exit, so don't interfere.
        if matches!(self.state, AgentState::NamingPokemon { decided: true, .. }) {
            return;
        }
        if game_mode == GameMode::TextBox {
            if !matches!(self.state, AgentState::ReadingTextBox { .. }) {
                // text box opened
                if let AgentState::OverworldMovement { destination, .. } = self.state {
                    self.abort_overworld(destination, OverworldActionAbortedReason::Textbox);
                }
                let reader = if matches!(self.state, AgentState::Battle(_)) {
                    PokemonTextReader::message_box_only()
                } else {
                    PokemonTextReader::default()
                };
                self.set_state(AgentState::ReadingTextBox { reader });
            }
        } else if let AgentState::ReadingTextBox { reader } = &self.state {
            // text box closed
            self.event(AgentEvent::text_box_from_reader(&reader));
            self.set_state(AgentState::Idle);
        }
    }

    pub fn update(&mut self, api: &mut PokemonApi, delta_cycles: MachineCycles) -> Result<(), String> {
        // ── Throttled decision-making ─────────────────────────────────────────────
        self.cycles += delta_cycles;
        if self.cycles < AGENT_RESOLUTION { return Ok(()); }

        let mut delta_cycles = MachineCycles::ZERO;
        while self.cycles >= AGENT_RESOLUTION {
            delta_cycles += AGENT_RESOLUTION;
            self.cycles -= AGENT_RESOLUTION;
        }

        let game_mode = api.game_mode()
            .ok_or_else(|| "Not in game".to_string())?;

        // A trainer has engaged (line of sight) and the battle is initialising on its own.
        // The agent must neither hold a direction (which wedges the trainer's walk-up) nor
        // mash A (which keeps re-triggering the pre-battle interaction and prevents InitBattle
        // from running).  Release everything and simply wait: once wIsInBattle flips,
        // trainer_battle_pending() goes false and assert_battle_state takes over.
        //
        // Exception: a *script*-triggered trainer (e.g. the Mt Moon Super Nerd, engaged by stepping
        // on a coord trigger rather than by line of sight) displays its challenge text in a text box
        // with `wCurOpponent` already set, and the battle only starts once that text is advanced.
        // In that case do NOT suppress input — fall through so the text-box handler mashes A and the
        // battle begins. There is no walk-up to wedge because the trainer is stationary.
        if api.trainer_battle_pending() && game_mode != GameMode::TextBox {
            api.release_all_buttons();
            return Ok(());
        }

        // ── Global "Which move should be forgotten?" handler ──────────────────────
        // The replace-move menu appears both in battle (a level-up learn) and in the overworld
        // (teaching an HM to a 4-move mon), so drive it here — before any state-specific handling,
        // including during TeachingMove — whenever its unique (5,8) geometry AND on-screen prompt
        // text are both present.
        {
            // Detect the forget menu by its on-screen prompt (robust against stale menu geometry) and
            // drive it from the live cursor (`wCurrentMenuItem`). This covers both the battle level-up
            // menu (origin (5,8)) and the overworld HM-teach menu (origin (15,8)), which
            // `battle_menu_state()` would not classify because it keys on x==5.
            let forget_showing = api.menu_state().is_some()
                && api.on_screen_text(true).map_or(false, |t| is_forget_move_prompt(&t));
            if forget_showing {
                let cursor = api.menu_geometry().2;
                self.drive_forget_menu(api, cursor)?;
                return Ok(());
            }
        }

        self.assert_naming_screen(game_mode, api)?;
        self.assert_script_state(game_mode);
        self.assert_battle_state(game_mode);
        self.assert_pokemart_state(game_mode, api)?;
        // Skip generic text-box handling while shopping or teaching a move — those state machines
        // drive their own menu input.
        if !matches!(self.state, AgentState::PokemartShopping(_) | AgentState::TeachingMove { .. } | AgentState::CuttingTree { .. } | AgentState::CheckingTrashCan { .. } | AgentState::UsingElevator { .. } | AgentState::UsingFieldItem { .. }) {
            self.assert_text_box_state(game_mode);
        }

        let mut new_events: Vec<AgentEvent> = vec!();

        match self.state {
            AgentState::Idle => {
                api.release_all_buttons();
                match game_mode {
                    GameMode::TextBox => {
                        self.set_state(AgentState::ReadingTextBox { reader: PokemonTextReader::default() });
                    }
                    GameMode::Script => { /* assert_script_state will transition to PendingScript */ }
                    GameMode::NamingScreen => {
                        self.set_state(AgentState::NamingPokemon {
                            species: api.naming_screen_species()?,
                            decided: false,
                        });
                    }
                    GameMode::WildBattle | GameMode::TrainerBattle => {
                        self.set_battle_state(BattleState::default());
                    }
                    GameMode::Overworld => {
                        self.set_state(AgentState::AwaitingOverworldAction { delay: DelayContext::long() });
                    }
                }
            }
            AgentState::RunningScript { rollback_deadline: ref mut rollback_delay } => {
                if rollback_delay.is_exhausted() {
                    // script already breached rollback deadline, start mashing the A button
                    api.toggle_button(JoypadButton::A);
                } else {
                    // Still inside the rollback window (might be a transient ledge jump).
                    // Whatever the cause, release any movement button the agent was holding:
                    // a held direction during a trainer's scripted walk-up wedges the
                    // engagement so the battle never starts. A is not needed yet — genuine
                    // scripts are advanced by the A-mashing branch once the deadline passes.
                    let crossed = rollback_delay.tick(delta_cycles);
                    api.release_all_buttons();
                    if crossed {
                        // script has just breached rollback deadline, commit to RunningScript so we can start mashing next cycle
                        if let Some(AgentState::OverworldMovement { destination, .. }) = self.backup_state.as_ref() {
                            self.event(AgentEvent::OverworldActionAborted {
                                destination: *destination,
                                reason: OverworldActionAbortedReason::Script,
                            });
                        }
                    }
                }
            }
            AgentState::AwaitingOverworldAction { ref mut delay } => {
                if delay.tick(delta_cycles) {
                    let game_state = self.observe_state(api)?;
                    // Incrementally build the world graph: the first time we settle on a new map,
                    // record that section's live (sprite-resolved) reachable warps/connections,
                    // keyed by the raw landing coords (the space warp `to_position`s use). The
                    // player is stationary here so the raw coords are the landing position.
                    if self.last_map != Some(game_state.map.map) {
                        self.last_map = Some(game_state.map.map);
                        self.world_graph.observe(game_state.map.map, api.raw_player_coords(), &game_state.map);
                        // Cut trees regrow when you leave and re-enter a map, so a tree cut on a prior
                        // visit is standing again — drop the stale "already cut" memory or the agent
                        // will route through a regrown tree and jam (e.g. re-entering the Vermilion gym
                        // enclosure after Lt. Surge). `CuttingTree` cuts sequentially without changing
                        // maps, so this never fires mid-cut.
                        self.cut_tiles.clear();
                    }
                    // A non-walking field action (e.g. teach an HM) takes priority over walking.
                    match self.policy.pick_field_move(&game_state) {
                        Some(crate::pokemon::policy::FieldMove::TeachMove { item, target_slot }) => {
                            api.release_all_buttons();
                            self.set_state(AgentState::TeachingMove { item, target_slot, press: true, entered_menu: false, settle: 0 });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::CutTree) => {
                            api.release_all_buttons();
                            let tree_pos = game_state.map.tile_in_front().map(|(p, _)| p)
                                .unwrap_or(game_state.map.player_position);
                            self.set_state(AgentState::CuttingTree { press: true, entered_menu: false, tree_pos });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::CheckTrashCan { target }) => {
                            api.release_all_buttons();
                            self.set_state(AgentState::CheckingTrashCan { target, checked: false, press: true });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::UseElevator { panel, floor }) => {
                            api.release_all_buttons();
                            self.set_state(AgentState::UsingElevator { panel, floor, selected: false, press: true });
                            return Ok(());
                        }
                        Some(crate::pokemon::policy::FieldMove::UseFieldItem { item, target }) => {
                            api.release_all_buttons();
                            self.set_state(AgentState::UsingFieldItem { item, target, press: true, entered_menu: false });
                            return Ok(());
                        }
                        None => {}
                    }
                    if let Some(action) = self.policy.pick_overworld_action(&game_state, &self.world_graph) {
                        self.take_overworld_action(action);
                    }
                }
            }
            AgentState::OverworldMovement { destination, map: expected_map } => {
                let game_state = self.observe_state(api)?;
                if game_state.mode != GameMode::Overworld {
                    self.abort_overworld(destination, OverworldActionAbortedReason::from_game_mode(game_state.mode));
                } else if game_state.map.map != expected_map {
                    // Map changed — success for warps and connections (both take you off the map).
                    if matches!(destination, MetaTile::Warp { .. } | MetaTile::Connection { .. } | MetaTile::ConnectionWater(_)) {
                        new_events.push(AgentEvent::OverworldActionCompleted { destination });
                        self.set_state(AgentState::Idle);
                    } else {
                        self.abort_overworld(destination, OverworldActionAbortedReason::WrongMap(game_state.map.map));
                    }
                } else if matches!(destination, MetaTile::Warp { .. })
                    && game_state.map.player_tile() == destination
                    && is_on_map_border(&game_state.map)
                {
                    // Player is standing on an EDGE warp tile (at y=0, y=max, x=0, or x=max).
                    // These only fire when the player presses the outward direction off the map
                    // edge — not just by standing on the tile.  Press that direction.
                    //
                    // Interior warps are handled by the route-execution branch below: a player
                    // already on an interior warp gets a [step-off, step-on] route, and stepping
                    // back onto the warp tile fires it (CheckWarpsNoCollision). Pressing a fixed
                    // geometric direction here would jam the player against a wall instead.
                    let pos = game_state.map.player_position;
                    let h = game_state.map.height.saturating_sub(1) as u8;
                    let exit_dir = if pos.y == 0 { JoypadButton::Up }
                        else if pos.y == h { JoypadButton::Down }
                        else if pos.x == 0 { JoypadButton::Left }
                        else { JoypadButton::Right };
                    api.release_all_buttons();
                    api.press_button(exit_dir);
                } else if game_state.map.player_tile() == destination && !matches!(destination, MetaTile::Warp { .. }) {
                    if destination == MetaTile::Grass {
                        let tile_a = game_state.map.player_position;
                        let tile_b = adjacent_grass(&game_state.map, tile_a);
                        if let Some(tile_b) = tile_b {
                            self.set_state(AgentState::WanderingInGrass { map: game_state.map.map, tile_a, tile_b, heading_to_b: true });
                        } else {
                            // TODO this should not happen, we shouldn't generate an action if this is true
                            //      the adjacent grass tile should be in the action
                            self.abort_overworld(destination, OverworldActionAbortedReason::NoAdjacentGrass);
                            self.set_state(AgentState::Idle);
                        }
                    } else {
                        new_events.push(AgentEvent::OverworldActionCompleted { destination });
                        self.set_state(AgentState::Idle);
                    }
                } else {
                    let action = game_state.map.actions().into_iter()
                        .find(|a| a.tile == destination)
                        // A specific connection landing isn't in `actions()` (only the nearest crossing
                        // is) — re-derive its route each tick so the walk to it can still be tracked.
                        .or_else(|| match destination {
                            MetaTile::Connection { to_map, to_position } =>
                                game_state.map.connection_action(to_map, to_position),
                            _ => None,
                        });
                    match action {
                        None => self.abort_overworld(destination, OverworldActionAbortedReason::NoRoute(destination)),
                        Some(a) => match a.route.first() {
                            // Pulse A via toggle so hJoyPressed fires every other tick —
                            // press_button (after release_all) would only fire once since A
                            // stays held and hJoyPressed goes dark on the next frame.
                            Some(&JoypadButton::A) => api.toggle_button(JoypadButton::A),
                            // Hold direction buttons for continuous walking.
                            Some(&btn) => {
                                api.release_all_buttons();
                                api.press_button(btn);
                            }
                            None => {
                                new_events.push(AgentEvent::OverworldActionCompleted { destination });
                                self.set_state(AgentState::Idle);
                            }
                        }
                    }
                }
            }
            AgentState::ReadingTextBox { ref mut reader } => {
                reader.update(api);
            }
            AgentState::Battle(ref mut battle_state) => {
                match battle_state {
                    BattleState::WaitingForMenu { reader, delay } => {
                        if let Some(menu_state) = api.menu_state() {
                            match menu_state.battle_menu_state() {
                                // Main menu is ready: normal battle opens on FIGHT, Safari on BALL.
                                Some(BattleMenuState::Fight) | Some(BattleMenuState::SafariBall) => {
                                    new_events.push(AgentEvent::text_box_from_reader(reader));

                                    api.release_all_buttons();
                                    self.set_battle_state(BattleState::AwaitingPolicy { delay: DelayContext::default() });
                                }
                                Some(BattleMenuState::PokemonList { index }) => {
                                    // Party list is showing — either a forced switch (active
                                    // fainted) or a voluntary switch (Navigating placed cursor
                                    // here). If the cursor is already on an alive member, confirm
                                    // it; otherwise navigate to the first alive member (forced
                                    // switch case where cursor starts on the fainted slot).
                                    let game_state = api.game_state()?;
                                    let cursor_hp = game_state.pokemon
                                        .get(index as usize)
                                        .map_or(0, |p| p.current_hp);
                                    if cursor_hp > 0 {
                                        api.toggle_button(JoypadButton::A);
                                    } else {
                                        let target = game_state.pokemon.iter().enumerate()
                                            .find(|(_, p)| p.current_hp > 0)
                                            .map(|(i, _)| i as u8)
                                            .unwrap_or(0);
                                        if index < target {
                                            api.toggle_button(JoypadButton::Down);
                                        } else {
                                            api.toggle_button(JoypadButton::Up);
                                        }
                                    }
                                }
                                Some(BattleMenuState::MoveList { index }) => {
                                    // A move list is showing. Normally this is the move Navigating
                                    // highlighted, so confirm it with A. But if the highlighted move
                                    // is Disabled, confirming bounces back with "… is disabled!"
                                    // forever — press B to back out to the main menu so the policy
                                    // re-picks a usable move (the disabled slot is excluded from the
                                    // available moves).
                                    let disabled = api.game_state().ok()
                                        .and_then(|g| g.battle)
                                        .and_then(|b| b.player.disabled_move_slot)
                                        == Some(index);
                                    if disabled {
                                        api.toggle_button(JoypadButton::B);
                                    } else {
                                        api.toggle_button(JoypadButton::A);
                                    }
                                },
                                Some(_) => {
                                    // battle menu is showing, do not read the text
                                    api.toggle_button(JoypadButton::A);
                                },
                                None => {
                                    // something other than the battle menu is showing — wait for
                                    // the text box to render before reading it
                                    if delay.tick(delta_cycles) {
                                        reader.update(api);
                                    }
                                }
                            }
                        } else {
                            // no menu is showing, click mashing the A button
                            api.toggle_button(JoypadButton::A);
                        }
                    }

                    BattleState::AwaitingPolicy { delay } => {
                        if delay.tick(delta_cycles) {
                            let game_state = api.game_state()?;
                            if let Some(action) = self.policy.pick_battle_action(&game_state) {
                                new_events.push(AgentEvent::BattleActionStarted { action });
                                self.set_battle_state(BattleState::Navigating { action, delay: DelayContext::default() });
                            }
                        }
                    }

                    BattleState::Navigating { action, delay } => {
                        if delay.tick(delta_cycles) {
                            let Some(menu_state) = api.menu_state().and_then(|s| s.battle_menu_state()) else {
                                // No battle menu is recognized — the turn is resolving (result text
                                // or animation), e.g. the "… is fast asleep!" message shown after a
                                // sleeping Pokémon's move is committed. Hand back to WaitingForMenu,
                                // which advances the text (pressing A) and re-detects the menu.
                                // Staying here would hang forever: Navigating issues no input while
                                // no battle menu is on screen, which deadlocks a sleep-locked battle.
                                api.release_all_buttons();
                                self.set_battle_state(BattleState::default());
                                return Ok(());
                            };
                            {
                                let menu_target = BattleMenuState::from_action(*action);

                                if menu_state == menu_target {
                                    api.release_all_buttons();
                                    self.set_battle_state(BattleState::default());
                                } else {
                                    let resolved_target = if let Some(target_parent) = menu_target.parent() {
                                        if menu_state.parent() == Some(target_parent) {
                                            menu_target
                                        } else {
                                            target_parent
                                        }
                                    } else {
                                        menu_target
                                    };

                                    let target_location = resolved_target.location();
                                    let current_location = menu_state.location();


                                    let btn = if target_location == current_location {
                                        JoypadButton::A
                                    } else if target_location.x > current_location.x {
                                        JoypadButton::Right
                                    } else if target_location.x < current_location.x {
                                        JoypadButton::Left
                                    } else if target_location.y > current_location.y {
                                        JoypadButton::Down
                                    } else {
                                        JoypadButton::Up
                                    };

                                    api.toggle_button(btn);
                                }
                            }
                        }
                    }
                }
            }
            AgentState::WanderingInGrass { map, tile_a, tile_b, ref mut heading_to_b } => {
                let game_state = api.game_state()?;
                if game_state.mode == GameMode::Overworld {
                    if game_state.map.map != map {
                        // Blackout or other warp moved us off the grass map — let policy re-route.
                        self.set_state(AgentState::Idle);
                        return Ok(());
                    }
                    let pos = game_state.map.player_position;
                    let target = if *heading_to_b { tile_b } else { tile_a };
                    if pos == target {
                        *heading_to_b = !*heading_to_b;
                    }
                    let next = if *heading_to_b { tile_b } else { tile_a };
                    if let Some(dir) = dir_to(pos, next) {
                        api.release_all_buttons();
                        api.press_button(dir);
                    }
                }
            }
            AgentState::PokemartShopping(ref pokemart_state) => {
                let menu = api.menu_state();

                match pokemart_state {
                    // Keep navigating/pressing A on the Buy/Sell/Quit menu until the item list appears.
                    PokemartState::ChoosingBuyOption(item) => {
                        if let Some(menu) = menu {
                            if menu.is_mart_item_list() {
                                self.set_pokemart_state(PokemartState::ChoosingItem(*item));
                            } else if menu.is_mart_buy_sell_menu() {
                                if menu.current_item == 0 {
                                    api.toggle_button(JoypadButton::A);
                                } else {
                                    api.toggle_button(JoypadButton::Up);
                                }
                            } else {
                                // Some other text box (e.g., greeting) — mash A.
                                api.toggle_button(JoypadButton::A);
                            }
                        } else {
                            api.toggle_button(JoypadButton::A);
                        }
                    }

                    // Navigate item list to target item, then press A to select it.
                    PokemartState::ChoosingItem(item) => {
                        if let Some(menu) = menu {
                            if menu.is_mart_item_list() {
                                let shop_items = api.mart_item_list();
                                let target_pos = shop_items.iter().position(|&id| id == item.id);
                                let current = menu.list_absolute_index() as usize;
                                match target_pos {
                                    None => {
                                        self.set_pokemart_state(PokemartState::Quitting);
                                    }
                                    Some(target_idx) => {
                                        if current == target_idx {
                                            // Clear stale wMaxItemQuantity==99 so AwaitingQtySelector
                                            // can detect the fresh write from pokemart.asm reliably.
                                            api.write_max_item_quantity(0);
                                            api.toggle_button(JoypadButton::A);
                                            self.set_pokemart_state(PokemartState::AwaitingQtySelector(*item));
                                        } else if current < target_idx {
                                            api.toggle_button(JoypadButton::Down);
                                        } else {
                                            api.toggle_button(JoypadButton::Up);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // A was pressed on the target item; keep pressing A each tick until
                    // wMaxItemQuantity==99 (set by pokemart.asm right after item selection).
                    // DisplayListMenuID may still be initializing when the first A is sent, so we
                    // retry. As soon as we detect 99 we release all buttons in the same agent tick,
                    // so the qty selector's halt-based wait loop sees hJoyPressed=0 on its next
                    // VBlank (no rising edge while A is still held), preventing an auto-confirm.
                    PokemartState::AwaitingQtySelector(item) => {
                        if api.mart_in_quantity_selector() {
                            api.release_all_buttons();
                            self.set_pokemart_state(PokemartState::ChoosingQuantity { item: *item, qty_last: 0, stall_ticks: 0 });
                        } else {
                            api.toggle_button(JoypadButton::A);
                        }
                    }

                    // Adjust quantity with Up/Down then confirm with A.
                    // is_mart_item_list() is NOT used here because wTextBoxID stays ListMenuBox
                    // throughout the qty-selector phase; is_yes_no_menu() is the real signal.
                    PokemartState::ChoosingQuantity { item, qty_last, stall_ticks } => {
                        let item = *item;
                        let qty_last = *qty_last;
                        let stall_ticks = *stall_ticks;
                        // NLL: borrow of self.state via pokemart_state ends here (values copied)

                        let yes_no = menu.map_or(false, |m| m.is_yes_no_menu());
                        let cur_qty = api.mart_item_quantity();

                        if yes_no {
                            // Release all buttons before entering ConfirmingPurchase so the
                            // next gb.run has joypad=0, guaranteeing a fresh A rising edge
                            // for the YES confirmation (avoids a held-A false-no-edge).
                            api.release_all_buttons();
                            self.set_pokemart_state(PokemartState::ConfirmingPurchase(item));
                            return Ok(());
                        }

                        if cur_qty == 0 {
                            // Qty selector not yet initialized — wait.
                            return Ok(());
                        }
                        let target = item.quantity;

                        // Track consecutive ticks where wItemQuantity didn't change.
                        let (new_qty_last, new_stall_ticks) = if cur_qty == qty_last {
                            (qty_last, stall_ticks + 1)
                        } else {
                            (cur_qty, 0)
                        };
                        if let AgentState::PokemartShopping(PokemartState::ChoosingQuantity { qty_last: ref mut ql, stall_ticks: ref mut st, .. }) = self.state {
                            *ql = new_qty_last;
                            *st = new_stall_ticks;
                        }

                        // Stall AND at target → stuck in post-confirm price-text before Yes/No.
                        // Only mash A here; when qty != target we keep pressing Up/Down below.
                        if new_stall_ticks >= 8 && cur_qty == target {
                            api.toggle_button(JoypadButton::A);
                            return Ok(());
                        }

                        if cur_qty == target {
                            api.toggle_button(JoypadButton::A);
                        } else if cur_qty < target {
                            api.toggle_button(JoypadButton::Up);
                        } else {
                            api.toggle_button(JoypadButton::Down);
                        }
                    }

                    // Select Yes on the Yes/No confirmation, navigate to YES and press A.
                    // On YES confirmed, transition to PurchasedItem to exit the post-purchase
                    // item list that .buyMenuLoop re-opens.
                    // Wrong qty (edge case): press B to cancel back to the item list.
                    PokemartState::ConfirmingPurchase(item) => {
                        if menu.map_or(false, |m| m.is_mart_item_list()) {
                            // B-cancel from Yes/No (wrong qty) sent us back to the item list.
                            api.release_all_buttons();
                            self.set_pokemart_state(PokemartState::ChoosingItem(*item));
                            return Ok(());
                        }
                        if let Some(menu) = &menu {
                            if menu.is_yes_no_menu() {
                                let confirmed_qty = api.mart_item_quantity();
                                if confirmed_qty == item.quantity {
                                    if menu.current_item == 0 {
                                        api.release_all_buttons();
                                        api.toggle_button(JoypadButton::A);
                                        self.set_pokemart_state(PokemartState::PurchasedItem { ticks: 0 });
                                    } else {
                                        api.toggle_button(JoypadButton::Up);
                                    }
                                } else {
                                    api.toggle_button(JoypadButton::B);
                                }
                            } else if menu.is_mart_buy_sell_menu() {
                                self.set_pokemart_state(PokemartState::Quitting);
                            } else {
                                api.toggle_button(JoypadButton::A);
                            }
                        } else {
                            api.toggle_button(JoypadButton::A);
                        }
                    }

                    // Purchase was confirmed; advance post-purchase text with B, then cancel
                    // the item list that .buyMenuLoop re-opens. If the yes/no box is still
                    // visible (HandleMenuInput.Delay3 hasn't read A yet), toggle A so the
                    // rising edge is detected correctly on the next joypad poll.
                    PokemartState::PurchasedItem { ticks } => {
                        let ticks = *ticks;
                        // NLL: borrow of self.state via pokemart_state ends here (value copied)
                        if menu.map_or(false, |m| m.is_mart_buy_sell_menu()) {
                            api.release_all_buttons();
                            self.set_pokemart_state(PokemartState::Quitting);
                        } else if menu.map_or(false, |m| m.is_yes_no_menu()) {
                            // HandleMenuInput runs Delay3 (3 VBlanks ≈ 50ms) before reading
                            // joypad. Toggle A so hJoyLast[A] resets to 0 on one tick and
                            // produces a rising edge (hJoyPressed[A]=1 → YES) on the next.
                            api.toggle_button(JoypadButton::A);
                        } else {
                            // Post-purchase text or transitional state — pulse B to advance.
                            let new_ticks = ticks + 1;
                            if let AgentState::PokemartShopping(PokemartState::PurchasedItem { ticks: ref mut t }) = self.state {
                                *t = new_ticks;
                            }
                            api.release_all_buttons();
                            if new_ticks % 2 == 1 {
                                api.press_button(JoypadButton::B);
                            }
                        }
                    }

                    PokemartState::Quitting => {
                        if let Some(menu) = &menu {
                            if menu.is_mart_buy_sell_menu() {
                                match menu.current_item {
                                    2 => api.toggle_button(JoypadButton::A),
                                    n if n < 2 => api.toggle_button(JoypadButton::Down),
                                    _ => api.toggle_button(JoypadButton::Up),
                                }
                            } else {
                                // Still in post-purchase text or a transition menu — press B.
                                api.toggle_button(JoypadButton::B);
                            }
                        } else {
                            api.toggle_button(JoypadButton::B);
                        }
                    }
                }
            }
            AgentState::TeachingMove { item, target_slot, press, entered_menu, settle } => {
                use crate::pokemon::menu::TextBoxId;
                let gs = api.game_state()?;
                // Done once the mon knows the move (the forget/learn text has run and committed).
                let knows = crate::pokemon::policy::hm_move(item).map_or(false, |mv|
                    gs.pokemon.get(target_slot as usize)
                        .map_or(false, |p| p.moves.iter().flatten().any(|m| m.name == mv)));
                if knows {
                    // Move learned — but Gen 1 returns to the bag/"teach to another?" party menu after
                    // teaching, so back out with B until a STABLE overworld. `settle` counts consecutive
                    // overworld ticks; if a menu re-appears it resets and B-mashing resumes. Requiring a
                    // sustained overworld avoids finishing on a one-tick flicker between closing menus
                    // (which would hand a still-open menu to the generic A-mash handler and re-teach).
                    if game_mode != GameMode::Overworld {
                        api.release_all_buttons();
                        if press { api.press_button(JoypadButton::B); }
                        self.set_state(AgentState::TeachingMove { item, target_slot, press: !press, entered_menu, settle: 0 });
                        return Ok(());
                    }
                    if settle < 15 {
                        api.release_all_buttons();
                        self.set_state(AgentState::TeachingMove { item, target_slot, press, entered_menu, settle: settle + 1 });
                        return Ok(());
                    }
                    self.event(AgentEvent::TextBox { message: format!("Taught the HM to party slot {target_slot}") });
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                // Returned to the overworld without learning — this attempt fizzled (e.g. a mis-nav);
                // bail so `pick_field_move` re-issues TeachMove and we start the menu chain fresh.
                if entered_menu && game_mode == GameMode::Overworld {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                let entered_menu = entered_menu || game_mode != GameMode::Overworld;

                // Plain press/release "mashing" — a fresh rising edge every 2 ticks (see CuttingTree).
                if !press {
                    api.release_all_buttons();
                    self.set_state(AgentState::TeachingMove { item, target_slot, press: true, entered_menu, settle: 0 });
                    return Ok(());
                }

                let (top_x, top_y, current, scroll) = api.menu_geometry();
                let tbid = api.menu_state().map(|m| m.text_box_id);
                let nav = |cur: u8, target: u8| -> JoypadButton {
                    if cur < target { JoypadButton::Down }
                    else if cur > target { JoypadButton::Up }
                    else { JoypadButton::A }
                };
                // Drive each menu of the teach chain to its target index, then confirm with A.
                let button = if game_mode == GameMode::Overworld {
                    JoypadButton::Start // no menu yet → open START
                } else if top_x == 11 && top_y == 2 {
                    nav(current, 2) // START menu → ITEM (index 2, Pokédex obtained)
                } else if tbid == Some(TextBoxId::ListMenuBox) {
                    // Bag list → the HM's row (absolute index = cursor + scroll), then A to select it.
                    let target_idx = api.bag_item_position(item).unwrap_or(0);
                    nav(current + scroll, target_idx)
                } else if tbid == Some(TextBoxId::UseTossMenuTemplate) {
                    nav(current, 0) // USE/TOSS → USE (index 0)
                } else if tbid == Some(TextBoxId::TwoOptionMenu) {
                    nav(current, 0) // "Make room for CUT?" yes/no → YES (index 0)
                } else if tbid == Some(TextBoxId::MessageBox) && top_x == 0 && (top_y == 1 || top_y == 3) {
                    nav(current, target_slot) // party menu ("Teach to which POKéMON?") → the target mon
                } else {
                    JoypadButton::A // transitional text ("Booted up… / 1, 2, 3… and Poof!")
                };
                api.release_all_buttons();
                api.press_button(button);
                self.set_state(AgentState::TeachingMove { item, target_slot, press: false, entered_menu, settle: 0 });
            }
            AgentState::CuttingTree { press, entered_menu, tree_pos } => {
                use crate::pokemon::menu::TextBoxId;
                // A successful Cut opens the Pokémon menu, plays a fade/animation, then returns to the
                // overworld. So once we've entered a menu, the first return to the overworld means the
                // cut is done: record the tile (the ROM map still shows the tree) and finish. A failed
                // cut ("nothing to cut") stays in the menu, so it won't false-trigger this — and the
                // preconditions were verified before entering (facing a 0x3d tree, Cascade Badge).
                if entered_menu && game_mode == GameMode::Overworld {
                    self.cut_tiles.insert((api.game_state()?.map.map, tree_pos));
                    self.event(AgentEvent::TextBox { message: format!("Cut down the tree at {tree_pos}") });
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                let entered_menu = entered_menu || game_mode != GameMode::Overworld;

                // Plain press/release "mashing": press a button on a press tick, clear all on a
                // release tick. Each button therefore gives a fresh rising edge every 2 ticks, and a
                // held direction never carries into the next menu.
                if !press {
                    api.release_all_buttons();
                    self.set_state(AgentState::CuttingTree { press: true, entered_menu, tree_pos });
                    return Ok(());
                }

                let (top_x, top_y, current, _) = api.menu_geometry();
                let tbid = api.menu_state().map(|m| m.text_box_id);
                let nav = |cur: u8, target: u8| -> JoypadButton {
                    if cur < target { JoypadButton::Down }
                    else if cur > target { JoypadButton::Up }
                    else { JoypadButton::A }
                };
                // Navigate each menu's cursor to its target index, then confirm with A. Only navigate
                // when a menu is actually open — in the overworld a Down/Up would move the player off
                // the tree, so there we only open the menu with Start.
                let button = if game_mode == GameMode::Overworld {
                    JoypadButton::Start // facing the tree, no menu yet → open START
                } else if tbid == Some(TextBoxId::FieldMoveMonMenu) || top_y == 10 {
                    nav(current, 0) // field-move menu → CUT (index 0, the mon's only field move)
                } else if top_x == 11 && top_y == 2 {
                    nav(current, 1) // START menu → POKéMON (index 1, Pokédex obtained)
                } else if top_x == 0 && (top_y == 1 || top_y == 3) {
                    nav(current, 0) // party menu → the Cut mon (slot 0)
                } else {
                    JoypadButton::A // transitional text ("used CUT!")
                };
                api.release_all_buttons();
                api.press_button(button);
                self.set_state(AgentState::CuttingTree { press: false, entered_menu, tree_pos });
            }
            AgentState::CheckingTrashCan { target, checked, press } => {
                // Checking a can triggers GymTrashScript, which prints a text box (leaving the
                // overworld). Once that has appeared (`checked`), the return to the overworld means
                // the check has resolved — finish so `pick_field_move` re-evaluates (advancing to the
                // second can, or popping the step once both locks are open).
                if checked && game_mode == GameMode::Overworld {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                if game_mode != GameMode::Overworld {
                    // The check's text box / script is up — advance it by mashing A.
                    api.release_all_buttons();
                    if press { api.press_button(JoypadButton::A); }
                    self.set_state(AgentState::CheckingTrashCan { target, checked: true, press: !press });
                    return Ok(());
                }
                // In the overworld: route to a tile adjacent to the can and face it, then check it.
                let gs = self.observe_state(api)?;
                match gs.map.route_to_face(target).as_deref() {
                    Some([]) => {
                        // Adjacent and facing the can — mash A (clean press/release edges) to check it.
                        api.release_all_buttons();
                        if press { api.press_button(JoypadButton::A); }
                        self.set_state(AgentState::CheckingTrashCan { target, checked, press: !press });
                    }
                    Some(&[btn, ..]) => {
                        // Walk/turn toward the can; hold the direction for continuous movement.
                        api.release_all_buttons();
                        api.press_button(btn);
                        self.set_state(AgentState::CheckingTrashCan { target, checked, press: true });
                    }
                    _ => {
                        // No reachable tile adjacent to the can — abort so we don't spin forever.
                        self.event(AgentEvent::TextBox { message: format!("Can't reach trash can at {target}") });
                        api.release_all_buttons();
                        self.set_state(AgentState::Idle);
                    }
                }
            }
            AgentState::UsingElevator { panel, floor, selected, press } => {
                let gs = self.observe_state(api)?;
                // Rode the elevator out — the map is no longer an elevator room. Done. (Any of the
                // game's elevators — Rocket Hideout, Silph Co, Celadon Mart — share this handler.)
                let in_elevator = matches!(gs.map.map,
                    Map::RocketHideoutElevator | Map::SilphCoElevator | Map::CeladonMartElevator);
                if !in_elevator {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                // Floor list-menu is up: navigate the cursor (wCurrentMenuItem) to `floor`, then A.
                // The elevator floor menu is a `SPECIALLISTMENU` (wListMenuID == 0x04) whose
                // `wTextBoxID` reads `MessageBox` (the "Which floor?" print), NOT `ListMenuBox` — so
                // detect it via the list-menu ID. Only while not yet selected: once picked, the menu
                // vars linger and re-driving them would trap us instead of walking out to the warp.
                const SPECIAL_LIST_MENU: u8 = 0x04;
                if !selected && api.list_menu_id() == SPECIAL_LIST_MENU {
                    // The floor menu scrolls (Silph Co has 11 floors), so the cursor position within
                    // the visible window (`current_item`) caps out — compare the *absolute* index
                    // (`current_item + scroll_offset`) against the target floor.
                    let current = api.menu_state().map(|m| m.list_absolute_index()).unwrap_or(0);
                    api.release_all_buttons();
                    let mut selected = selected;
                    if press {
                        use std::cmp::Ordering;
                        match current.cmp(&floor) {
                            Ordering::Less    => api.press_button(JoypadButton::Down),
                            Ordering::Greater => api.press_button(JoypadButton::Up),
                            Ordering::Equal   => { api.press_button(JoypadButton::A); selected = true; }
                        }
                    }
                    self.set_state(AgentState::UsingElevator { panel, floor, selected, press: !press });
                    return Ok(());
                }
                // Non-overworld and not (yet) the floor list. Pulse A in both cases:
                //  - before selecting: the "Which floor?" message box precedes the SPECIALLISTMENU and
                //    waits for a button — pulse A to advance into the list.
                //  - after selecting: the pick's A-press can be swallowed if it lands mid-scroll, leaving
                //    the list menu open with the cursor already on the target floor; pulsing A re-confirms
                //    it (and is harmless during the subsequent shake / "arriving" text).
                if game_mode != GameMode::Overworld {
                    api.toggle_button(JoypadButton::A);
                    self.set_state(AgentState::UsingElevator { panel, floor, selected, press: true });
                    return Ok(());
                }
                if !selected {
                    // Face the panel and press A to open the floor menu.
                    match gs.map.route_to_face(panel).as_deref() {
                        Some([]) => {
                            api.release_all_buttons();
                            if press { api.press_button(JoypadButton::A); }
                            self.set_state(AgentState::UsingElevator { panel, floor, selected, press: !press });
                        }
                        Some(&[btn, ..]) => {
                            api.release_all_buttons();
                            api.press_button(btn);
                            self.set_state(AgentState::UsingElevator { panel, floor, selected, press: true });
                        }
                        _ => {
                            self.event(AgentEvent::TextBox { message: format!("Can't reach elevator panel at {panel}") });
                            api.release_all_buttons();
                            self.set_state(AgentState::Idle);
                        }
                    }
                } else {
                    // Floor picked and the menu redirected the exit warp — step onto it to ride out.
                    // (The static map still shows it leading back up, but stepping on fires the
                    // redirected warp; we finish on the map change at the top of this arm.)
                    let warp = gs.map.actions().into_iter()
                        .find(|a| matches!(a.tile, MetaTile::Warp { .. }));
                    match warp.as_ref().and_then(|a| a.route.first().copied()) {
                        Some(btn) => {
                            api.release_all_buttons();
                            api.press_button(btn);
                            self.set_state(AgentState::UsingElevator { panel, floor, selected, press: true });
                        }
                        None => {
                            self.event(AgentEvent::TextBox { message: "Can't reach the elevator exit warp".into() });
                            api.release_all_buttons();
                            self.set_state(AgentState::Idle);
                        }
                    }
                }
            }
            AgentState::UsingFieldItem { item, target, press, entered_menu } => {
                use crate::pokemon::menu::TextBoxId;
                // Back in the overworld after entering the bag menus → this attempt has resolved (the
                // item's effect ran and, for the Poké Flute, its battle was fought). Bail to Idle so the
                // policy re-checks: target gone → the step pops; still present → it re-issues.
                if entered_menu && game_mode == GameMode::Overworld {
                    api.release_all_buttons();
                    self.set_state(AgentState::Idle);
                    return Ok(());
                }
                // Still in the overworld: route to face the target sprite, then open the bag with START.
                if game_mode == GameMode::Overworld {
                    let gs = self.observe_state(api)?;
                    match gs.map.route_to_face(target).as_deref() {
                        Some([]) => {
                            api.release_all_buttons();
                            if press { api.press_button(JoypadButton::Start); }
                            self.set_state(AgentState::UsingFieldItem { item, target, press: !press, entered_menu });
                        }
                        Some(&[btn, ..]) => {
                            api.release_all_buttons();
                            api.press_button(btn);
                            self.set_state(AgentState::UsingFieldItem { item, target, press: true, entered_menu });
                        }
                        _ => {
                            self.event(AgentEvent::TextBox { message: format!("Can't reach the field-item target at {target}") });
                            api.release_all_buttons();
                            self.set_state(AgentState::Idle);
                        }
                    }
                    return Ok(());
                }
                // In the bag menus. Plain press/release mashing (fresh rising edge every 2 ticks).
                if !press {
                    api.release_all_buttons();
                    self.set_state(AgentState::UsingFieldItem { item, target, press: true, entered_menu: true });
                    return Ok(());
                }
                let (top_x, top_y, current, scroll) = api.menu_geometry();
                let tbid = api.menu_state().map(|m| m.text_box_id);
                let nav = |cur: u8, tgt: u8| -> JoypadButton {
                    if cur < tgt { JoypadButton::Down } else if cur > tgt { JoypadButton::Up } else { JoypadButton::A }
                };
                // Drive START → ITEM → (bag) item → USE. After USE the item's field effect starts a
                // battle (Snorlax); assert_battle_state then takes over, so here we just advance any
                // transitional / wake-up text with A.
                let button = if top_x == 11 && top_y == 2 {
                    nav(current, 2) // START menu → ITEM (index 2, Pokédex obtained)
                } else if tbid == Some(TextBoxId::ListMenuBox) {
                    let target_idx = api.bag_item_position(item).unwrap_or(0);
                    nav(current + scroll, target_idx) // bag list → the item's row, then A
                } else if tbid == Some(TextBoxId::UseTossMenuTemplate) {
                    nav(current, 0) // USE/TOSS → USE (index 0)
                } else {
                    JoypadButton::A // transitional / "woke up" text
                };
                api.release_all_buttons();
                api.press_button(button);
                self.set_state(AgentState::UsingFieldItem { item, target, press: false, entered_menu: true });
            }
            AgentState::NamingPokemon { species, decided } => {
                if decided {
                    // Buffer already written; keep pulsing START until DisplayNamingScreen
                    // exits (wFontLoaded → 0, so game_mode leaves TextBox/NamingScreen).
                    // Also stay while wIsInBattle is still 1 after a catch (game_mode stays
                    // WildBattle after the name is written but before EndOfBattle runs).
                    api.toggle_button(JoypadButton::Start);
                    let still_in_naming = matches!(
                        game_mode,
                        GameMode::TextBox | GameMode::NamingScreen
                            | GameMode::WildBattle | GameMode::TrainerBattle
                    );
                    if !still_in_naming {
                        api.release_all_buttons();
                        self.set_state(AgentState::Idle);
                    }
                } else if let Some(decision) = self.policy.pick_nickname(species) {
                    // Write the nickname directly into the naming screen's string buffer,
                    // bypassing character-grid navigation.  The screen copies this buffer
                    // when START is pressed.  An empty/None nickname causes AskName to
                    // fall back to the default species name.
                    api.write_naming_screen_buffer(decision.as_deref())?;
                    self.set_state(AgentState::NamingPokemon { species, decided: true });
                } else {
                    api.release_all_buttons();
                }
            }
        }

        for x in new_events.into_iter() {
            self.event(x);
        }

        Ok(())
    }

}

/// Returns the direction to step from `from` to an orthogonally adjacent `to`.
fn dir_to(from: Point8, to: Point8) -> Option<JoypadButton> {
    match (to.x as i16 - from.x as i16, to.y as i16 - from.y as i16) {
        ( 1,  0) => Some(JoypadButton::Right),
        (-1,  0) => Some(JoypadButton::Left),
        ( 0,  1) => Some(JoypadButton::Down),
        ( 0, -1) => Some(JoypadButton::Up),
        _        => None,
    }
}

/// Finds a grass tile orthogonally adjacent to `pos` in `map`, returning the first one found.
fn adjacent_grass(map: &crate::pokemon::tile_map::MetaTileMap, pos: Point8) -> Option<Point8> {
    let neighbors = [
        Point8 { x: pos.x,                  y: pos.y.saturating_sub(1) },
        Point8 { x: pos.x,                  y: pos.y.saturating_add(1) },
        Point8 { x: pos.x.saturating_sub(1),  y: pos.y                 },
        Point8 { x: pos.x.saturating_add(1),  y: pos.y                 },
    ];
    neighbors.into_iter().find(|&p| {
        (p.x as usize) < map.width
            && (p.y as usize) < map.height
            && map.meta_tiles[p.x as usize + p.y as usize * map.width] == MetaTile::Grass
    })
}

/// True if the player is on the outermost row/column of the (expanded) map — i.e. an edge
/// warp/connection tile that fires by stepping off the map edge rather than by stepping on.
fn is_on_map_border(map: &crate::pokemon::tile_map::MetaTileMap) -> bool {
    let pos = map.player_position;
    pos.x == 0
        || pos.y == 0
        || pos.x as usize == map.width.saturating_sub(1)
        || pos.y as usize == map.height.saturating_sub(1)
}
