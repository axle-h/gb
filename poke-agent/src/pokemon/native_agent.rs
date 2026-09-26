//! A policy's decisions carried out on the recreation, one [`Command`] at a time.
//!
//! [`PokemonAgent`](crate::pokemon::agent::PokemonAgent) infers from memory and the screen what is
//! on the table and presses buttons until the game has moved on. The recreation says what it is
//! waiting for and takes a typed command, so this loop only has to choose the command: the same
//! [`Policy`], the same [`GameState`] and the same rows, with the pressing left to `pokered`.

use gb::joypad::JoypadButton;
use poke_core::geometry::Point8;
use pokered::command::{Command, Decision, Refusal, Reply};
use pokered::mode::{Mode, Status};
use pokered::modes::field_move_menu::FieldMoveChoice;
use pokered::modes::pc::bills_pc::BillsPcMenu;
use pokered::modes::start_menu::StartMenuEntry;
use pokered::systems::overworld::location::Direction;
use pokered::{Event, Game, Input};

use crate::pokemon::agent::{is_on_map_border, step_pos, AgentEvent, OverworldActionAbortedReason};
use crate::pokemon::battle::BattleAction;
use crate::pokemon::encoding::GameMode;
use crate::pokemon::map::Map;
use crate::pokemon::native::NativeGame;
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::bag::BagItem;
use crate::pokemon::item::ItemId;
use crate::pokemon::policy::{field_move_at, field_move_carrier, FieldMove, Policy};
use crate::pokemon::map_metadata::PlayerFacingDirection;
use crate::pokemon::postgame::item_storage::PcItemOp;
use crate::pokemon::postgame::items::{Effect, UseTarget};
use crate::pokemon::postgame::pc_box::PcBoxOp;
use crate::pokemon::postgame::fishing::Rod;
use crate::pokemon::postgame::game_corner::Prize;
use crate::pokemon::postgame::gifts::PartyScript;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::tile::{HiddenObject, MetaTile};
use crate::pokemon::world_graph::WorldGraph;
use crate::pokemon::GameState;

/// Decision points a row may go without a route before the walk is given up: a person standing in
/// the only doorway moves on in a few of their own steps.
const MAX_ROUTE_LOST_POLLS: u32 = 120;

/// A row being walked: re-derived at every decision point, as the emulated agent re-derives it
/// every tick, so nothing depends on a route's tail.
#[derive(Debug, Clone, Copy)]
struct Walk {
    destination: MetaTile,
    map: Map,
    /// The last command was A at the row's end, so a text box is the row succeeding.
    pressed_a: bool,
    route_lost: u32,
}

/// Menus the agent walks itself. A walk it was started from stays in [`NativeAgent::walk`], which is
/// not judged while a task holds the screen.
#[derive(Debug, Clone, Copy)]
enum Task {
    FieldMove(FieldMoveUse),
    Push(Push),
    Bag(BagUse),
    Pc(PcUse),
    Mart(MartVisit),
    Talk(TalkUse),
    Switch(SwitchUse),
    Pace(Pace),
}

/// Something faced and pressed A at, whose script then asks: a trash can, a vending machine, an
/// elevator's panel, a prize vendor, the Day Care, the Name Rater, a trade. Its one menu is
/// answered once and any after it backed out of; every yes/no is YES.
#[derive(Debug, Clone, Copy)]
struct TalkUse {
    at: Point8,
    facing: Option<PlayerFacingDirection>,
    what: Talk,
    opened: bool,
    answered: bool,
    before: TalkBefore,
}

#[derive(Debug, Clone, Copy)]
enum Talk {
    /// A trash can, or a vending machine, whose menu opens on its cheapest drink.
    Press,
    Elevator { floor: u8 },
    Prize(Prize),
    Party { script: PartyScript, slot: u8 },
}

#[derive(Debug, Clone, Copy)]
struct TalkBefore {
    coins: u32,
    party: usize,
    nick: Option<[u8; 11]>,
}

/// SWITCH on the party menu, to bring a mon to the front.
#[derive(Debug, Clone, Copy)]
struct SwitchUse {
    slot: u8,
    opened: bool,
    /// The mon was chosen, then SWITCH, then the front slot: every party menu after is the way out.
    stage: u8,
}

/// A clerk's BUY/SELL/QUIT, from the first time it is up until the goodbye. Buying is asked of the
/// policy there, as on the emulated side; a sale is walked to the clerk first.
#[derive(Debug, Clone, Copy)]
struct MartVisit {
    /// The square to face and talk across, for a sale: the counter, as `pick_sale` names it.
    clerk: Option<(Point8, PlayerFacingDirection)>,
    opened: bool,
    /// `pick_mart_purchase` has answered for this visit.
    asked: bool,
    want: Option<MartWant>,
    /// BUY or SELL was chosen for `want`.
    chosen: bool,
    /// The want's row was taken off its list, so the list coming back is the way out.
    picked: bool,
    /// The bag's count of the want's item as it began.
    before: u8,
    quitting: bool,
}

#[derive(Debug, Clone, Copy)]
enum MartWant {
    Buy(BagItem),
    Sell(BagItem),
}

/// A Pokémon Center's PC, or the one in the player's room: turned on facing it, the PC wanted, the
/// job, then every menu backed out of, LOG OFF and SEE YA! being rows rather than B.
#[derive(Debug, Clone, Copy)]
struct PcUse {
    at: Point8,
    job: PcJob,
    opened: bool,
    /// The player's or Bill's PC was chosen off the Pokémon Center's menu.
    entered: bool,
    /// The job's row was chosen on the player's or Bill's menu.
    started: bool,
    /// The job's item or mon was chosen off its list.
    picked: bool,
    before: PcBefore,
}

#[derive(Debug, Clone, Copy)]
enum PcJob {
    Items { op: PcItemOp, item: ItemId, quantity: u8 },
    Box(PcBoxOp),
}

#[derive(Debug, Clone, Copy)]
struct PcBefore {
    /// The item on the side it leaves.
    quantity: u8,
    party: u8,
    boxed: u8,
}

/// START, ITEM, the item, USE or TOSS, and whatever the use asks after: a mon, a move, a count, a
/// yes, a move to forget. The bag coming back is where every use but a few ends, so it is backed
/// out of, and what changed in the world says whether the use took.
#[derive(Debug, Clone, Copy)]
struct BagUse {
    item: ItemId,
    how: BagHow,
    /// Someone to turn to first, for an item used on them.
    face: Option<Point8>,
    opened: bool,
    /// The item's row was taken, so the list coming back is the way out.
    picked: bool,
    party: bool,
    moves: bool,
    /// The bag came back after the item was picked, which a cast does only when refused.
    back: bool,
    before: Before,
}

#[derive(Debug, Clone, Copy)]
enum BagHow {
    Use(UseTarget),
    Toss,
    Teach { slot: u8 },
    Evolve { slot: u8 },
    /// A rod at the water faced; a bite is a battle, which ends the task where it starts.
    Cast { rod: Rod, row: bool },
}

/// What a use is judged against.
#[derive(Debug, Clone, Copy)]
struct Before {
    quantity: u8,
    species: Option<PokemonSpecies>,
    moves: [Option<PokemonMoveName>; 4],
    walk_bike_surf: u8,
}

/// START, POKéMON, the mon, the move, and for FLY the town.
#[derive(Debug, Clone, Copy)]
struct FieldMoveUse {
    slot: u8,
    name: PokemonMoveName,
    /// FLY's town.
    to: Option<Map>,
    /// SURF's water, which the player turns to before the menu is opened.
    face: Option<Direction>,
    /// CUT's tree, or SURF's water.
    at: Option<Point8>,
    opened: bool,
    /// The move's row was taken, so the party menu coming back is the game refusing it.
    chosen: bool,
    refused: bool,
    then: Then,
}

#[derive(Debug, Clone, Copy)]
enum Then {
    /// Say what happened; a walk being ridden, as SURF's mount is, goes on by itself.
    Report,
    /// The walk's row was this move, so the move finishes it.
    Complete,
    /// A shove that needed Strength first.
    Push(Push),
}

/// One shove of the boulder at `boulder`: walk behind it, arm Strength if it is not, push.
#[derive(Debug, Clone, Copy)]
struct Push {
    boulder: Point8,
    dir: JoypadButton,
    /// Strength has been asked for once already.
    armed: bool,
    /// One shove of a [`BoulderGoal`], which re-plans when it lands.
    goal: bool,
}

/// A boulder to be shoved onto a switch or down a hole, re-planned after every shove because the
/// floor moves under a stored plan. The plan's length is the progress measure.
#[derive(Debug, Clone, Copy)]
struct BoulderGoal {
    which: Point8,
    target: Point8,
    hole: bool,
    pushes: u8,
    best: usize,
    /// Shoves since the plan last got shorter.
    stale: u8,
}

/// `PacingForEncounters`: back and forth between two squares until something appears.
#[derive(Debug, Clone, Copy)]
struct Pace {
    destination: MetaTile,
    map: Map,
    a: Point8,
    b: Point8,
    heading_to_b: bool,
    /// Refused steps in a row, which a pair the map has moved under earns.
    stalled: u8,
    /// The frame the budget runs out on.
    until: u64,
}

pub struct NativeAgent {
    native: NativeGame,
    policy: Box<dyn Policy>,
    graph: WorldGraph,
    last_map: Option<Map>,
    walk: Option<Walk>,
    task: Option<Task>,
    /// The row being walked is a quiz answer of NO, so the next yes/no is answered NO.
    answer_no: bool,
    /// The row being walked is a vending machine's drink, by its row in the machine's menu.
    vending_pick: Option<u8>,
    /// The boulder goal being worked, between its shoves.
    boulder_goal: Option<BoulderGoal>,
    /// Frames ticked, which is game time under any pacing.
    frames: u64,
    /// `pick_move_to_forget`'s answer, held while its `LearnMove` is up.
    forget: Option<Option<usize>>,
    /// Trees cut on this visit, which the cartridge's own tables still draw. A battle reloads the
    /// map and they grow back, as they do on leaving it.
    cut_trees: Vec<Point8>,
    /// Accepted and not yet done or interrupted.
    running: Option<Command>,
    in_battle: bool,
}

impl NativeAgent {
    pub fn new(game: Game, policy: Box<dyn Policy>) -> Result<Self, String> {
        Ok(Self {
            native: NativeGame::new(game)?,
            policy,
            graph: WorldGraph::new(),
            last_map: None,
            walk: None,
            task: None,
            cut_trees: Vec::new(),
            forget: None,
            answer_no: false,
            vending_pick: None,
            boulder_goal: None,
            frames: 0,
            running: None,
            in_battle: false,
        })
    }

    pub fn game(&self) -> &Game {
        self.native.game()
    }

    /// The game state with the trees cut on this visit cleared.
    pub fn game_state(&self) -> Result<GameState, String> {
        let mut state = self.native.game_state()?;
        for &at in &self.cut_trees {
            let index = at.x as usize + at.y as usize * state.map.width;
            if state.map.meta_tiles.get(index) == Some(&MetaTile::CutTree) {
                state.map.meta_tiles[index] = MetaTile::Empty;
            }
        }
        Ok(state)
    }

    pub fn policy(&self) -> &dyn Policy {
        self.policy.as_ref()
    }

    /// One frame: the command in flight carried on, or the next one chosen and handed in.
    pub fn tick(&mut self) -> Result<(), String> {
        self.frames += 1;
        if self.running.is_some() {
            let frame = self.native.game_mut().frame(Input::None);
            self.finish(&frame.events);
            return Ok(());
        }
        self.notice_battle_end();
        if self.learning().is_none() {
            self.forget = None;
        }
        let status = self.native.game().status();
        if self.task.is_some() {
            if !self.battle_is_up() {
                let command = self.task_step(status)?;
                return self.issue_or_wait(command);
            }
            // A battle out of a step behind a boulder, or the step onto the water. A boulder goal
            // goes too: the battle may move the player off the floor.
            self.task = None;
            self.boulder_goal = None;
        }
        if self.walk.is_some() && status != Status::Waiting(Decision::Overworld) {
            self.end_walk_off_the_map()?;
        }
        let command = match status {
            Status::Busy | Status::Idle => None,
            Status::Waiting(Decision::Text) => Some(Command::Advance),
            Status::Waiting(Decision::TwoOption | Decision::ForgetMove) if self.learning().is_some() => self.learn_answer(&status)?,
            Status::Waiting(Decision::TwoOption) => Some(Command::ChooseOption(std::mem::take(&mut self.answer_no) as u8)),
            Status::Waiting(Decision::NamingScreen) => self.name_answer()?,
            Status::Waiting(Decision::CursorMenu) => Some(match self.vending_pick.take() {
                Some(row) => Command::ChooseOption(row),
                None => Command::CancelOption,
            }),
            // A menu the agent did not open is closed, not confirmed.
            Status::Waiting(Decision::List) => Some(Command::CancelList),
            Status::Waiting(Decision::StartMenu) => Some(Command::CloseStartMenu),
            Status::Waiting(Decision::Options) => Some(Command::CloseOptions),
            Status::Waiting(Decision::PartyMenu | Decision::UseToss | Decision::MoveMenu | Decision::TownMap
                            | Decision::FlyDestination) if !self.battle_is_up() => Some(Command::CancelOption),
            Status::Waiting(Decision::TrainerCard | Decision::StatusScreen) => Some(Command::Advance),
            Status::Waiting(Decision::Pokedex | Decision::PokedexSideMenu | Decision::PokedexData) => Some(Command::CloseDex),
            // A clerk the policy walked up to.
            Status::Waiting(Decision::BuySellQuit) => {
                self.task = Some(Task::Mart(MartVisit {
                    clerk: None, opened: true, asked: false, want: None, chosen: false, picked: false, before: 0, quitting: false,
                }));
                self.task_step(status)?
            }
            Status::Waiting(Decision::Overworld) => self.overworld()?,
            Status::Waiting(Decision::BattleMenu | Decision::BattleMoves) => self.battle()?,
            Status::Waiting(Decision::PartyMenu) if self.battle_is_up() => self.battle()?,
            Status::Waiting(decision) => return Err(format!("no native answer yet for {decision:?}")),
        };
        self.issue_or_wait(command)
    }

