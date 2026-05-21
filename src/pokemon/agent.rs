use std::collections::VecDeque;
use std::time::Duration;
use crate::cycles::MachineCycles;
use crate::game_boy::GameBoy;
use crate::geometry::Point8;
use crate::joypad::JoypadButton;
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::{PokemonApi, PokemonApiTrait};
use crate::pokemon::delay::DelayContext;
use crate::pokemon::encoding::{GameMode, MetaTile};
use crate::pokemon::map::Map;
use crate::pokemon::menu::BattleMenuState;
use crate::pokemon::policy::{Policy, RandomPolicy};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::text::PokemonTextReader;

// too long and player veers off course on the overworld, too short and the game doesn't get chance to update values between turns
pub const AGENT_RESOLUTION: MachineCycles = MachineCycles::from_duration(Duration::from_millis(20));

pub struct PokemonAgent {
    state: AgentState,
    event_buffer: VecDeque<AgentEvent>,
    cycles: MachineCycles,
    policy: Box<dyn Policy>,
    /// Consecutive ticks where game_mode == Script while not yet in RunningScript.
    /// The commit threshold varies by agent state — see `assert_script_state`.
    script_debounce: u32,
}

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
    RunningScript,
    /// The Pokémon nickname entry screen is active.
    /// `decided` is false while waiting for the policy; once true the name has been
    /// written to the naming buffer and START is toggled each tick until the screen exits.
    NamingPokemon { species: PokemonSpecies, decided: bool },

    /// Player alternates between two adjacent grass tiles until a wild battle triggers.
    WanderingInGrass { tile_a: Point8, tile_b: Point8, heading_to_b: bool },

    Battle(BattleState),
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

impl Default for PokemonAgent {
    fn default() -> Self { Self::new(Box::new(RandomPolicy)) }
}

impl PokemonAgent {
    pub fn new(policy: Box<dyn Policy>) -> Self {
        Self {
            state: AgentState::default(),
            event_buffer: VecDeque::new(),
            cycles: MachineCycles::default(),
            policy,
            script_debounce: 0,
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

    /// If a map script triggers while navigating, abort and let RunningScript handle it.
    fn assert_script_state(&mut self, game_mode: GameMode) {
        if game_mode == GameMode::Script {
            if self.state != AgentState::RunningScript {
                // Threshold varies by state.
                //
                // During overworld navigation (OverworldMovement / WanderingInGrass) a
                // south-facing ledge jump causes a false Script positive: pokered calls
                // StartSimulatingJoypadStates (sets bit 7) for the 2-step jump while a
                // residual wScriptedNPCWalkCounter from the OaksLab NPC walks stays
                // non-zero — exactly the first Script condition.  A 2-step ledge jump
                // lasts 16 frames ≈ 13 agent ticks (at 20 ms per tick).  Requiring 20
                // consecutive Script ticks to commit from these states means the false
                // positive (~20 ticks, measured) never fires, while a genuine trainer/NPC freeze
                // script (which persists for hundreds of ticks) still commits.
                //
                // From any other state (Idle, AwaitingOverworldAction …) the script is
                // external / spontaneous — commit quickly (2 ticks) so Oak's dialog and
                // the "come with me" OaksLab guidance enter RunningScript promptly and
                // receive the A presses they need to advance.
                // A south-facing ledge jump fires Script for up to ~33 consecutive
                // agent ticks (measured empirically at 20 ms per tick), depending on
                // how the jump aligns with frame boundaries.  Using 40 gives a safe
                // margin so no ledge jump ever commits, while genuine NPC freeze
                // scripts (which persist for hundreds of ticks) still commit at most
                // 800 ms after the script starts.
                let threshold = match self.state {
                    AgentState::OverworldMovement { .. } | AgentState::WanderingInGrass { .. } => 40,
                    _ => 2,
                };
                self.script_debounce += 1;
                if self.script_debounce >= threshold {
                    if let AgentState::OverworldMovement { destination, .. } = self.state {
                        self.abort_overworld(destination, OverworldActionAbortedReason::Script);
                    }
                    self.set_state(AgentState::RunningScript);
                }
            }
        } else {
            self.script_debounce = 0;
            if self.state == AgentState::RunningScript {
                self.set_state(AgentState::Idle);
            }
        }
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

    pub fn update(&mut self, gb: &mut GameBoy, delta_cycles: MachineCycles) -> Result<(), String> {
        // ── Throttled decision-making ─────────────────────────────────────────────
        self.cycles += delta_cycles;
        if self.cycles < AGENT_RESOLUTION { return Ok(()); }

        let mut delta_cycles = MachineCycles::ZERO;
        while self.cycles >= AGENT_RESOLUTION {
            delta_cycles += AGENT_RESOLUTION;
            self.cycles -= AGENT_RESOLUTION;
        }

        let mut api = PokemonApi::new(gb);

        let game_mode = api.game_mode()
            .ok_or_else(|| "Not in game".to_string())?;

        self.assert_naming_screen(game_mode, &mut api)?;
        self.assert_script_state(game_mode);
        self.assert_battle_state(game_mode);
        self.assert_text_box_state(game_mode);

        let mut new_events: Vec<AgentEvent> = vec!();

        match self.state {
            AgentState::Idle => {
                api.release_all_buttons();
                match game_mode {
                    GameMode::TextBox => {
                        self.set_state(AgentState::ReadingTextBox { reader: PokemonTextReader::default() });
                    }
                    GameMode::Script => {
                        self.set_state(AgentState::RunningScript);
                    }
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
            AgentState::RunningScript => {
                api.toggle_button(JoypadButton::A);
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
                if game_state.mode == GameMode::Script {
                    // Script guard.
                } else if game_state.mode != GameMode::Overworld {
                    self.abort_overworld(destination, OverworldActionAbortedReason::from_game_mode(game_state.mode));
                } else if game_state.map.map != expected_map {
                    // Map changed — success for warps and connections (both take you off the map).
                    if matches!(destination, MetaTile::Warp(_) | MetaTile::Connection(_) | MetaTile::ConnectionWater(_)) {
                        new_events.push(AgentEvent::OverworldActionCompleted { destination });
                        self.set_state(AgentState::Idle);
                    } else {
                        self.abort_overworld(destination, OverworldActionAbortedReason::WrongMap(game_state.map.map));
                    }
                } else if game_state.map.player_tile() == destination && !matches!(destination, MetaTile::Warp(_)) {
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
                reader.update(&mut api);
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
                                        reader.update(&mut api);
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
            AgentState::NamingPokemon { species, decided } => {
                if decided {
                    // Buffer already written; keep pulsing START until DisplayNamingScreen
                    // exits (wFontLoaded → 0, so game_mode leaves TextBox/NamingScreen).
                    api.toggle_button(JoypadButton::Start);
                    if game_mode != GameMode::TextBox && game_mode != GameMode::NamingScreen {
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
