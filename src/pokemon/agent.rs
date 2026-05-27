use std::collections::VecDeque;
use std::time::Duration;
use std::rc::Rc;
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
use crate::pokemon::menu::BattleMenuState;
use crate::pokemon::policy::{Policy, RandomPolicy};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::text::PokemonTextReader;

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
    /// Quantity selector active (wItemQuantity > 0). Adjust then press A.
    ChoosingQuantity(BagItem),
    /// Yes/No confirmation visible. Press A on "Yes".
    ConfirmingPurchase,
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
    WanderingInGrass { tile_a: Point8, tile_b: Point8, heading_to_b: bool },

    Battle(BattleState),

    /// Navigating the Pokémart buy flow.
    PokemartShopping(PokemartState),
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

pub struct PokemonAgent {
    state: AgentState,
    backup_state: Option<AgentState>,
    event_buffer: VecDeque<AgentEvent>,
    cycles: MachineCycles,
    policy: Box<dyn Policy>,
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
        }
    }

    pub fn policy_exhausted(&self) -> bool {
        self.policy.is_exhausted()
    }

    /// Drains all buffered events and returns them.
    pub fn drain_events(&mut self) -> Vec<AgentEvent> {
        self.event_buffer.drain(..).collect()
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

    fn set_state(&mut self, state: AgentState) {
        if self.state != state {
            self.state = state;

            if self.state != AgentState::Idle {
                println!("{:?}", &self.state);
            }
        }
    }

    fn set_battle_state(&mut self, state: BattleState) {
        self.set_state(AgentState::Battle(state));
    }

    fn set_pokemart_state(&mut self, state: PokemartState) {
        self.set_state(AgentState::PokemartShopping(state));
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

        self.assert_naming_screen(game_mode, api)?;
        self.assert_script_state(game_mode);
        self.assert_battle_state(game_mode);
        self.assert_pokemart_state(game_mode, api)?;
        // Skip generic text-box handling while shopping — the mart state machine handles input.
        if !matches!(self.state, AgentState::PokemartShopping(_)) {
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
                        self.set_state(AgentState::Battle(BattleState::default()));
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
                } else if rollback_delay.tick(delta_cycles) {
                    api.release_all_buttons();
                    // script has just breached rollback deadline, commit to RunningScript so we can start mashing next cycle
                    if let Some(AgentState::OverworldMovement { destination, .. }) = self.backup_state.as_ref() {
                        self.event(AgentEvent::OverworldActionAborted {
                            destination: *destination,
                            reason: OverworldActionAbortedReason::Script,
                        });
                    }
                }
            }
            AgentState::AwaitingOverworldAction { ref mut delay } => {
                if delay.tick(delta_cycles) {
                    let game_state = api.game_state()?;
                    if let Some(action) = self.policy.pick_overworld_action(&game_state) {
                        self.take_overworld_action(action);
                    }
                }
            }
            AgentState::OverworldMovement { destination, map: expected_map } => {
                let game_state = api.game_state()?;
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
                } else if matches!(destination, MetaTile::Warp { .. }) && game_state.map.player_tile() == destination {
                    // Player is standing on the warp tile.  For edge warps (tile at y=0, y=max,
                    // x=0, or x=max) the connection only fires when the player presses the
                    // outward direction from the edge — not just by stepping on the tile.
                    // Pressing the outward direction here walks the player off the map and
                    // triggers the connection.  Interior warps (stairs, etc.) fire automatically
                    // when stepped on, so the map change is detected before this branch runs.
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
                            self.set_state(AgentState::WanderingInGrass { tile_a, tile_b, heading_to_b: true });
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
                        .find(|a| a.tile == destination);
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
                                Some(BattleMenuState::Fight) => {
                                    new_events.push(AgentEvent::text_box_from_reader(reader));

                                    api.release_all_buttons();
                                    self.set_battle_state(BattleState::AwaitingPolicy { delay: DelayContext::default() });
                                }
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
                            if let Some(menu_state) = api.menu_state().map(|s| s.battle_menu_state()).flatten() {
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
            AgentState::WanderingInGrass { tile_a, tile_b, ref mut heading_to_b } => {
                let game_state = api.game_state()?;
                if game_state.mode == GameMode::Overworld {
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
                    // The quantity selector is detected by wMaxItemQuantity == 99 (set by pokemart.asm).
                    PokemartState::ChoosingItem(item) => {
                        if api.mart_in_quantity_selector() {
                            self.set_pokemart_state(PokemartState::ChoosingQuantity(*item));
                            return Ok(());
                        }

                        if let Some(menu) = menu {
                            if menu.is_mart_item_list() {
                                let shop_items = api.mart_item_list();
                                let target_pos = shop_items.iter().position(|&id| id == item.id);
                                match target_pos {
                                    None => {
                                        // TODO emit event "no items to buy"
                                        self.set_pokemart_state(PokemartState::Quitting);
                                    }
                                    Some(target_idx) => {
                                        let current = menu.list_absolute_index() as usize;
                                        if current == target_idx {
                                            api.toggle_button(JoypadButton::A);
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

                    // Adjust quantity with Up/Down then confirm with A.
                    // Detect Yes/No menu by checking for is_yes_no_menu().
                    PokemartState::ChoosingQuantity(item) => {
                        if let Some(menu) = &menu {
                            if menu.is_yes_no_menu() {
                                self.set_pokemart_state(PokemartState::ConfirmingPurchase);
                                return Ok(());
                            }
                        }

                        // If wMaxItemQuantity is no longer 99 (quantity selector exited), fall back.
                        if !api.mart_in_quantity_selector() {
                            return Ok(());
                        }

                        let current_qty = api.mart_item_quantity();
                        if current_qty == 0 {
                            return Ok(());
                        }
                        let target = item.quantity;
                        if current_qty == target {
                            api.toggle_button(JoypadButton::A);
                        } else if current_qty < target {
                            api.toggle_button(JoypadButton::Up);
                        } else {
                            api.toggle_button(JoypadButton::Down);
                        }
                    }

                    // Select Yes on the Yes/No confirmation.
                    PokemartState::ConfirmingPurchase => {
                        if let Some(menu) = &menu {
                            if menu.is_yes_no_menu() {
                                if menu.current_item == 0 {
                                    api.toggle_button(JoypadButton::A);
                                } else {
                                    api.toggle_button(JoypadButton::Up);
                                }
                            } else if menu.is_mart_buy_sell_menu() {
                                // BuySellQuit menu returned — purchase was completed.
                                self.set_pokemart_state(PokemartState::Quitting);
                            } else if menu.is_mart_item_list() {
                                // Item list reappeared after purchase — press B to return to
                                // BuySellQuit rather than accidentally re-selecting an item.
                                api.toggle_button(JoypadButton::B);
                            } else {
                                // Post-purchase text ("OK! Here you are!") — mash A.
                                api.toggle_button(JoypadButton::A);
                            }
                        } else {
                            api.toggle_button(JoypadButton::A);
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
                                api.toggle_button(JoypadButton::A);
                            }
                        } else {
                            api.toggle_button(JoypadButton::A);
                        }
                    }
                }
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