    fn issue_or_wait(&mut self, command: Option<Command>) -> Result<(), String> {
        match command {
            None => {
                self.native.game_mut().frame(Input::None);
                Ok(())
            }
            Some(command) => self.issue(command),
        }
    }

    fn issue(&mut self, command: Command) -> Result<(), String> {
        let frame = self.native.game_mut().frame(Input::Command(command.clone()));
        match frame.reply {
            Some(Reply::Accepted) => {
                if let Some(walk) = self.walk.as_mut() {
                    walk.pressed_a = command == Command::Interact;
                    walk.route_lost = 0;
                }
                self.running = Some(command);
                self.finish(&frame.events);
                Ok(())
            }
            // Someone stepped into the way between the route and the step: ask again next frame.
            Some(Reply::Refused(Refusal::Invalid(reason))) if matches!(command, Command::Step(_)) && self.walk.is_some() => {
                self.lose_route(&reason)
            }
            Some(Reply::Refused(Refusal::Busy)) => Ok(()),
            other => Err(format!("{command:?} was not taken: {other:?}")),
        }
    }

    fn finish(&mut self, events: &[Event]) {
        for event in events {
            match event {
                Event::CommandDone(done) if Some(done) == self.running.as_ref() => self.running = None,
                Event::CommandInterrupted { command, .. } if Some(command) == self.running.as_ref() => self.running = None,
                _ => {}
            }
        }
    }

    fn event(&mut self, event: AgentEvent) {
        self.policy.on_event(&event);
    }

    fn battle_is_up(&self) -> bool {
        self.native.game().modes().iter().any(|mode| matches!(mode, Mode::Battle(_)))
    }

    fn notice_battle_end(&mut self) {
        if self.in_battle && !self.battle_is_up() {
            self.in_battle = false;
            self.cut_trees.clear();
            self.event(AgentEvent::BattleEnded);
        }
    }

    // ---- The overworld ----

    fn overworld(&mut self) -> Result<Option<Command>, String> {
        let state = self.game_state()?;
        if self.last_map != Some(state.map.map) {
            self.last_map = Some(state.map.map);
            self.cut_trees.clear();
            let location = &self.native.game().world().location;
            self.graph.observe(state.map.map, Point8 { x: location.x, y: location.y }, &state.map);
        }
        if let Some(walk) = self.walk {
            return self.walk_on(walk, &state);
        }
        if let Some(field_move) = self.policy.pick_field_move(&state) {
            return self.field_move(field_move, &state);
        }
        let Some(action) = self.policy.pick_overworld_action(&state, &self.graph) else { return Ok(None) };
        self.answer_no = matches!(action.tile, MetaTile::Switch { object: HiddenObject::Quiz { yes: false }, .. });
        self.vending_pick = match action.tile {
            MetaTile::Switch { object: HiddenObject::VendingMachine, ordinal } => Some(ordinal - 1),
            _ => None,
        };
        self.event(AgentEvent::StartedOverworldAction { destination: action.tile, id: action.id() });
        let walk = Walk { destination: action.tile, map: action.map, pressed_a: false, route_lost: 0 };
        self.walk = Some(walk);
        self.walk_on(walk, &state)
    }

    /// `OverworldMovement`'s judgement, at a decision point rather than a tick.
    fn walk_on(&mut self, walk: Walk, state: &GameState) -> Result<Option<Command>, String> {
        let map = &state.map;
        let destination = walk.destination;
        if map.map != walk.map {
            if matches!(destination, MetaTile::Warp { .. } | MetaTile::Connection { .. } | MetaTile::ConnectionWater(_)) {
                self.complete();
            } else {
                // No position: it would be read against the wrong map.
                self.abort(OverworldActionAbortedReason::WrongMap(map.map), None);
            }
            return Ok(None);
        }
        if let MetaTile::Warp { to_map, to_position } = destination
            && to_map == map.map && map.player_position == to_position
        {
            // A teleport pad does not change the map.
            self.complete();
            return Ok(None);
        }
        if matches!(destination, MetaTile::Warp { .. }) && map.player_tile() == destination
            && is_on_map_border(map) && !map.is_step_on_warp(map.player_position) && !map.surfing && map.standing_on_warp
        {
            let at = map.player_position;
            let exit = if at.y == 0 { Direction::Up }
                else if at.y as usize == map.height.saturating_sub(1) { Direction::Down }
                else if at.x == 0 { Direction::Left }
                else { Direction::Right };
            return Ok(Some(Command::Step(exit)));
        }
        // A cave wander paces from wherever it is; a grass row once it stands in the grass.
        if destination == MetaTile::Empty {
            return match crate::pokemon::agent::adjacent_pacing_pair(map, map.player_position) {
                Some((a, b)) => self.start_pace(destination, state, a, b, false),
                None => {
                    self.abort(OverworldActionAbortedReason::NoRoute(destination), Some(map.player_position));
                    Ok(None)
                }
            };
        }
        if destination == MetaTile::Grass && map.player_tile() == destination {
            let at = map.player_position;
            let pair = crate::pokemon::agent::adjacent_grass(map, at).map(|b| (at, b))
                .or_else(|| crate::pokemon::agent::adjacent_pacing_pair(map, at));
            return match pair {
                Some((a, b)) => self.start_pace(destination, state, a, b, true),
                None => {
                    self.abort(OverworldActionAbortedReason::NoAdjacentGrass, Some(at));
                    Ok(None)
                }
            };
        }
        if map.player_tile() == destination && !matches!(destination, MetaTile::Warp { .. }) {
            self.complete();
            return Ok(None);
        }
        let row = map.actions().into_iter()
            .find(|a| a.tile.is_same_row_as(&destination))
            .or_else(|| match destination {
                MetaTile::Connection { to_map, to_position } => map.connection_action(to_map, to_position),
                MetaTile::ConnectionWater(to_map) => map.water_connection_action(to_map),
                _ => None,
            });
        let Some(row) = row else {
            if !map.position_settled {
                return Ok(None);
            }
            self.lose_route("no route")?;
            return Ok(None);
        };
        match row.route.first() {
            Some(JoypadButton::A) => Ok(Some(Command::Interact)),
            Some(&button) => {
                let direction = direction(button).ok_or_else(|| format!("a route pressed {button:?}"))?;
                let onto = step_pos(map.player_position, button).and_then(|at| map.tile_at_checked(at));
                if !map.surfing && matches!(onto, Some(MetaTile::Water | MetaTile::ConnectionWater(_)))
                    && let Some((slot, _)) = field_move_carrier(state, PokemonMoveName::Surf)
                {
                    let at = step_pos(map.player_position, button);
                    return self.start_field_move(FieldMoveUse::new(slot, PokemonMoveName::Surf, Then::Report)
                        .facing(direction).at(at));
                }
                // A press against what the row is for, a tree or a person, is a turn, which a step
                // would be refused as.
                Ok(Some(self.step_or_face(button)?))
            }
            None => match destination {
                // The route ends facing the tree.
                MetaTile::Cut { at } if state.can_use_cut
                    && map.tile_in_front() == Some((at, MetaTile::CutTree)) =>
                    match field_move_carrier(state, PokemonMoveName::Cut) {
                        Some((slot, _)) =>
                            self.start_field_move(FieldMoveUse::new(slot, PokemonMoveName::Cut, Then::Complete).at(Some(at))),
                        None => Err("a cut row with nobody to cut".into()),
                    },
                MetaTile::Pace { water } => {
                    let at = map.player_position;
                    match map.pacing_neighbour(at, crate::pokemon::agent::pace_kind(water)) {
                        Some(b) => self.start_pace(destination, state, at, b, true),
                        None => {
                            self.abort(OverworldActionAbortedReason::NoRoute(destination), Some(at));
                            Ok(None)
                        }
                    }
                }
                MetaTile::BoulderGoal { boulder, at, hole } => {
                    self.boulder_goal = Some(BoulderGoal { which: boulder, target: at, hole, pushes: 0, best: usize::MAX, stale: 0 });
                    self.goal_step(state)
                }
                MetaTile::BoulderPush { boulder, dir } => {
                    self.task = Some(Task::Push(Push { boulder, dir, armed: false, goal: false }));
                    self.push_step(state)
                }
                // The empty route is where a fishing row turns into a cast.
                MetaTile::Fish { .. } => {
                    let bag = &state.bag;
                    match (Rod::best_in_bag(bag), crate::pokemon::postgame::fishing::nearest_castable_water(map)) {
                        (Some(rod), Some(at)) => self.start_bag(rod.item(), BagHow::Cast { rod, row: true }, Some(at), state),
                        _ => Err(format!("a fishing row with no rod or no water: {destination}")),
                    }
                }
                MetaTile::Cut { .. } => Err(format!("a cut row with no tree in front: {destination}")),
                _ => {
                    self.complete();
                    Ok(None)
                }
            },
        }
    }

    /// Wait out a route that has gone, up to a bound, then give the walk up.
    fn lose_route(&mut self, _why: &str) -> Result<(), String> {
        let Some(walk) = self.walk.as_mut() else { return Ok(()) };
        walk.route_lost += 1;
        if walk.route_lost > MAX_ROUTE_LOST_POLLS {
            let destination = walk.destination;
            let at = self.player_at();
            self.abort(OverworldActionAbortedReason::NoRoute(destination), at);
        }
        self.native.game_mut().frame(Input::None);
        Ok(())
    }

    /// Something took the screen from the walk: a talk the walk was for, or a battle, a script or a
    /// text box that stopped it.
    fn end_walk_off_the_map(&mut self) -> Result<(), String> {
        let Some(walk) = self.walk else { return Ok(()) };
        let mode = self.native.game_state()?.mode;
        if walk.pressed_a && mode == GameMode::TextBox {
            self.walk = None;
            self.event(AgentEvent::OverworldInteractionCompleted { target: walk.destination });
        } else {
            let at = self.player_at();
            self.abort(OverworldActionAbortedReason::from_game_mode(mode), at);
        }
        Ok(())
    }

    fn player_at(&self) -> Option<Point8> {
        let location = &self.native.game().world().location;
        Some(Point8 { x: location.x, y: location.y })
    }

    fn complete(&mut self) {
        if let Some(walk) = self.walk.take() {
            self.event(AgentEvent::OverworldActionCompleted { destination: walk.destination });
        }
    }

    fn abort(&mut self, reason: OverworldActionAbortedReason, at: Option<Point8>) {
        if let Some(walk) = self.walk.take() {
            self.event(AgentEvent::OverworldActionAborted { destination: walk.destination, reason, at });
        }
    }

    // ---- Menus the agent walks itself ----

    /// A policy's own `FieldMove`.
    fn field_move(&mut self, field_move: FieldMove, state: &GameState) -> Result<Option<Command>, String> {
        match field_move {
            FieldMove::UseFieldMove { slot, move_index } => {
                let name = state.pokemon.get(slot as usize).and_then(|mon| field_move_at(mon, move_index))
                    .ok_or_else(|| format!("slot {slot} has no field move on row {move_index}"))?;
                self.start_field_move(FieldMoveUse::new(slot, name, Then::Report))
            }
            FieldMove::CutTree => {
                let carrier = field_move_carrier(state, PokemonMoveName::Cut).filter(|_| state.can_use_cut);
                let Some((slot, _)) = carrier else {
                    self.say("Cut needs a party member that knows it, and the CascadeBadge; not cutting");
                    return Ok(None);
                };
                let at = state.map.tile_in_front().map(|(at, _)| at);
                self.start_field_move(FieldMoveUse::new(slot, PokemonMoveName::Cut, Then::Report).at(at))
            }
            FieldMove::Fly { to } => {
                let world = self.native.game().world();
                let why = if (to as u8) >= 16 || world.location.towns_visited & 1 << to as u8 == 0 {
                    Some(format!("{to} has never been visited, so it is not on the town map"))
                } else {
                    None
                };
                match (why, field_move_carrier(state, PokemonMoveName::Fly)) {
                    (Some(why), _) => self.say(&format!("Fly: {why}")),
                    (None, None) => self.say("Fly: no party member knows FLY"),
                    (None, Some((slot, _))) => {
                        let mut fly = FieldMoveUse::new(slot, PokemonMoveName::Fly, Then::Report);
                        fly.to = Some(to);
                        return self.start_field_move(fly);
                    }
                }
                Ok(None)
            }
            FieldMove::PushBoulder { boulder, dir } => {
                self.task = Some(Task::Push(Push { boulder, dir, armed: false, goal: false }));
                self.push_step(state)
            }
            FieldMove::TossItem { item } => self.start_bag(item, BagHow::Toss, None, state),
            FieldMove::UseBagItem { item, target } => self.start_bag(item, BagHow::Use(target), None, state),
            FieldMove::TeachMove { item, target_slot } => {
                // Nothing the menus do could take this back, so it is refused here.
                if state.pokemon.get(target_slot as usize).is_some_and(|mon| !crate::pokemon::learnset::can_learn(mon.species, item)) {
                    self.say(&crate::pokemon::learnset::teach_refusal(state, item, target_slot));
                    return Ok(None);
                }
                self.start_bag(item, BagHow::Teach { slot: target_slot }, None, state)
            }
            FieldMove::EvolveWithStone { stone, target_slot, .. } =>
                self.start_bag(stone, BagHow::Evolve { slot: target_slot }, None, state),
            FieldMove::UseFieldItem { item, target } => {
                if let Some(refusal) = crate::pokemon::item_use::field_use_refusal(item) {
                    self.say(&refusal);
                    return Ok(None);
                }
                self.start_bag(item, BagHow::Use(UseTarget::Nothing), Some(target), state)
            }
            FieldMove::UseItemPc { op, item, qty, pc } => self.start_pc(pc, PcJob::Items { op, item, quantity: qty }),
            FieldMove::UsePcBox { op, pc } => self.start_pc(pc, PcJob::Box(op)),
            FieldMove::CheckTrashCan { target, facing } => self.start_talk(target, facing, Talk::Press),
            FieldMove::UseElevator { panel, floor } => self.start_talk(panel, None, Talk::Elevator { floor }),
            FieldMove::RedeemPrize { prize } => self.start_talk(prize.vendor_tile(), None, Talk::Prize(prize)),
            FieldMove::UsePartyScript { script, slot, npc } => self.start_talk(npc.0, Some(npc.1), Talk::Party { script, slot }),
            FieldMove::Fish { rod, at } => self.start_bag(rod.item(), BagHow::Cast { rod, row: false }, Some(at), state),
            FieldMove::ReorderParty { slot: 0 } => Ok(None),
            FieldMove::ReorderParty { slot } => {
                self.task = Some(Task::Switch(SwitchUse { slot, opened: false, stage: 0 }));
                self.task_step(self.native.game().status())
            }
            FieldMove::SellToMart { item, clerk } => {
                self.task = Some(Task::Mart(MartVisit {
                    clerk: Some(clerk), opened: false, asked: true, want: Some(MartWant::Sell(item)), chosen: false,
                    picked: false, before: bag_quantity(self.native.game().world(), item.id), quitting: false,
                }));
                self.task_step(self.native.game().status())
            }
        }
    }

    fn start_talk(&mut self, at: Point8, facing: Option<PlayerFacingDirection>, what: Talk) -> Result<Option<Command>, String> {
        let world = self.native.game().world();
        let nick = match what {
            Talk::Party { script: PartyScript::NameRater, slot } => world.party.get(slot as usize).map(|named| {
                let mut nick = [0x50; 11];
                for (to, &from) in nick.iter_mut().zip(&named.nick) {
                    *to = from;
                }
                nick
            }),
            _ => None,
        };
        let before = TalkBefore { coins: bcd(&world.coins), party: world.party.len(), nick };
        self.task = Some(Task::Talk(TalkUse { at, facing, what, opened: false, answered: false, before }));
        self.task_step(self.native.game().status())
    }

    fn talk_step(&mut self, mut talk: TalkUse, status: Status) -> Result<Option<Command>, String> {
        let command = match status {
            Status::Busy | Status::Idle => None,
            // Picked a floor: the exit is redirected, so it is walked.
            Status::Waiting(Decision::Overworld) if talk.opened && matches!(talk.what, Talk::Elevator { .. })
                && is_elevator(self.native.game().world().location.map) => {
                let state = self.game_state()?;
                let warp = state.map.actions().into_iter().find(|row| matches!(row.tile, MetaTile::Warp { .. }));
                match warp.and_then(|row| row.route.first().copied()) {
                    Some(button) if talk.answered => Some(self.step_or_face(button)?),
                    _ => return self.talk_done(talk),
                }
            }
            Status::Waiting(Decision::Overworld) if talk.opened => return self.talk_done(talk),
            Status::Waiting(Decision::Overworld) => {
                let state = self.game_state()?;
                let route = match talk.facing {
                    Some(facing) => state.map.route_to_face_dir(talk.at, Some(facing)),
                    None => state.map.route_to_face(talk.at),
                };
                match route.as_deref() {
                    Some([]) => {
                        talk.opened = true;
                        Some(Command::Interact)
                    }
                    Some(&[button, ..]) => Some(self.step_or_face(button)?),
                    None => {
                        self.task = None;
                        let what = state.map.tile_at_checked(talk.at)
                            .map_or_else(|| "a square that is not on this map".to_string(), |tile| format!("{tile}"));
                        self.say(&format!("Could not get next to {what} at {} to face it", talk.at));
                        return Ok(None);
                    }
                }
            }
            Status::Waiting(Decision::Text) => Some(Command::Advance),
            Status::Waiting(Decision::TwoOption) => Some(Command::ChooseOption(0)),
            Status::Waiting(Decision::NamingScreen) => self.name_answer()?,
            Status::Waiting(Decision::CursorMenu | Decision::List | Decision::PartyMenu) if talk.answered =>
                Some(match status {
                    Status::Waiting(Decision::List) => Command::CancelList,
                    _ => Command::CancelOption,
                }),
            Status::Waiting(decision @ (Decision::CursorMenu | Decision::List | Decision::PartyMenu)) => {
                talk.answered = true;
                let row = match talk.what {
                    Talk::Press => 0,
                    Talk::Elevator { floor } => floor,
                    Talk::Prize(prize) => prize.menu_row(),
                    Talk::Party { script: PartyScript::Trade { give, .. }, .. } => self.native.game().world().party.iter()
                        .position(|named| named.mon.mon.species == give).map_or(0, |slot| slot as u8),
                    Talk::Party { slot, .. } => slot,
                };
                Some(match decision {
                    Decision::List => Command::ChooseListEntry(row),
                    _ => Command::ChooseOption(row),
                })
            }
            Status::Waiting(decision) => return Err(format!("{:?} met {decision:?}", talk.what)),
        };
        self.task = Some(Task::Talk(talk));
        Ok(command)
    }

    fn talk_done(&mut self, talk: TalkUse) -> Result<Option<Command>, String> {
        self.task = None;
        let world = self.native.game().world();
        let message = match talk.what {
            Talk::Press => None,
            Talk::Elevator { floor } if is_elevator(world.location.map) => Some(format!("elevator: floor {floor} was not reached")),
            Talk::Elevator { .. } => None,
            Talk::Prize(prize) => {
                let coins = bcd(&world.coins);
                Some(match talk.before.coins.checked_sub(coins).filter(|&spent| spent > 0) {
                    Some(spent) => format!("bought {prize:?} for {spent} coins ({coins} left)"),
                    None => format!("prize: {prize:?} was not handed over"),
                })
            }
            Talk::Party { script, slot } => {
                let party = world.party.len();
                let done = match script {
                    PartyScript::Daycare => (party != talk.before.party).then(|| format!("party {} to {party}", talk.before.party)),
                    PartyScript::NameRater => world.party.get(slot as usize)
                        .filter(|named| talk.before.nick.is_some_and(|nick| !nick.starts_with(&named.nick) || nick.get(named.nick.len()) != Some(&0x50)))
                        .map(|_| format!("slot {slot} renamed")),
                    PartyScript::Trade { give, .. } => (!world.party.iter().any(|named| named.mon.mon.species == give))
                        .then(|| format!("traded away {give:?}")),
                };
                Some(match done {
                    Some(done) => format!("{script:?}: {done}"),
                    None => format!("party-script: {script:?} changed nothing"),
                })
            }
        };
        if let Some(message) = message {
            self.say(&message);
        }
        Ok(None)
    }

    fn switch_step(&mut self, mut switch: SwitchUse, status: Status) -> Result<Option<Command>, String> {
        let command = match status {
            Status::Busy | Status::Idle => None,
            Status::Waiting(Decision::Overworld) if switch.opened => {
                self.task = None;
                if switch.stage >= 3 {
                    self.say(&format!("Moved party slot {} to the front", switch.slot));
                } else {
                    self.say(&format!("party slot {} was not moved", switch.slot));
                }
                return Ok(None);
            }
            Status::Waiting(Decision::Overworld) => {
                switch.opened = true;
                Some(Command::OpenStartMenu)
            }
            Status::Waiting(Decision::StartMenu) if switch.stage > 0 => Some(Command::CloseStartMenu),
            Status::Waiting(Decision::StartMenu) => Some(Command::ChooseStartMenuEntry(StartMenuEntry::Pokemon)),
            Status::Waiting(Decision::PartyMenu) => match switch.stage {
                0 => {
                    switch.stage = 1;
                    Some(Command::ChooseOption(switch.slot))
                }
                2 => {
                    switch.stage = 3;
                    Some(Command::ChooseOption(0))
                }
                _ => {
                    switch.stage = switch.stage.max(4);
                    Some(Command::CancelOption)
                }
            },
            Status::Waiting(Decision::FieldMoveMenu) => {
                let Some(Mode::FieldMoveMenu(menu)) = self.native.game().modes().last() else {
                    return Err("the field-move menu is waiting but not on top".into());
                };
                let row = (0..menu.rows()).find(|&row| menu.choice(row) == FieldMoveChoice::Switch)
                    .ok_or("the field-move menu has no SWITCH")?;
                switch.stage = 2;
                Some(Command::ChooseOption(row))
            }
            Status::Waiting(Decision::Text) => Some(Command::Advance),
            Status::Waiting(decision) => return Err(format!("SWITCH met {decision:?}")),
        };
        self.task = Some(Task::Switch(switch));
        Ok(command)
    }

    /// A naming screen: a nickname is `pick_nickname`'s, and an empty name keeps the species'.
    fn name_answer(&mut self) -> Result<Option<Command>, String> {
        let Some(Mode::NamingScreen(screen)) = self.native.game().modes().last() else {
            return Err("a naming screen is waiting but not on top".into());
        };
        let (kind, species, limit) = (screen.kind(), screen.species(), screen.limit());
        let name = match (kind, species) {
            (pokered::modes::naming_screen::NamingScreenType::Mon, Some(species)) => {
                let Some(name) = self.policy.pick_nickname(species) else { return Ok(None) };
                name
            }
            (kind, _) => return Err(format!("no native answer yet for naming the {kind:?}")),
        };
        let bytes = name.and_then(|name| poke_core::charmap::encode(&name).ok())
            .filter(|bytes| bytes.len() <= limit && bytes.iter().all(|&byte| pokered::modes::naming_screen::NamingScreen::position_of(byte).is_some()))
            .unwrap_or_default();
        Ok(Some(Command::EnterName(bytes)))
    }

    fn mart_step(&mut self, mut mart: MartVisit, status: Status) -> Result<Option<Command>, String> {
        let command = match status {
            Status::Busy | Status::Idle => None,
            Status::Waiting(Decision::Overworld) if mart.opened => {
                self.task = None;
                return Ok(None);
            }
            Status::Waiting(Decision::Overworld) => {
                let Some((at, facing)) = mart.clerk else { return Err("a sale with no clerk".into()) };
                let state = self.game_state()?;
                match state.map.route_to_face_dir(at, Some(facing)).as_deref() {
                    Some([]) => {
                        mart.opened = true;
                        Some(Command::Interact)
                    }
                    Some(&[button, ..]) => Some(self.step_or_face(button)?),
                    None => {
                        self.task = None;
                        self.say(&format!("Can't reach the clerk at {at}"));
                        return Ok(None);
                    }
                }
            }
            Status::Waiting(Decision::Text) => Some(Command::Advance),
            Status::Waiting(Decision::BuySellQuit) => {
                if let Some(want) = mart.want.filter(|_| mart.chosen) {
                    self.mart_report(want, mart.before);
                    mart.want = match want {
                        MartWant::Buy(_) => self.policy.next_mart_purchase().and_then(|item| self.affordable(item)).map(MartWant::Buy),
                        MartWant::Sell(_) => None,
                    };
                    mart.chosen = false;
                    mart.picked = false;
                }
                if !mart.asked {
                    let state = self.native.game_state()?;
                    let Some(want) = self.policy.pick_mart_purchase(&state) else { return Ok(None) };
                    mart.asked = true;
                    mart.want = want.and_then(|item| self.affordable(item)).map(MartWant::Buy);
                }
                match mart.want {
                    Some(want) => {
                        let item = match want { MartWant::Buy(item) | MartWant::Sell(item) => item.id };
                        mart.before = bag_quantity(self.native.game().world(), item);
                        mart.chosen = true;
                        Some(Command::ChooseOption(matches!(want, MartWant::Sell(_)) as u8))
                    }
                    None => {
                        mart.quitting = true;
                        Some(Command::ChooseOption(2))
                    }
                }
            }
            Status::Waiting(Decision::List) if mart.picked || mart.want.is_none() => Some(Command::CancelList),
            Status::Waiting(Decision::List) => {
                mart.picked = true;
                let game = self.native.game();
                let row = match mart.want {
                    Some(MartWant::Buy(item)) => game.modes().iter().rev().find_map(|mode| match mode {
                        Mode::Pokemart(mart) => Some(mart.stock().iter().position(|&id| id == item.id)),
                        _ => None,
                    }).flatten(),
                    Some(MartWant::Sell(item)) => game.world().bag.items.iter().position(|slot| slot.id == item.id),
                    None => None,
                };
                Some(row.map_or(Command::CancelList, |row| Command::ChooseListEntry(row as u8)))
            }
            Status::Waiting(Decision::Quantity) => match (self.native.game().modes().last(), mart.want) {
                (Some(Mode::QuantityMenu(menu)), Some(MartWant::Buy(item) | MartWant::Sell(item))) =>
                    Some(Command::ChooseQuantity(item.quantity.clamp(1, menu.max()))),
                _ => Some(Command::CancelQuantity),
            },
            Status::Waiting(Decision::TwoOption) => Some(Command::ChooseOption(0)),
            Status::Waiting(decision) => return Err(format!("the mart met {decision:?}")),
        };
        self.task = Some(Task::Mart(mart));
        Ok(command)
    }

    fn mart_report(&mut self, want: MartWant, before: u8) {
        let world = self.native.game().world();
        let message = match want {
            MartWant::Buy(item) => match bag_quantity(world, item.id).saturating_sub(before) {
                0 => format!("Bought no {}", item.id),
                n => format!("Bought {} x{n}", item.id),
            },
            MartWant::Sell(item) => match before.saturating_sub(bag_quantity(world, item.id)) {
                0 => format!("Sold no {}", item.id),
                n => format!("Sold {} x{n}", item.id),
            },
        };
        self.say(&message);
    }

    /// Trimmed to the wallet, as the emulated side trims every purchase.
    fn affordable(&self, item: BagItem) -> Option<BagItem> {
        let money = bcd(&self.native.game().world().money);
        let item = match poke_core::item::price(item.id).map(|price| bcd(&price)) {
            Some(price) if price > 0 => BagItem::new(item.id, item.quantity.min((money / price).min(99) as u8)),
            _ => item,
        };
        (item.quantity > 0).then_some(item)
    }

    fn start_pc(&mut self, at: Point8, job: PcJob) -> Result<Option<Command>, String> {
        let world = self.native.game().world();
        let party = world.party.len() as u8;
        let boxed = world.boxes.get(world.current_box as usize).map_or(0, Vec::len) as u8;
        let quantity = match job {
            PcJob::Items { op, item, .. } => {
                let quantity = source(world, op).items.iter().find(|slot| slot.id == item).map_or(0, |slot| slot.quantity);
                if quantity == 0 {
                    self.say(&format!("{op:?} {item}: there is none to move"));
                    return Ok(None);
                }
                quantity
            }
            PcJob::Box(op) => {
                // The game answers these with a message and the same menu, which asking again loops on.
                if let Some(why) = op.blocked_by(party, boxed, world.current_box) {
                    self.say(&format!("PC box: {op:?} not possible: {why}"));
                    return Ok(None);
                }
                0
            }
        };
        let before = PcBefore { quantity, party, boxed };
        self.task = Some(Task::Pc(PcUse { at, job, opened: false, entered: false, started: false, picked: false, before }));
        self.task_step(self.native.game().status())
    }

    fn pc_step(&mut self, mut pc: PcUse, status: Status) -> Result<Option<Command>, String> {
        let modes = self.native.game().modes();
        let owner = modes.len().checked_sub(2).and_then(|below| modes.get(below));
        let command = match status {
            Status::Busy | Status::Idle => None,
            Status::Waiting(Decision::Overworld) if pc.opened => return self.pc_done(pc),
            Status::Waiting(Decision::Overworld) => {
                let state = self.game_state()?;
                match state.map.route_to_face_dir(pc.at, Some(PlayerFacingDirection::Up)).as_deref() {
                    Some([]) => {
                        pc.opened = true;
                        Some(Command::Interact)
                    }
                    Some(&[button, ..]) => Some(self.step_or_face(button)?),
                    None => {
                        self.task = None;
                        self.say(&format!("Can't reach the PC at {}", pc.at));
                        return Ok(None);
                    }
                }
            }
            Status::Waiting(Decision::Text) => Some(Command::Advance),
            Status::Waiting(Decision::CursorMenu) => {
                let Some(Mode::CursorMenu(menu)) = modes.last() else {
                    return Err("a cursor menu is waiting but not on top".into());
                };
                let log_off = menu.rows() - 1;
                Some(Command::ChooseOption(match (owner, pc.job) {
                    (Some(Mode::PcMenu(_)), _) if pc.entered => log_off,
                    (Some(Mode::PcMenu(_)), job) => {
                        pc.entered = true;
                        match job { PcJob::Items { .. } => 1, PcJob::Box(_) => 0 }
                    }
                    (Some(Mode::PlayerPc(_)), _) if pc.started => log_off,
                    (Some(Mode::PlayerPc(_)), PcJob::Items { op, .. }) => {
                        pc.started = true;
                        match op { PcItemOp::Withdraw => 0, PcItemOp::Deposit => 1 }
                    }
                    (Some(Mode::BillsPc(bills)), job) => match (bills.menu_up(), job) {
                        (Some(BillsPcMenu::Main), _) if pc.started => log_off,
                        (Some(BillsPcMenu::Main), PcJob::Box(op)) => {
                            pc.started = true;
                            match op {
                                PcBoxOp::Withdraw { .. } => 0,
                                PcBoxOp::Deposit { .. } => 1,
                                PcBoxOp::Release { .. } => 2,
                                PcBoxOp::ChangeBox { .. } => 3,
                            }
                        }
                        // WITHDRAW or DEPOSIT, the row above STATS.
                        (Some(BillsPcMenu::DepositWithdraw), _) => 0,
                        (Some(BillsPcMenu::Boxes), PcJob::Box(PcBoxOp::ChangeBox { n })) => n,
                        (menu, job) => return Err(format!("Bill's PC showed {menu:?} to {job:?}")),
                    },
                    (owner, job) => return Err(format!("a PC's cursor menu over {:?} for {job:?}",
                                                       owner.map(std::mem::discriminant))),
                }))
            }
            Status::Waiting(Decision::List) if pc.picked => Some(Command::CancelList),
            Status::Waiting(Decision::List) => {
                pc.picked = true;
                let world = self.native.game().world();
                let row = match pc.job {
                    PcJob::Items { op, item, .. } => source(world, op).items.iter().position(|slot| slot.id == item).map(|row| row as u8),
                    PcJob::Box(PcBoxOp::Deposit { slot }) => Some(slot),
                    PcJob::Box(PcBoxOp::Withdraw { box_slot } | PcBoxOp::Release { box_slot }) => Some(box_slot),
                    PcJob::Box(PcBoxOp::ChangeBox { .. }) => None,
                };
                Some(row.map_or(Command::CancelList, Command::ChooseListEntry))
            }
            Status::Waiting(Decision::Quantity) => match (modes.last(), pc.job) {
                (Some(Mode::QuantityMenu(menu)), PcJob::Items { quantity, .. }) => Some(Command::ChooseQuantity(quantity.min(menu.max()))),
                _ => return Err("a PC asked how many of something it was not asked to move".into()),
            },
            // Release's and CHANGE BOX's question.
            Status::Waiting(Decision::TwoOption) => Some(Command::ChooseOption(0)),
            Status::Waiting(decision) => return Err(format!("the PC met {decision:?}")),
        };
        self.task = Some(Task::Pc(pc));
        Ok(command)
    }

    fn pc_done(&mut self, pc: PcUse) -> Result<Option<Command>, String> {
        self.task = None;
        let world = self.native.game().world();
        let message = match pc.job {
            PcJob::Items { op, item, .. } => {
                let left = source(world, op).items.iter().find(|slot| slot.id == item).map_or(0, |slot| slot.quantity);
                match pc.before.quantity.saturating_sub(left) {
                    0 => format!("{op:?} {item}: the PC moved none"),
                    moved => format!("{op:?} {item} x{moved} via the PC"),
                }
            }
            PcJob::Box(op) => {
                let party = world.party.len() as u8;
                let boxed = world.boxes.get(world.current_box as usize).map_or(0, Vec::len) as u8;
                let done = match op {
                    PcBoxOp::Deposit { .. } => party < pc.before.party,
                    PcBoxOp::Withdraw { .. } => party > pc.before.party,
                    PcBoxOp::Release { .. } => boxed < pc.before.boxed,
                    PcBoxOp::ChangeBox { n } => world.current_box == n,
                };
                match done {
                    true => format!("PC box: {op:?} done, party {party}, box {} holds {boxed}", world.current_box + 1),
                    false => format!("PC box: {op:?} did nothing"),
                }
            }
        };
        self.say(&message);
        Ok(None)
    }

    fn start_bag(&mut self, item: ItemId, how: BagHow, face: Option<Point8>, state: &GameState) -> Result<Option<Command>, String> {
        let world = self.native.game().world();
        let quantity = world.bag.items.iter().find(|slot| slot.id == item).map_or(0, |slot| slot.quantity);
        if quantity == 0 {
            self.say(&format!("there is no {item:?} in the bag"));
            return Ok(None);
        }
        let slot = match how {
            BagHow::Use(target) => target.slot(),
            BagHow::Teach { slot } | BagHow::Evolve { slot } => Some(slot),
            BagHow::Toss | BagHow::Cast { .. } => None,
        };
        let mon = slot.and_then(|slot| state.pokemon.get(slot as usize));
        let before = Before {
            quantity,
            species: mon.map(|mon| mon.species),
            moves: mon.map_or([None; 4], |mon| mon.moves.map(|mv| mv.map(|mv| mv.name))),
            walk_bike_surf: world.location.walk_bike_surf,
        };
        self.task = Some(Task::Bag(BagUse { item, how, face, opened: false, picked: false, party: false, moves: false, back: false, before }));
        self.task_step(self.native.game().status())
    }

    fn bag_step(&mut self, mut bag: BagUse, status: Status) -> Result<Option<Command>, String> {
        let slot = match bag.how {
            BagHow::Use(target) => target.slot(),
            BagHow::Teach { slot } | BagHow::Evolve { slot } => Some(slot),
            BagHow::Toss | BagHow::Cast { .. } => None,
        };
        let command = match status {
            // A Rare Candy's level-up can be kept from evolving the mon, as B would.
            Status::Busy | Status::Idle => match self.native.game().modes().last() {
                Some(Mode::Evolution(evolution)) if evolution.can_cancel()
                    && matches!(bag.how, BagHow::Use(UseTarget::Party { evolve: false, .. })) => Some(Command::CancelEvolution),
                _ => None,
            },
            Status::Waiting(Decision::Overworld) if bag.opened => return self.bag_done(bag),
            Status::Waiting(Decision::Overworld) => match bag.face {
                Some(target) => {
                    let state = self.game_state()?;
                    match state.map.route_to_face(target).as_deref() {
                        Some([]) => {
                            bag.opened = true;
                            Some(Command::OpenStartMenu)
                        }
                        Some(&[button, ..]) => Some(self.step_or_face(button)?),
                        None => {
                            self.task = None;
                            self.say(&format!("Can't reach the field-item target at {target}"));
                            return Ok(None);
                        }
                    }
                }
                None => {
                    bag.opened = true;
                    Some(Command::OpenStartMenu)
                }
            },
            Status::Waiting(Decision::StartMenu) if bag.picked => Some(Command::CloseStartMenu),
            Status::Waiting(Decision::StartMenu) => Some(Command::ChooseStartMenuEntry(StartMenuEntry::Item)),
            Status::Waiting(Decision::List) if bag.picked => {
                bag.back = true;
                Some(Command::CancelList)
            }
            Status::Waiting(Decision::List) => {
                bag.picked = true;
                match self.native.game().world().bag.items.iter().position(|slot| slot.id == bag.item) {
                    Some(index) => Some(Command::ChooseListEntry(index as u8)),
                    None => Some(Command::CancelList),
                }
            }
            Status::Waiting(Decision::UseToss) => Some(Command::ChooseOption(matches!(bag.how, BagHow::Toss) as u8)),
            Status::Waiting(Decision::PartyMenu) => match slot.filter(|_| !bag.party) {
                Some(slot) => {
                    bag.party = true;
                    Some(Command::ChooseOption(slot))
                }
                None => Some(Command::CancelOption),
            },
            Status::Waiting(Decision::MoveMenu) => match bag.how {
                BagHow::Use(UseTarget::Move { move_index, .. }) if !bag.moves => {
                    bag.moves = true;
                    Some(Command::ChooseOption(move_index))
                }
                _ => Some(Command::CancelOption),
            },
            // A toss is of the whole stack, which is what frees the slot.
            Status::Waiting(Decision::Quantity) => match self.native.game().modes().last() {
                Some(Mode::QuantityMenu(menu)) => Some(Command::ChooseQuantity(menu.max())),
                _ => return Err("a quantity is waiting but its menu is not on top".into()),
            },
            Status::Waiting(Decision::TwoOption | Decision::ForgetMove) if self.learning().is_some() => self.learn_answer(&status)?,
            Status::Waiting(Decision::TwoOption) => Some(Command::ChooseOption(0)),
            Status::Waiting(Decision::Text) => Some(Command::Advance),
            Status::Waiting(decision) => return Err(format!("{:?} from the bag met {decision:?}", bag.item)),
        };
        self.task = Some(Task::Bag(bag));
        Ok(command)
    }

    fn bag_done(&mut self, bag: BagUse) -> Result<Option<Command>, String> {
        self.task = None;
        let world = self.native.game().world();
        let quantity = world.bag.items.iter().find(|slot| slot.id == bag.item).map_or(0, |slot| slot.quantity);
        let mon = |slot: u8| world.party.get(slot as usize).map(|named| &named.mon.mon);
        let item = bag.item;
        let message = match bag.how {
            BagHow::Toss if quantity < bag.before.quantity => format!("Tossed {item:?} to free a bag slot"),
            BagHow::Toss => format!("the game would not toss {item:?}"),
            BagHow::Teach { slot } if mon(slot).is_some_and(|mon| mon.moves != bag.before.moves) =>
                format!("Taught {item:?} to party slot {slot}"),
            BagHow::Teach { slot } => format!("Party slot {slot} did not learn the move from {item:?}"),
            BagHow::Evolve { slot } if mon(slot).is_some_and(|mon| Some(mon.species) != bag.before.species) =>
                format!("Evolved party slot {slot}"),
            BagHow::Evolve { slot } => format!("Party slot {slot} did not evolve on {item:?}"),
            BagHow::Cast { rod, .. } if bag.back => {
                self.abort(OverworldActionAbortedReason::Textbox, self.player_at());
                format!("the game would not let the {} be cast here", rod.name())
            }
            BagHow::Cast { rod, row } => {
                if row {
                    self.complete();
                }
                format!("You cast the {} in. Not even a nibble.", rod.name())
            }
            BagHow::Use(target) => {
                let took = match crate::pokemon::postgame::items::effect(item) {
                    Effect::Consumed => quantity < bag.before.quantity,
                    Effect::TogglesBicycle => world.location.walk_bike_surf != bag.before.walk_bike_surf,
                    Effect::OneShot => true,
                };
                if took {
                    format!("Used {item:?} ({target:?})")
                } else {
                    format!("the game refused to use {item:?} here and put the bag back")
                }
            }
        };
        self.say(&message);
        Ok(None)
    }

    /// The `LearnMove` on the stack, if a mon is being offered a move.
    fn learning(&self) -> Option<&pokered::modes::learn_move::LearnMove> {
        self.native.game().modes().iter().rev().find_map(|mode| match mode {
            Mode::LearnMove(learn) => Some(learn),
            _ => None,
        })
    }

    /// `LearnMove`'s two questions and its list, answered by `pick_move_to_forget` once and held.
    /// An HM cannot be forgotten, so choosing one is declining.
    fn learn_answer(&mut self, status: &Status) -> Result<Option<Command>, String> {
        let Some(learn) = self.learning() else { return Ok(None) };
        let (slot, new_move) = learn.learning();
        let asking_to_delete = learn.asking_to_delete();
        let answer = match self.forget {
            Some(answer) => answer,
            None => {
                let state = self.native.game_state()?;
                let current: Vec<_> = state.pokemon.get(slot as usize)
                    .map(|mon| mon.moves.iter().flatten().copied().collect()).unwrap_or_default();
                let Some(answer) = self.policy.pick_move_to_forget(slot as usize, &current, new_move) else { return Ok(None) };
                let answer = answer.filter(|&i| current.get(i).is_some_and(|mv| !pokered::systems::learn_move::is_move_hm(mv.name)));
                self.forget = Some(answer);
                answer
            }
        };
        Ok(Some(match status {
            Status::Waiting(Decision::ForgetMove) => match answer {
                Some(row) => Command::ChooseOption(row as u8),
                None => Command::CancelOption,
            },
            // YES deletes a move; on the other question, YES abandons learning.
            _ if asking_to_delete => Command::ChooseOption(answer.is_none() as u8),
            _ => Command::ChooseOption(answer.is_some() as u8),
        }))
    }

    /// A press toward a row's target: a step, or a turn where a step would be refused.
    fn step_or_face(&self, button: JoypadButton) -> Result<Command, String> {
        let direction = direction(button).ok_or_else(|| format!("a route pressed {button:?}"))?;
        let world = self.native.game().world();
        let blocked = match self.native.game().modes().last() {
            Some(Mode::Overworld(overworld)) => overworld.step_refusal(direction, world).is_some(),
            _ => false,
        };
        Ok(if blocked && world.location.facing != direction.facing() { Command::Face(direction) } else { Command::Step(direction) })
    }

    fn start_field_move(&mut self, field_move: FieldMoveUse) -> Result<Option<Command>, String> {
        self.task = Some(Task::FieldMove(field_move));
        self.task_step(self.native.game().status())
    }

    fn task_step(&mut self, status: Status) -> Result<Option<Command>, String> {
        match self.task {
            Some(Task::FieldMove(field_move)) => self.field_move_step(field_move, status),
            Some(Task::Bag(bag)) => self.bag_step(bag, status),
            Some(Task::Pc(pc)) => self.pc_step(pc, status),
            Some(Task::Mart(mart)) => self.mart_step(mart, status),
            Some(Task::Talk(talk)) => self.talk_step(talk, status),
            Some(Task::Switch(switch)) => self.switch_step(switch, status),
            Some(Task::Pace(pace)) => match status {
                Status::Waiting(Decision::Overworld) => {
                    let state = self.game_state()?;
                    self.pace_step(pace, &state)
                }
                Status::Waiting(Decision::Text) => Ok(Some(Command::Advance)),
                Status::Busy | Status::Idle => Ok(None),
                Status::Waiting(decision) => Err(format!("pacing met {decision:?}")),
            },
            Some(Task::Push(_)) => match status {
                Status::Waiting(Decision::Overworld) => {
                    let state = self.game_state()?;
                    self.push_step(&state)
                }
                Status::Waiting(Decision::Text) => Ok(Some(Command::Advance)),
                Status::Busy | Status::Idle => Ok(None),
                Status::Waiting(decision) => Err(format!("a boulder's push met {decision:?}")),
            },
            None => Ok(None),
        }
    }

    /// One decision point of a field move's menus. What comes back to the overworld is judged by
    /// whether the party menu came back first, which is how every refusal ends.
    fn field_move_step(&mut self, mut use_: FieldMoveUse, status: Status) -> Result<Option<Command>, String> {
        let command = match status {
            Status::Busy | Status::Idle => None,
            Status::Waiting(Decision::Overworld) if use_.opened => return self.field_move_done(use_),
            Status::Waiting(Decision::Overworld) => match use_.face {
                Some(direction) if self.native.game().world().location.facing != direction.facing() =>
                    Some(Command::Face(direction)),
                _ => {
                    use_.opened = true;
                    Some(Command::OpenStartMenu)
                }
            },
            Status::Waiting(Decision::StartMenu) if use_.chosen => Some(Command::CloseStartMenu),
            Status::Waiting(Decision::StartMenu) => Some(Command::ChooseStartMenuEntry(StartMenuEntry::Pokemon)),
            Status::Waiting(Decision::PartyMenu) if use_.chosen => {
                use_.refused = true;
                Some(Command::CancelOption)
            }
            Status::Waiting(Decision::PartyMenu) => Some(Command::ChooseOption(use_.slot)),
            Status::Waiting(Decision::FieldMoveMenu) => {
                let Some(Mode::FieldMoveMenu(menu)) = self.native.game().modes().last() else {
                    return Err("the field-move menu is waiting but not on top".into());
                };
                let row = (0..menu.rows()).find(|&row| menu.choice(row) == FieldMoveChoice::Move(use_.name))
                    .ok_or_else(|| format!("slot {} has no {} row", use_.slot, use_.name))?;
                use_.chosen = true;
                Some(Command::ChooseOption(row))
            }
            // The fly screen's rows are the visited towns in map order. The town is taken once: the
            // screen coming back means it was not.
            Status::Waiting(Decision::FlyDestination) => match use_.to.take() {
                Some(to) => {
                    let visited = self.native.game().world().location.towns_visited;
                    Some(Command::ChooseOption((visited & ((1u16 << to as u8) - 1)).count_ones() as u8))
                }
                None => {
                    use_.refused = true;
                    Some(Command::CancelOption)
                }
            },
            Status::Waiting(Decision::Text) => Some(Command::Advance),
            Status::Waiting(decision) => return Err(format!("{} from the party menu met {decision:?}", use_.name)),
        };
        self.task = Some(Task::FieldMove(use_));
        Ok(command)
    }

    fn field_move_done(&mut self, use_: FieldMoveUse) -> Result<Option<Command>, String> {
        self.task = None;
        if use_.name == PokemonMoveName::Cut && !use_.refused && let Some(at) = use_.at {
            self.cut_trees.push(at);
        }
        let state = self.game_state()?;
        let succeeded = !use_.refused && match use_.name {
            PokemonMoveName::Surf => state.map.surfing,
            PokemonMoveName::Strength => state.strength_active,
            _ => true,
        };
        let at = use_.at.map_or_else(String::new, |at| format!(" at {at}"));
        match (use_.then, succeeded) {
            (Then::Push(push), true) => {
                self.task = Some(Task::Push(push));
                return self.push_step(&state);
            }
            (Then::Push(push), false) => {
                self.boulder_goal = None;
                self.say(&format!("Strength did not arm, so the boulder at {} was not pushed", push.boulder));
                self.abort(OverworldActionAbortedReason::Unknown, self.player_at());
            }
            (Then::Complete, true) => self.complete(),
            (_, false) => {
                self.say(&format!("the game refused {}{at}", use_.name));
                self.abort(OverworldActionAbortedReason::Textbox, self.player_at());
            }
            (Then::Report, true) => match use_.name {
                PokemonMoveName::Surf => self.say(&format!("Surfed onto the water{at}")),
                PokemonMoveName::Cut => self.say(&format!("Cut down the tree{at}")),
                PokemonMoveName::Fly => self.say(&format!("Flew to {}", state.map.map)),
                _ => {}
            },
        }
        Ok(None)
    }

    /// `SolvingBoulderPuzzle`: judged and re-planned between shoves.
    fn goal_step(&mut self, state: &GameState) -> Result<Option<Command>, String> {
        /// Absolute ceiling on the shoves one goal may spend.
        const MAX_PUSHES: u8 = 120;
        /// The bound that protects: shoves since the plan last got shorter.
        const MAX_PUSHES_WITHOUT_PROGRESS: u8 = 12;
        let Some(mut goal) = self.boulder_goal else { return Ok(None) };
        let live = state.map.boulders();
        let away = |b: &Point8| (b.x as i32 - goal.which.x as i32).abs() + (b.y as i32 - goal.which.y as i32).abs();
        let moved_to = live.iter().copied().min_by_key(away).filter(|b| away(b) <= 1);
        // A switch keeps its boulder and a hole swallows it.
        let done = goal.pushes > 0 && (live.contains(&goal.target)
            || goal.hole && !live.contains(&goal.which) && moved_to.is_none());
        if done {
            self.boulder_goal = None;
            self.complete();
            return Ok(None);
        }
        if goal.pushes >= MAX_PUSHES || goal.stale >= MAX_PUSHES_WITHOUT_PROGRESS {
            self.boulder_goal = None;
            self.abort(OverworldActionAbortedReason::PuzzleRanLong { pushes: goal.pushes }, Some(state.map.player_position));
            return Ok(None);
        }
        goal.which = moved_to.unwrap_or(goal.which);
        let plan = state.map.solve_boulder_push_for(goal.which, goal.target);
        if let Some(steps) = plan.as_ref().map(Vec::len) && steps < goal.best {
            goal.best = steps;
            goal.stale = 0;
        }
        match plan.and_then(|plan| plan.into_iter().next()) {
            Some((boulder, dir)) => {
                goal.pushes += 1;
                goal.stale = goal.stale.saturating_add(1);
                self.boulder_goal = Some(goal);
                self.task = Some(Task::Push(Push { boulder, dir, armed: false, goal: true }));
                self.push_step(state)
            }
            None => {
                self.boulder_goal = None;
                self.abort(OverworldActionAbortedReason::PuzzleUnsolvable, Some(state.map.player_position));
                Ok(None)
            }
        }
    }

    fn start_pace(&mut self, destination: MetaTile, state: &GameState, a: Point8, b: Point8, heading_to_b: bool) -> Result<Option<Command>, String> {
        /// `PACING_BUDGET_TICKS`' minute of game time, in frames.
        const PACING_BUDGET_FRAMES: u64 = 60 * 60;
        let pace = Pace { destination, map: state.map.map, a, b, heading_to_b, stalled: 0, until: self.frames + PACING_BUDGET_FRAMES };
        self.task = Some(Task::Pace(pace));
        self.pace_step(pace, state)
    }

    /// A step toward whichever of the pair is next; a pair the map has moved under is picked again.
    fn pace_step(&mut self, mut pace: Pace, state: &GameState) -> Result<Option<Command>, String> {
        /// Refused steps before the pair is picked again.
        const STALL_STEPS: u8 = 8;
        let map = &state.map;
        let at = map.player_position;
        if map.map != pace.map {
            self.task = None;
            self.abort(OverworldActionAbortedReason::WrongMap(map.map), Some(at));
            return Ok(None);
        }
        if self.frames >= pace.until {
            self.task = None;
            self.abort(OverworldActionAbortedReason::NothingAppeared, Some(at));
            return Ok(None);
        }
        if at == if pace.heading_to_b { pace.b } else { pace.a } {
            pace.heading_to_b = !pace.heading_to_b;
            pace.stalled = 0;
        }
        let next = if pace.heading_to_b { pace.b } else { pace.a };
        let command = crate::pokemon::agent::dir_to(at, next).and_then(direction).map(|direction| {
            let refused = match self.native.game().modes().last() {
                Some(Mode::Overworld(overworld)) => overworld.step_refusal(direction, self.native.game().world()).is_some(),
                _ => false,
            };
            (direction, refused)
        });
        let command = match command {
            Some((direction, false)) => Some(Command::Step(direction)),
            // Someone stands on it, or it is not beside the player any more.
            _ => {
                pace.stalled += 1;
                if pace.stalled >= STALL_STEPS {
                    let repicked = match pace.destination {
                        MetaTile::Pace { water } => map.pacing_neighbour(at, crate::pokemon::agent::pace_kind(water)).map(|b| (at, b)),
                        _ => crate::pokemon::agent::adjacent_grass(map, at).map(|b| (at, b))
                            .or_else(|| crate::pokemon::agent::adjacent_pacing_pair(map, at)),
                    }.filter(|&pair| pair != (pace.a, pace.b));
                    match repicked {
                        Some((a, b)) => {
                            pace = Pace { a, b, heading_to_b: true, stalled: 0, ..pace };
                        }
                        None => {
                            self.task = None;
                            self.abort(OverworldActionAbortedReason::Unknown, Some(at));
                            return Ok(None);
                        }
                    }
                }
                None
            }
        };
        self.task = Some(Task::Pace(pace));
        Ok(command)
    }

    /// `PushingBoulder`: done once the boulder has left its square, a shove or a hole later.
    fn push_step(&mut self, state: &GameState) -> Result<Option<Command>, String> {
        let Some(Task::Push(push)) = self.task else { return Ok(None) };
        let map = &state.map;
        let boulder_at = |at: Point8| map.sprites.iter()
            .any(|sprite| sprite.name.starts_with("Boulder") && !sprite.hidden && sprite.position == at);
        if !boulder_at(push.boulder) {
            self.task = None;
            if push.goal {
                return self.goal_step(state);
            }
            self.complete();
            return Ok(None);
        }
        if let Some(refusal) = map.boulder_push_refusal(push.boulder, push.dir) {
            self.task = None;
            // A refused shove ends the goal rather than re-planning into it.
            self.boulder_goal = None;
            self.say(&refusal);
            self.abort(OverworldActionAbortedReason::Unknown, self.player_at());
            return Ok(None);
        }
        if !state.strength_active {
            let carrier = field_move_carrier(state, PokemonMoveName::Strength)
                .filter(|_| state.badges.contains(crate::pokemon::badge::Badge::RainbowBadge) && !push.armed);
            let Some((slot, _)) = carrier else {
                self.task = None;
                self.boulder_goal = None;
                self.say("Strength needs a party member that knows it, and the RainbowBadge; not pushing");
                self.abort(OverworldActionAbortedReason::Unknown, self.player_at());
                return Ok(None);
            };
            return self.start_field_move(FieldMoveUse::new(slot, PokemonMoveName::Strength,
                                                           Then::Push(Push { armed: true, ..push })));
        }
        let dir = direction(push.dir).ok_or_else(|| format!("a boulder pushed {:?}", push.dir))?;
        let behind = step_pos(push.boulder, opposite(push.dir));
        if behind == Some(map.player_position) {
            return Ok(Some(Command::Push(dir)));
        }
        match behind.and_then(|behind| map.route_to_push_tile(behind)).and_then(|route| route.first().copied()) {
            Some(button) => Ok(Some(Command::Step(direction(button).ok_or_else(|| format!("a route pressed {button:?}"))?))),
            None => {
                self.task = None;
                self.boulder_goal = None;
                let row = MetaTile::BoulderPush { boulder: push.boulder, dir: push.dir };
                self.abort(OverworldActionAbortedReason::NoRoute(row), self.player_at());
                Ok(None)
            }
        }
    }

    fn say(&mut self, message: &str) {
        self.event(AgentEvent::TextBox { message: message.to_string() });
    }

    // ---- The battle ----

    fn battle(&mut self) -> Result<Option<Command>, String> {
        if !self.in_battle {
            self.in_battle = true;
            self.event(AgentEvent::BattleStarted);
        }
        let state = self.native.game_state()?;
        let Some(action) = self.policy.pick_battle_action(&state) else { return Ok(None) };
        Ok(Some(match action {
            BattleAction::Fight { slot, .. } => Command::Fight(slot),
            BattleAction::UseItem { item, target, .. } => Command::UseItem { item: item.id, target },
            BattleAction::SwitchPokemon { slot, .. } => Command::SwitchPokemon(slot),
            BattleAction::Run => Command::Run,
            BattleAction::SafariBall => Command::SafariBall,
            BattleAction::SafariBait => Command::SafariBait,
            BattleAction::SafariRock => Command::SafariRock,
        }))
    }
}

fn is_elevator(map: Map) -> bool {
    matches!(map, Map::RocketHideoutElevator | Map::SilphCoElevator | Map::CeladonMartElevator)
}

fn bag_quantity(world: &pokered::world::World, item: ItemId) -> u8 {
    world.bag.items.iter().find(|slot| slot.id == item).map_or(0, |slot| slot.quantity)
}

/// Money and prices as the cartridge keeps them, three bytes of two decimal digits.
fn bcd(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |total, &byte| total * 100 + (byte >> 4) as u32 * 10 + (byte & 0x0F) as u32)
}

/// The side an item leaves.
fn source(world: &pokered::world::World, op: PcItemOp) -> &pokered::systems::inventory::Inventory {
    match op {
        PcItemOp::Deposit => &world.bag,
        PcItemOp::Withdraw => &world.pc_items,
    }
}

impl FieldMoveUse {
    fn new(slot: u8, name: PokemonMoveName, then: Then) -> Self {
        Self { slot, name, to: None, face: None, at: None, opened: false, chosen: false, refused: false, then }
    }

    fn facing(self, direction: Direction) -> Self {
        Self { face: Some(direction), ..self }
    }

    fn at(self, at: Option<Point8>) -> Self {
        Self { at, ..self }
    }
}

fn opposite(button: JoypadButton) -> JoypadButton {
    match button {
        JoypadButton::Up => JoypadButton::Down,
        JoypadButton::Down => JoypadButton::Up,
        JoypadButton::Left => JoypadButton::Right,
        JoypadButton::Right => JoypadButton::Left,
        other => other,
    }
}

fn direction(button: JoypadButton) -> Option<Direction> {
    match button {
        JoypadButton::Up => Some(Direction::Up),
        JoypadButton::Down => Some(Direction::Down),
        JoypadButton::Left => Some(Direction::Left),
        JoypadButton::Right => Some(Direction::Right),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use poke_core::charmap::encode;
    use poke_core::species::PokemonSpecies;
    use pokered::modes::overworld::Overworld;
    use pokered::party::Named;
    use pokered::rng::GameRng;
    use pokered::systems::add_mon::{new_party_mon, Origin};
    use pokered::systems::overworld::location::Location;
    use pokered::world::World;
    use pokered::Pacing;

    use super::*;
    use crate::pokemon::actions::OverworldAction;
    use crate::pokemon::move_name::PokemonMoveName;

    #[derive(Debug, Clone, Copy)]
    enum Goal {
        /// Take the row into this map.
        Enter(Map),
        /// Talk to the person of this name on the map.
        Talk(&'static str),
        /// Take the first cut row on the map.
        Cut,
        /// Take the first row of this kind: grass, a boulder goal, a vending machine's drink.
        Row(fn(&MetaTile) -> bool),
        /// Ask for this, once.
        Field(FieldMove),
    }

    #[derive(Default)]
    struct Log {
        events: Vec<String>,
        battles: u32,
        goals_left: usize,
    }

    /// Takes the goals in order, a row at a time, and fights with the first move that has PP.
    struct Goals {
        goals: Vec<Goal>,
        log: Rc<RefCell<Log>>,
        /// The row `pick_move_to_forget` answers with; none declines.
        forget: Option<usize>,
        /// What each mart visit buys, in order.
        purchases: Vec<BagItem>,
    }

    impl Policy for Goals {
        fn name(&self) -> &'static str {
            "goals"
        }

        fn pick_move_to_forget(&mut self, _slot: usize, _moves: &[crate::pokemon::move_name::PokemonMove],
                               _new_move: PokemonMoveName) -> Option<Option<usize>> {
            Some(self.forget)
        }

        fn pick_mart_purchase(&mut self, _state: &GameState) -> Option<Option<BagItem>> {
            Some(self.next_mart_purchase())
        }

        fn next_mart_purchase(&mut self) -> Option<BagItem> {
            (!self.purchases.is_empty()).then(|| self.purchases.remove(0))
        }

        fn pick_field_move(&mut self, _state: &GameState) -> Option<FieldMove> {
            let Some(&Goal::Field(field_move)) = self.goals.first() else { return None };
            self.goals.remove(0);
            self.log.borrow_mut().goals_left = self.goals.len();
            Some(field_move)
        }

        fn pick_overworld_action(&mut self, state: &GameState, _graph: &WorldGraph) -> Option<OverworldAction> {
            let goal = *self.goals.first()?;
            let row = state.map.actions().into_iter().find(|row| match (goal, row.tile) {
                (Goal::Enter(map), MetaTile::Warp { to_map, .. } | MetaTile::Connection { to_map, .. }
                                   | MetaTile::ConnectionWater(to_map)) => to_map == map,
                (Goal::Talk(who), MetaTile::Sprite(name)) => name == who,
                (Goal::Cut, MetaTile::Cut { .. }) => true,
                (Goal::Row(wanted), tile) => wanted(&tile),
                _ => false,
            });
            Some(row.unwrap_or_else(|| panic!("no row for {goal:?} on {}", state.map.map)))
        }

        fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
            let battle = state.battle?;
            battle.player.available_battle_moves().into_iter().next()
        }

        fn on_event(&mut self, event: &AgentEvent) {
            let mut log = self.log.borrow_mut();
            if let AgentEvent::OverworldInteractionCompleted { target: MetaTile::Sprite(who) } = event
                && matches!(self.goals.first(), Some(Goal::Talk(name)) if name == who)
            {
                self.goals.remove(0);
            }
            if let AgentEvent::OverworldActionCompleted { destination: MetaTile::Warp { to_map, .. }
                | MetaTile::Connection { to_map, .. } | MetaTile::ConnectionWater(to_map) } = event
                && matches!(self.goals.first(), Some(Goal::Enter(map)) if map == to_map)
            {
                self.goals.remove(0);
            }
            if let AgentEvent::OverworldActionCompleted { destination: MetaTile::Cut { .. } } = event
                && matches!(self.goals.first(), Some(Goal::Cut))
            {
                self.goals.remove(0);
            }
            if let AgentEvent::OverworldActionCompleted { destination } | AgentEvent::OverworldInteractionCompleted { target: destination } = event
                && matches!(self.goals.first(), Some(Goal::Row(wanted)) if wanted(destination))
            {
                self.goals.remove(0);
            }
            log.goals_left = self.goals.len();
            if matches!(event, AgentEvent::BattleStarted) {
                log.battles += 1;
            }
            log.events.push(format!("{event:?}"));
        }
    }

    const BOULDER: u8 = 1 << 0;
    const CASCADE: u8 = 1 << 1;
    const THUNDER: u8 = 1 << 2;
    const SOUL: u8 = 1 << 4;

    fn named(species: PokemonSpecies, moves: [Option<PokemonMoveName>; 4]) -> Named<pokered::party::PartyMon> {
        let mut mon = new_party_mon(species, 70, 1, &Origin::Trainer, &mut GameRng::tape(vec![]));
        for (slot, name) in moves.into_iter().enumerate().skip(1) {
            if name.is_some() {
                mon.mon.moves[slot] = name;
                mon.mon.pp[slot] = 10;
            }
        }
        let name = species.name();
        Named { mon, ot: encode("RED").unwrap(), nick: name.to_vec() }
    }

    /// The player standing at `(x, y)` on `map`, past Oak's stopping them at the grass, with a
    /// Mewtwo and whatever `edit` adds.
    fn game_at(map: Map, x: u8, y: u8, edit: impl FnOnce(&mut World)) -> Game {
        let mut world = World::default();
        world.player_name = encode("RED").unwrap();
        world.party = vec![named(PokemonSpecies::Mewtwo, [None; 4])];
        world.location = Location { map, x, y, last_map: map, ..Location::default() };
        world.events.set(poke_core::symbols::pokered_events::EVENT_FOLLOWED_OAK_INTO_LAB);
        edit(&mut world);
        let mut game = Game::new(world, GameRng::seeded(11), Pacing::Instant);
        game.push(Mode::Overworld(Overworld::new()));
        game
    }

    fn pallet_town() -> Game {
        game_at(Map::PalletTown, 5, 6, |_| {})
    }

    fn agent(game: Game, goals: Vec<Goal>) -> (NativeAgent, Rc<RefCell<Log>>) {
        agent_forgetting(game, goals, None)
    }

    fn agent_forgetting(game: Game, goals: Vec<Goal>, forget: Option<usize>) -> (NativeAgent, Rc<RefCell<Log>>) {
        let log = Rc::new(RefCell::new(Log { goals_left: goals.len(), ..Log::default() }));
        let agent = NativeAgent::new(game, Box::new(Goals { goals, log: log.clone(), forget, purchases: Vec::new() })).unwrap();
        (agent, log)
    }

    /// One field move asked of a game, run until the agent is back on the overworld.
    fn ask(game: Game, field_move: FieldMove, forget: Option<usize>) -> (NativeAgent, Vec<String>) {
        let (mut agent, log) = agent_forgetting(game, vec![Goal::Field(field_move)], forget);
        run(&mut agent, &log, settled);
        let events = log.borrow().events.clone();
        assert!(matches!(agent.game().modes(), [Mode::Overworld(_)]), "every menu closed: {events:#?}");
        (agent, events)
    }

    fn quantity(agent: &NativeAgent, item: ItemId) -> u8 {
        agent.game().world().bag.items.iter().find(|slot| slot.id == item).map_or(0, |slot| slot.quantity)
    }

    fn says(events: &[String], start: &str) -> bool {
        events.iter().any(|event| event.starts_with(&format!("TextBox {{ message: \"{start}")))
    }

    fn with_bag(items: &[(ItemId, u8)], edit: impl FnOnce(&mut World)) -> Game {
        let items = items.to_vec();
        game_at(Map::PalletTown, 5, 6, move |world| {
            for (item, quantity) in items {
                world.bag.add(item, quantity);
            }
            edit(world);
        })
    }

    fn level(species: PokemonSpecies, level: u8) -> Named<pokered::party::PartyMon> {
        let mon = new_party_mon(species, level, 1, &Origin::Trainer, &mut GameRng::tape(vec![]));
        Named { mon, ot: encode("RED").unwrap(), nick: species.name().to_vec() }
    }

    /// Ticks until `done`, or ten minutes of game time.
    fn run(agent: &mut NativeAgent, log: &Rc<RefCell<Log>>, done: impl Fn(&NativeAgent, &Log) -> bool) {
        for tick in 0.. {
            assert!(tick < 60 * 60 * 10, "ten minutes of game time, {:?} in {:?}: {:#?}", agent.game().status(),
                    agent.game().modes().iter().map(|mode| format!("{mode:?}").chars().take(300).collect::<String>()).collect::<Vec<_>>(), log.borrow().events);
            if let Err(error) = agent.tick() {
                panic!("{error} after {:#?}", log.borrow().events);
            }
            if done(agent, &log.borrow()) {
                return;
            }
        }
    }

    /// Back on the overworld with nothing left to do.
    fn settled(agent: &NativeAgent, log: &Log) -> bool {
        log.goals_left == 0 && agent.task.is_none() && agent.walk.is_none()
            && agent.game().status() == Status::Waiting(Decision::Overworld)
    }

    #[test]
    fn a_policy_walks_up_route_1_into_viridian_and_its_pokemon_centre_by_commands() {
        use Goal::*;
        let goals = vec![Talk("Girl"), Enter(Map::Route1), Enter(Map::ViridianCity), Enter(Map::ViridianPokecenter),
                         Enter(Map::ViridianCity)];
        let (mut agent, log) = agent(pallet_town(), goals);
        for tick in 0.. {
            assert!(tick < 60 * 60 * 20, "twenty minutes of game time and still walking, {:?}: {:#?}",
                    agent.game().status(), log.borrow().events);
            if let Err(error) = agent.tick() {
                panic!("{error} after {:#?}", log.borrow().events);
            }
            if log.borrow().goals_left == 0 {
                break;
            }
        }
        let log = log.borrow();
        assert!(log.events.iter().any(|e| e.starts_with("OverworldInteractionCompleted")), "the talk: {:#?}", log.events);
        assert_eq!(agent.game().world().location.map, Map::ViridianCity);
        // A battle is the one thing allowed to stop a walk, which is then taken up again.
        let aborted = log.events.iter().filter(|e| e.starts_with("OverworldActionAborted")).count();
        assert_eq!(aborted, log.battles as usize, "{:#?}", log.events);
        println!("{} wild battles on the way", log.battles);
    }

    #[test]
    fn a_walk_onto_the_water_surfs_and_rides_on_into_route_21() {
        let game = game_at(Map::PalletTown, 5, 6, |world| {
            world.badges = SOUL;
            world.party[0] = named(PokemonSpecies::Mewtwo, [None, Some(PokemonMoveName::Surf), None, None]);
        });
        let (mut agent, log) = agent(game, vec![Goal::Enter(Map::Route21)]);
        run(&mut agent, &log, settled);
        let log = log.borrow();
        assert_eq!(agent.game().world().location.map, Map::Route21, "{:#?}", log.events);
        assert!(agent.game_state().unwrap().map.surfing);
        assert!(says(&log.events, "Surfed onto the water at"), "{:#?}", log.events);
    }

    #[test]
    fn a_cut_row_walks_up_to_the_tree_and_cuts_it_down() {
        let game = game_at(Map::VermilionCity, 11, 4, |world| {
            world.badges = BOULDER | CASCADE;
            world.party[0] = named(PokemonSpecies::Mewtwo, [None, Some(PokemonMoveName::Cut), None, None]);
        });
        let cut = NativeGame::new(game.clone()).unwrap().game_state().unwrap().map.actions().into_iter()
            .find_map(|row| match row.tile { MetaTile::Cut { at } => Some(at), _ => None }).expect("a cut row");
        let (mut agent, log) = agent(game, vec![Goal::Cut]);
        run(&mut agent, &log, settled);
        let log = log.borrow();
        assert!(log.events.iter().any(|event| event.starts_with("OverworldActionCompleted { destination: Cut")),
                "{:#?}", log.events);
        let tree = agent.game_state().unwrap().map.tile_at_checked(cut);
        assert_ne!(tree, Some(MetaTile::CutTree), "the tree at {cut} is gone");
    }

    #[test]
    fn cut_with_no_tree_in_front_is_refused_and_the_menus_are_backed_out_of() {
        let game = game_at(Map::PalletTown, 5, 6, |world| {
            world.badges = BOULDER | CASCADE;
            world.party[0] = named(PokemonSpecies::Mewtwo, [None, Some(PokemonMoveName::Cut), None, None]);
        });
        let (mut agent, log) = agent(game, vec![Goal::Field(FieldMove::CutTree)]);
        run(&mut agent, &log, settled);
        assert!(says(&log.borrow().events, "the game refused Cut"), "{:#?}", log.borrow().events);
        assert!(matches!(agent.game().modes(), [Mode::Overworld(_)]), "every menu closed");
    }

    #[test]
    fn fly_takes_the_bird_to_a_visited_town() {
        let game = game_at(Map::PalletTown, 5, 6, |world| {
            world.badges = BOULDER | CASCADE | THUNDER;
            world.party.push(named(PokemonSpecies::Pidgeot, [None, Some(PokemonMoveName::Fly), None, None]));
            world.location.towns_visited = 1 << Map::PalletTown as u8 | 1 << Map::ViridianCity as u8
                | 1 << Map::PewterCity as u8;
        });
        let (mut agent, log) = agent(game, vec![Goal::Field(FieldMove::Fly { to: Map::PewterCity })]);
        run(&mut agent, &log, settled);
        let log = log.borrow();
        assert_eq!(agent.game().world().location.map, Map::PewterCity, "{:#?}", log.events);
        assert!(says(&log.events, "Flew to"), "{:#?}", log.events);
    }

    #[test]
    fn fly_to_a_town_never_visited_is_refused_before_any_menu() {
        let game = game_at(Map::PalletTown, 5, 6, |world| {
            world.badges = THUNDER;
            world.party.push(named(PokemonSpecies::Pidgeot, [None, Some(PokemonMoveName::Fly), None, None]));
            world.location.towns_visited = 1 << Map::PalletTown as u8;
        });
        let (mut agent, log) = agent(game, vec![Goal::Field(FieldMove::Fly { to: Map::CeruleanCity })]);
        run(&mut agent, &log, settled);
        assert!(says(&log.borrow().events, "Fly: CeruleanCity has never been visited"), "{:#?}", log.borrow().events);
        assert_eq!(agent.game().world().location.map, Map::PalletTown);
    }

    #[test]
    fn a_push_arms_strength_walks_behind_the_boulder_and_shoves_it() {
        let game = game_at(Map::VictoryRoad1F, 8, 16, |world| {
            world.badges = 0xFF;
            world.party[0] = named(PokemonSpecies::Mewtwo, [None, Some(PokemonMoveName::Strength), None, None]);
            // A battle on the way ends the push, as it does on the cartridge.
            world.location.repel_steps = 200;
        });
        let state = NativeGame::new(game.clone()).unwrap().game_state().unwrap();
        let boulders: Vec<Point8> = state.map.sprites.iter()
            .filter(|sprite| sprite.name.starts_with("Boulder") && !sprite.hidden).map(|sprite| sprite.position).collect();
        // A shove the map allows once Strength is on, from a square the player can reach.
        let mut armed = state.clone();
        armed.strength_active = true;
        let (boulder, dir) = boulders.iter()
            .flat_map(|&boulder| [JoypadButton::Up, JoypadButton::Down, JoypadButton::Left, JoypadButton::Right]
                .map(|dir| (boulder, dir)))
            .find(|&(boulder, dir)| armed.map.boulder_push_refusal(boulder, dir).is_none()
                && step_pos(boulder, opposite(dir)).and_then(|behind| armed.map.route_to_push_tile(behind)).is_some())
            .unwrap_or_else(|| panic!("no boulder to push among {boulders:?}"));
        let (mut agent, log) = agent(game, vec![Goal::Field(FieldMove::PushBoulder { boulder, dir })]);
        run(&mut agent, &log, settled);
        let state = agent.game_state().unwrap();
        assert!(state.strength_active, "{:#?}", log.borrow().events);
        assert!(!state.map.sprites.iter().any(|sprite| sprite.name.starts_with("Boulder") && sprite.position == boulder),
                "the boulder at {boulder} was shoved {dir:?}: {:#?}", log.borrow().events);
    }

    #[test]
    fn a_toss_throws_the_whole_stack_away() {
        let game = with_bag(&[(ItemId::Antidote, 1), (ItemId::Potion, 5)], |_| {});
        let (agent, events) = ask(game, FieldMove::TossItem { item: ItemId::Potion }, None);
        assert_eq!(quantity(&agent, ItemId::Potion), 0, "{events:#?}");
        assert_eq!(quantity(&agent, ItemId::Antidote), 1);
        assert!(says(&events, "Tossed Potion"), "{events:#?}");
    }

    #[test]
    fn a_key_item_is_too_important_to_toss_and_the_bag_is_backed_out_of() {
        let game = with_bag(&[(ItemId::SSTicket, 1)], |_| {});
        let (agent, events) = ask(game, FieldMove::TossItem { item: ItemId::SSTicket }, None);
        assert_eq!(quantity(&agent, ItemId::SSTicket), 1);
        assert!(says(&events, "the game would not toss"), "{events:#?}");
    }

    #[test]
    fn a_potion_is_used_on_the_mon_it_names() {
        let game = with_bag(&[(ItemId::Potion, 2)], |world| world.party[0].mon.mon.hp = 10);
        let target = UseTarget::Party { slot: 0, evolve: true };
        let (agent, events) = ask(game, FieldMove::UseBagItem { item: ItemId::Potion, target }, None);
        assert_eq!(agent.game().world().party[0].mon.mon.hp, 30, "{events:#?}");
        assert_eq!(quantity(&agent, ItemId::Potion), 1);
        assert!(says(&events, "Used Potion"), "{events:#?}");
    }

    #[test]
    fn an_ether_is_used_on_the_move_it_names() {
        let game = with_bag(&[(ItemId::Ether, 1)], |world| world.party[0].mon.mon.pp[1] = 0);
        let target = UseTarget::Move { slot: 0, move_index: 1 };
        let (agent, events) = ask(game, FieldMove::UseBagItem { item: ItemId::Ether, target }, None);
        assert_eq!(agent.game().world().party[0].mon.mon.pp[1] & 0x3F, 10, "{events:#?}");
        assert_eq!(quantity(&agent, ItemId::Ether), 0);
    }

    #[test]
    fn the_bicycle_skips_use_and_toss_and_is_ridden() {
        let game = with_bag(&[(ItemId::Bicycle, 1)], |_| {});
        let (agent, events) = ask(game, FieldMove::UseBagItem { item: ItemId::Bicycle, target: UseTarget::Nothing }, None);
        assert_eq!(agent.game().world().location.walk_bike_surf, 1, "{events:#?}");
    }

    fn four_moves() -> Named<pokered::party::PartyMon> {
        use PokemonMoveName::*;
        let mut mewtwo = level(PokemonSpecies::Mewtwo, 70);
        mewtwo.mon.mon.moves = [Some(Confusion), Some(Disable), Some(Swift), Some(Psychic)];
        mewtwo
    }

    #[test]
    fn an_hm_taught_to_a_mon_with_four_moves_forgets_the_one_the_policy_names() {
        let game = with_bag(&[(ItemId::Hm04Strength, 1)], |world| world.party[0] = four_moves());
        let (agent, events) = ask(game, FieldMove::TeachMove { item: ItemId::Hm04Strength, target_slot: 0 }, Some(2));
        assert_eq!(agent.game().world().party[0].mon.mon.moves[2], Some(PokemonMoveName::Strength), "{events:#?}");
        assert_eq!(quantity(&agent, ItemId::Hm04Strength), 1, "an HM is not spent");
        assert!(says(&events, "Taught Hm04Strength to party slot 0"), "{events:#?}");
    }

    #[test]
    fn declining_to_forget_a_move_ends_the_teach_unlearned() {
        let game = with_bag(&[(ItemId::Hm04Strength, 1)], |world| world.party[0] = four_moves());
        let (agent, events) = ask(game, FieldMove::TeachMove { item: ItemId::Hm04Strength, target_slot: 0 }, None);
        assert!(!agent.game().world().party[0].mon.mon.moves.contains(&Some(PokemonMoveName::Strength)), "{events:#?}");
        assert!(says(&events, "Party slot 0 did not learn"), "{events:#?}");
    }

    #[test]
    fn a_thunder_stone_evolves_a_pikachu() {
        let game = with_bag(&[(ItemId::ThunderStone, 1)], |world| world.party.push(level(PokemonSpecies::Pikachu, 20)));
        let evolve = FieldMove::EvolveWithStone { stone: ItemId::ThunderStone, target_slot: 1, evolve_from: PokemonSpecies::Pikachu };
        let (agent, events) = ask(game, evolve, None);
        assert_eq!(agent.game().world().party[1].mon.mon.species, PokemonSpecies::Raichu, "{events:#?}");
        assert!(says(&events, "Evolved party slot 1"), "{events:#?}");
    }

    #[test]
    fn a_rare_candy_told_not_to_evolve_leaves_the_mon_as_it_was() {
        let game = with_bag(&[(ItemId::RareCandy, 1)], |world| world.party[0] = level(PokemonSpecies::Charmander, 15));
        let target = UseTarget::Party { slot: 0, evolve: false };
        let (agent, events) = ask(game, FieldMove::UseBagItem { item: ItemId::RareCandy, target }, None);
        let mon = &agent.game().world().party[0].mon;
        assert_eq!((mon.mon.species, mon.level), (PokemonSpecies::Charmander, 16), "{events:#?}");
    }

    #[test]
    fn the_poke_flute_is_played_facing_the_snorlax_and_wakes_it_to_fight() {
        let game = game_at(Map::Route12, 10, 60, |world| {
            world.bag.add(ItemId::PokeFlute, 1);
        });
        let snorlax = NativeGame::new(game.clone()).unwrap().game_state().unwrap().map.sprites.iter()
            .find(|sprite| sprite.name.starts_with("Snorlax")).map(|sprite| sprite.position).expect("a Snorlax on Route 12");
        let (mut agent, log) = agent(game, vec![Goal::Field(FieldMove::UseFieldItem { item: ItemId::PokeFlute, target: snorlax })]);
        run(&mut agent, &log, |_, log| log.battles > 0);
    }

    fn pokecenter(edit: impl FnOnce(&mut World)) -> (Game, Point8) {
        let pc = crate::pokemon::tile_map::pc_locations_for(Map::ViridianPokecenter)[0];
        (game_at(Map::ViridianPokecenter, 4, 5, edit), pc)
    }

    fn boxed(species: PokemonSpecies) -> Named<pokered::party::BoxMon> {
        let named = level(species, 10);
        Named { mon: pokered::systems::add_mon::deposit(named.mon), ot: named.ot, nick: named.nick }
    }

    fn box_len(agent: &NativeAgent) -> usize {
        let world = agent.game().world();
        world.boxes.get(world.current_box as usize).map_or(0, Vec::len)
    }

    #[test]
    fn items_go_into_the_pc_as_many_as_asked() {
        let (game, pc) = pokecenter(|world| { world.bag.add(ItemId::Potion, 5); });
        let (agent, events) = ask(game, FieldMove::UseItemPc { op: PcItemOp::Deposit, item: ItemId::Potion, qty: 3, pc }, None);
        let world = agent.game().world();
        assert_eq!(quantity(&agent, ItemId::Potion), 2, "{events:#?}");
        assert_eq!(world.pc_items.items.iter().find(|slot| slot.id == ItemId::Potion).map(|slot| slot.quantity), Some(3));
        assert!(says(&events, "Deposit Potion x3 via the PC"), "{events:#?}");
    }

    #[test]
    fn items_come_out_of_the_pc_all_of_them() {
        let (game, pc) = pokecenter(|world| { world.pc_items.add(ItemId::Antidote, 4); });
        let (agent, events) = ask(game, FieldMove::UseItemPc { op: PcItemOp::Withdraw, item: ItemId::Antidote, qty: u8::MAX, pc }, None);
        assert_eq!(quantity(&agent, ItemId::Antidote), 4, "{events:#?}");
        assert!(agent.game().world().pc_items.items.iter().all(|slot| slot.id != ItemId::Antidote));
    }

    #[test]
    fn a_mon_is_deposited_in_the_box_and_withdrawn_again() {
        let (game, pc) = pokecenter(|world| world.party.push(level(PokemonSpecies::Pidgey, 5)));
        let (agent, events) = ask(game, FieldMove::UsePcBox { op: PcBoxOp::Deposit { slot: 1 }, pc }, None);
        assert_eq!((agent.game().world().party.len(), box_len(&agent)), (1, 1), "{events:#?}");
        assert!(says(&events, "PC box: Deposit"), "{events:#?}");

        let game = agent.game().clone();
        let (agent, events) = ask(game, FieldMove::UsePcBox { op: PcBoxOp::Withdraw { box_slot: 0 }, pc }, None);
        let world = agent.game().world();
        assert_eq!((world.party.len(), box_len(&agent)), (2, 0), "{events:#?}");
        assert_eq!(world.party[1].mon.mon.species, PokemonSpecies::Pidgey);
    }

    #[test]
    fn a_mon_is_released_and_the_box_changed() {
        let (game, pc) = pokecenter(|world| {
            world.boxes = vec![vec![boxed(PokemonSpecies::Rattata), boxed(PokemonSpecies::Pidgey)]];
        });
        let (agent, events) = ask(game, FieldMove::UsePcBox { op: PcBoxOp::Release { box_slot: 0 }, pc }, None);
        let world = agent.game().world();
        assert_eq!(world.boxes[0].len(), 1, "{events:#?}");
        assert_eq!(world.boxes[0][0].mon.species, PokemonSpecies::Pidgey);

        let game = agent.game().clone();
        let (agent, events) = ask(game, FieldMove::UsePcBox { op: PcBoxOp::ChangeBox { n: 2 }, pc }, None);
        assert_eq!(agent.game().world().current_box, 2, "{events:#?}");
    }

    #[test]
    fn the_last_mon_is_not_deposited_and_no_menu_is_opened() {
        let (game, pc) = pokecenter(|_| {});
        let (agent, events) = ask(game, FieldMove::UsePcBox { op: PcBoxOp::Deposit { slot: 0 }, pc }, None);
        assert!(says(&events, "PC box: Deposit { slot: 0 } not possible"), "{events:#?}");
        assert_eq!(agent.game().world().party.len(), 1);
    }

    fn pewter_mart(money: [u8; 3], edit: impl FnOnce(&mut World)) -> Game {
        game_at(Map::PewterMart, 3, 5, |world| {
            world.money = money;
            edit(world);
        })
    }

    fn money(agent: &NativeAgent) -> u32 {
        bcd(&agent.game().world().money)
    }

    #[test]
    fn talking_to_the_clerk_buys_what_the_policy_asks_one_item_after_another() {
        let log = Rc::new(RefCell::new(Log { goals_left: 1, ..Log::default() }));
        let purchases = vec![BagItem::new(ItemId::PokeBall, 3), BagItem::new(ItemId::Antidote, 2)];
        let policy = Goals { goals: vec![Goal::Talk("Clerk")], log: log.clone(), forget: None, purchases };
        let mut agent = NativeAgent::new(pewter_mart([0x00, 0x30, 0x00], |_| {}), Box::new(policy)).unwrap();
        run(&mut agent, &log, settled);
        let events = log.borrow().events.clone();
        assert_eq!((quantity(&agent, ItemId::PokeBall), quantity(&agent, ItemId::Antidote)), (3, 2), "{events:#?}");
        assert_eq!(money(&agent), 3000 - 3 * 200 - 2 * 100);
        assert!(says(&events, "Bought PokeBall x3") && says(&events, "Bought Antidote x2"), "{events:#?}");
        assert!(matches!(agent.game().modes(), [Mode::Overworld(_)]));
    }

    #[test]
    fn a_purchase_is_trimmed_to_the_wallet() {
        let log = Rc::new(RefCell::new(Log { goals_left: 1, ..Log::default() }));
        let policy = Goals { goals: vec![Goal::Talk("Clerk")], log: log.clone(), forget: None,
                             purchases: vec![BagItem::new(ItemId::Potion, 10)] };
        let mut agent = NativeAgent::new(pewter_mart([0x00, 0x07, 0x00], |_| {}), Box::new(policy)).unwrap();
        run(&mut agent, &log, settled);
        assert_eq!(quantity(&agent, ItemId::Potion), 2, "{:#?}", log.borrow().events);
        assert_eq!(money(&agent), 100);
    }

    #[test]
    fn a_sale_walks_to_the_counter_and_sells_as_many_as_asked() {
        let game = pewter_mart([0; 3], |world| { world.bag.add(ItemId::Potion, 4); });
        let state = NativeGame::new(game.clone()).unwrap().game_state().unwrap();
        let sale = crate::pokemon::postgame::game_corner::pick_sale(&state, BagItem::new(ItemId::Potion, 3)).expect("a clerk to sell to");
        let (agent, events) = ask(game, sale, None);
        assert_eq!(quantity(&agent, ItemId::Potion), 1, "{events:#?}");
        assert_eq!(money(&agent), 3 * 150);
        assert!(says(&events, "Sold"), "{events:#?}");
    }

    #[test]
    fn a_vending_machine_sells_its_cheapest_drink() {
        let game = game_at(Map::CeladonMartRoof, 10, 4, |world| world.money = [0x00, 0x10, 0x00]);
        let (agent, events) = ask(game, FieldMove::CheckTrashCan { target: Point8 { x: 10, y: 1 }, facing: None }, None);
        assert_eq!(quantity(&agent, ItemId::FreshWater), 1, "{events:#?}");
        assert_eq!(money(&agent), 1000 - 200);
    }

    #[test]
    fn an_elevator_takes_the_floor_asked_and_its_door_is_walked_out_of() {
        let game = game_at(Map::SilphCoElevator, 1, 2, |world| world.location.last_map = Map::SilphCo1F);
        let (mut agent, log) = agent(game, vec![Goal::Field(FieldMove::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 4 })]);
        run(&mut agent, &log, settled);
        assert_eq!(agent.game().world().location.map, Map::SilphCo5F, "{:#?}", log.borrow().events);
    }

    #[test]
    fn a_prize_is_bought_with_coins_and_its_nickname_kept() {
        let game = game_at(Map::GameCornerPrizeRoom, 4, 6, |world| {
            world.coins = [0x05, 0x00];
            world.bag.add(ItemId::CoinCase, 1);
        });
        let (agent, events) = ask(game, FieldMove::RedeemPrize { prize: Prize::Abra }, None);
        let world = agent.game().world();
        assert_eq!(bcd(&world.coins), 500 - 180, "{events:#?}");
        assert_eq!(world.party[1].mon.mon.species, PokemonSpecies::Abra);
        assert_eq!(world.party[1].nick, PokemonSpecies::Abra.name().to_vec(), "no nickname was given");
        assert!(says(&events, "bought Abra for 180 coins"), "{events:#?}");
    }

    #[test]
    fn the_day_care_takes_the_mon_named() {
        let game = game_at(Map::Daycare, 2, 5, |world| {
            world.money = [0x00, 0x10, 0x00];
            world.party.push(level(PokemonSpecies::Pidgey, 5));
        });
        let state = NativeGame::new(game.clone()).unwrap().game_state().unwrap();
        let deposit = crate::pokemon::postgame::gifts::pick(&state, PartyScript::Daycare, 1).expect("the Day Care man");
        let (agent, events) = ask(game, deposit, None);
        assert_eq!(agent.game().world().party.len(), 1, "{events:#?}");
        assert!(says(&events, "Daycare: party 2 to 1"), "{events:#?}");
    }

    #[test]
    fn an_in_game_trade_hands_over_the_mon_it_wants() {
        let trade = crate::pokemon::postgame::trades::trade_for(PokemonSpecies::Abra);
        let game = game_at(trade.at, 2, 5, |world| world.party.push(level(PokemonSpecies::Abra, 5)));
        let state = NativeGame::new(game.clone()).unwrap().game_state().unwrap();
        let swap = crate::pokemon::postgame::gifts::pick(&state, trade.script(), 1).expect("the trader");
        let (agent, events) = ask(game, swap, None);
        assert_eq!(agent.game().world().party[1].mon.mon.species, trade.get, "{events:#?}");
    }

    #[test]
    fn switch_brings_a_mon_to_the_front() {
        let game = game_at(Map::PalletTown, 5, 6, |world| world.party.push(level(PokemonSpecies::Pidgey, 5)));
        let (agent, events) = ask(game, FieldMove::ReorderParty { slot: 1 }, None);
        let party = &agent.game().world().party;
        assert_eq!((party[0].mon.mon.species, party[1].mon.mon.species), (PokemonSpecies::Pidgey, PokemonSpecies::Mewtwo), "{events:#?}");
        assert!(says(&events, "Moved party slot 1 to the front"), "{events:#?}");
    }

    #[test]
    fn a_rod_is_cast_at_the_water_faced() {
        let game = game_at(Map::PalletTown, 5, 6, |world| { world.bag.add(ItemId::OldRod, 1); });
        let state = NativeGame::new(game.clone()).unwrap().game_state().unwrap();
        let at = crate::pokemon::postgame::fishing::nearest_castable_water(&state.map).expect("water in Pallet Town");
        let (mut agent, log) = agent(game, vec![Goal::Field(FieldMove::Fish { rod: Rod::Old, at })]);
        run(&mut agent, &log, |agent, log| log.battles > 0 || settled(agent, log));
        let log = log.borrow();
        assert!(log.battles > 0 || says(&log.events, "You cast the"), "{:#?}", log.events);
    }

    #[test]
    fn a_grass_row_paces_in_the_grass_until_something_appears() {
        // Repelled on the way, so the battle is the pace's.
        let game = game_at(Map::Route1, 10, 30, |world| world.location.repel_steps = 1);
        let (mut agent, log) = agent(game, vec![Goal::Row(|tile| *tile == MetaTile::Grass)]);
        run(&mut agent, &log, |agent, _| matches!(agent.task, Some(Task::Pace(_))));
        run(&mut agent, &log, |_, log| log.battles > 0);
        let log = log.borrow();
        assert!(log.events.iter().any(|event| event.starts_with("OverworldActionAborted { destination: Grass, reason: Battle")),
                "a battle is what ends a pace: {:#?}", log.events);
    }

    #[test]
    fn a_vending_row_buys_the_drink_it_names() {
        let game = game_at(Map::CeladonMartRoof, 10, 4, |world| world.money = [0x00, 0x10, 0x00]);
        let (mut agent, log) = agent(game, vec![Goal::Row(|tile| matches!(tile,
            MetaTile::Switch { object: HiddenObject::VendingMachine, ordinal: 2 }))]);
        run(&mut agent, &log, settled);
        assert_eq!(quantity(&agent, ItemId::SodaPop), 1, "{:#?}", log.borrow().events);
    }

    #[test]
    fn a_boulder_goal_is_shoved_until_the_boulder_is_on_its_switch() {
        let game = game_at(Map::VictoryRoad1F, 8, 16, |world| {
            world.badges = 0xFF;
            world.party[0] = named(PokemonSpecies::Mewtwo, [None, Some(PokemonMoveName::Strength), None, None]);
            world.location.repel_steps = 250;
        });
        let state = NativeGame::new(game.clone()).unwrap().game_state().unwrap();
        let mut armed = state.clone();
        armed.strength_active = true;
        let rows: Vec<String> = armed.map.actions().iter().map(|row| format!("{:?}", row.tile)).collect();
        let (mut agent, log) = agent(game, vec![Goal::Row(|tile| matches!(tile, MetaTile::BoulderGoal { .. }))]);
        run(&mut agent, &log, settled);
        let log = log.borrow();
        assert!(log.events.iter().any(|event| event.starts_with("OverworldActionCompleted { destination: BoulderGoal")),
                "{:#?} from rows {rows:#?}", log.events);
    }
}
