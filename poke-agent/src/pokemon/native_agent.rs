//! A policy's decisions carried out on the recreation, one [`Command`] at a time.
//!
//! [`PokemonAgent`](crate::pokemon::agent::PokemonAgent) infers from memory and the screen what is
//! on the table and presses buttons until the game has moved on. The recreation says what it is
//! waiting for and takes a typed command, so this loop only has to choose the command: the same
//! [`Policy`], the same [`GameState`] and the same rows, with the pressing left to `pokered`.

use std::collections::VecDeque;

use gb::cycles::MachineCycles;
use gb::joypad::JoypadButton;
use poke_core::geometry::Point8;
use pokered::command::{Command, Decision, Refusal, Reply};
use pokered::mode::{Mode, Status};
use pokered::modes::field_move_menu::FieldMoveChoice;
use pokered::modes::pc::bills_pc::BillsPcMenu;
use pokered::modes::start_menu::StartMenuEntry;
use pokered::systems::overworld::collision;
use pokered::systems::overworld::location::Direction;
use pokered::audio::Write;
use pokered::input::Joypad;
use pokered::{Event, Game, Input};

use crate::pokemon::agent::{is_on_map_border, step_pos, AgentEvent, OverworldActionAbortedReason};
use crate::pokemon::battle::BattleAction;
use crate::pokemon::encoding::GameMode;
use crate::pokemon::map::Map;
use crate::pokemon::native::NativeGame;
use crate::pokemon::text::PokemonTextReader;
use crate::pokemon::move_name::PokemonMoveName;
use crate::pokemon::bag::BagItem;
use crate::pokemon::item::ItemId;
use crate::pokemon::agent::MANUAL_INPUT_CAPACITY;
use crate::pokemon::policy::{field_move_at, field_move_carrier, FieldMove, Jam, Policy};
use crate::pokemon::map_metadata::PlayerFacingDirection;
use crate::pokemon::postgame::item_storage::PcItemOp;
use crate::pokemon::postgame::items::{Effect, UseTarget};
use crate::pokemon::postgame::pc_box::PcBoxOp;
use crate::pokemon::postgame::fishing::Rod;
use crate::pokemon::postgame::game_corner::{Prize, SlotSession, SLOTS_LEFT_UNPLAYED};
use crate::pokemon::postgame::gifts::PartyScript;
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::tile::{HiddenObject, MetaTile};
use crate::pokemon::world_graph::WorldGraph;
use crate::pokemon::GameState;

/// A new game played from power-on through the intro, on the preset names, to Red's room, with
/// `options` in force. The intro is played at [`pokered::Pacing::Instant`]: no policy answers it.
pub fn new_game(rng: impl Fn() -> pokered::rng::GameRng, options: pokered::world::Options, pacing: pokered::Pacing) -> Result<Game, String> {
    let mut game = Game::power_on(rng(), pokered::Pacing::Instant);
    for _ in 0..60_000 {
        if matches!(game.modes(), [Mode::Overworld(_)]) {
            let mut world = game.world().clone();
            world.options = options;
            let mut game = Game::new(world, rng(), pacing);
            game.push(Mode::Overworld(pokered::modes::overworld::Overworld::new()));
            return Ok(game);
        }
        let command = match game.status() {
            Status::Waiting(Decision::TitleScreen | Decision::Text) => Some(Command::Advance),
            Status::Waiting(Decision::MainMenu) => Some(Command::ChooseOption(0)),
            Status::Waiting(Decision::IntroNameMenu) => Some(Command::ChooseOption(1)),
            _ => None,
        };
        game.frame(command.map_or(Input::None, Input::Command));
    }
    Err(format!("the intro never reached Red's room: {:?}", game.status()))
}

/// The emulated agent's bounds on a row gone missing, in frames, each of which is a decision point:
/// one for any row, and a longer one once `row_blocked_by_people` says someone stands on the route.
const MAX_ROUTE_LOST_POLLS: u32 = ticks_to_frames(crate::pokemon::agent::MAX_ROUTE_LOST_TICKS);
const MAX_ROUTE_BLOCKED_POLLS: u32 = ticks_to_frames(crate::pokemon::agent::MAX_ROUTE_BLOCKED_TICKS);

const fn ticks_to_frames(ticks: u16) -> u32 {
    (ticks as u64 * crate::pokemon::agent::AGENT_RESOLUTION.m_cycles() / MachineCycles::PER_FRAME.m_cycles()) as u32
}

/// Frames the overworld stands free before the policy is asked: a script can free the player for a
/// frame or two between its steps, as Oak's lab does before Blue speaks. The emulated agent's own
/// wait starts sooner, so a longer one here outlasts it.
const ASK_AFTER_FRAMES: u32 = crate::pokemon::delay::SHORT_DELAY_CYCLES.m_cycles()
    .div_ceil(MachineCycles::PER_FRAME.m_cycles()) as u32;

/// Talks whose menu the emulated agent's A answers with its first row: the drinks the roof's girl
/// is shown, and the fossils the lab's scientist is.
const FIRST_ROW_TALKS: [(Map, &str); 2] = [(Map::CeladonMartRoof, "Little Girl"), (Map::CinnabarLabFossilRoom, "Scientist 1")];

/// Steps a walk may take without its route getting shorter before it is given up.
const MAX_STALE_STEPS: u32 = 64;

/// Decision points a task's refused step waits for whoever stands in it to move on: longer than a
/// row's, since a person at a counter can stand there for seconds. Twenty seconds of game time.
const MAX_TASK_BLOCKED_POLLS: u32 = 1200;

/// Presses of A at one Silph door before it is taken for a wall that looks like one.
const CARD_KEY_PRESSES: usize = 3;

/// A row being walked: re-derived at every decision point, as the emulated agent re-derives it
/// every tick, so nothing depends on a route's tail.
#[derive(Debug, Clone, Copy)]
struct Walk {
    destination: MetaTile,
    map: Map,
    /// The last command was A at the row's end, so a text box is the row succeeding.
    pressed_a: bool,
    route_lost: u32,
    /// Asked once [`MAX_ROUTE_LOST_POLLS`] runs out: whether people are what stands on the route.
    route_lost_to_people: bool,
    /// The square the player last left: a step pressed on a poll offered early may never be taken.
    came_from: Option<Point8>,
    /// The shortest the row's route has been, and the steps since it last got shorter: a current
    /// or a slope can take back every step a walk makes. Counted where the player stood last.
    closest: usize,
    stale: u32,
    last_at: Option<Point8>,
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
    BikeShop(BikeShopBuy),
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

/// SWITCH on the party menu, a neighbour at a time, to bring a mon to the front with the rest
/// behind it in their order: what the emulated agent's write of the party leaves.
#[derive(Debug, Clone, Copy)]
struct SwitchUse {
    slot: u8,
    /// Where the mon being moved stands now.
    at: u8,
    opened: bool,
    /// The mon was chosen, then SWITCH, then the slot in front of it, until it leads: every party
    /// menu after is the way out.
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

/// The Bicycle chosen off the Bike Shop's menu, which the clerk refuses whatever the wallet holds;
/// reported as a mart visit once the overworld is back.
#[derive(Debug, Clone, Copy)]
struct BikeShopBuy {
    item: BagItem,
    before: u8,
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
    /// The row the next cursor menu is answered with: a vending machine's drink, or a
    /// [`FIRST_ROW_TALKS`] menu's first row.
    menu_pick: Option<u8>,
    /// A slots row is being played; `None` leaves any slot machine at the bet.
    slots: Option<SlotSession>,
    /// The bet menu waiting and what was decided when it came up: a row, or `None` to leave.
    slot_bet: Option<Option<u8>>,
    /// The boulder goal being worked, between its shoves.
    boulder_goal: Option<BoulderGoal>,
    /// Frames ticked, which is game time under any pacing.
    frames: u64,
    /// Frames in a row the overworld has waited with nothing in flight; an agent made where the
    /// player stands free counts as having waited.
    free: u32,
    /// `pick_move_to_forget`'s answer, held while its `LearnMove` is up.
    forget: Option<Option<usize>>,
    /// Accepted and not yet done or interrupted.
    running: Option<Command>,
    /// `BattleStarted` has been said and `BattleEnded` not yet.
    in_battle: bool,
    /// A battle was on the stack after the last frame, so the text before it is cut off once.
    battle_was_up: bool,
    /// Every A pressed at a card-key door, so one that never opens is given up on.
    card_key_presses: Vec<(Map, Point8)>,
    /// A walk stopped on a square (in the tile map's coordinates), and the square its
    /// step left (in the map's): the next free moment says whether the game walked the player back.
    turn_back_watch: Option<(Map, Point8, Point8)>,
    /// Squares this visit was walked back off, which a script refuses and no tile shows.
    turned_back: Vec<Point8>,
    /// Where the player stood at the last tick, for a walk's `came_from`.
    square: Option<(Map, Point8)>,
    /// The text on screen outside a battle, reported as one event once it is gone.
    text: PokemonTextReader,
    /// A talk landed on an in-game trader, and this is the species their party menu is answered
    /// with, as the emulated agent arms it: nothing else says a trade is happening.
    trade_give: Option<PokemonSpecies>,
    /// An item ball was talked to, to be looked for again once the overworld is back.
    pending_pickup: Option<&'static str>,
    /// Decision points in a row a task's walk has been refused or found no way: somebody stands
    /// in it, and moves on in a few of their own steps.
    task_blocked: u32,
    /// The next wait on Route 17 lets go of the pad, for the slope to turn the rider down.
    coast: bool,
    /// `World::hall_of_fame_teams` as last seen; `None` until the first tick seeds it, so a game
    /// loaded after its ceremony does not announce it again.
    hall_of_fame_teams: Option<u8>,
    /// Kept only for a host that asked with [`NativeAgent::hosted`], since nothing else drains it.
    outputs: Option<Outputs>,
    /// Game time since the policy was last asked to decide anything, which the watchdog reads.
    since_asked: MachineCycles,
    /// [`Policy::stuck_timeout`], read once when the agent was built.
    stuck_after: Option<MachineCycles>,
    /// `since_asked` at the last [`AgentEvent::WatchdogFired`], so each timeout reports once.
    stuck_reported_at: MachineCycles,
    /// The policy's raw presses, a frame's pad each, played ahead of anything the agent decides.
    manual: VecDeque<Joypad>,
}

/// Frames a raw press is held before a frame let go, as the emulated agent holds one for two ticks.
const MANUAL_HOLD_FRAMES: usize = 2;

/// What a host plays and publishes from the frames the agent ran.
#[derive(Default)]
struct Outputs {
    audio: Vec<Write>,
    events: Vec<AgentEvent>,
    /// What the game asked of the host's save slots, in order: the Hall of Fame's autosave.
    slot_requests: Vec<pokered::save_slots::SlotRequest>,
}

impl NativeAgent {
    pub fn new(game: Game, policy: Box<dyn Policy>) -> Result<Self, String> {
        Ok(Self {
            native: NativeGame::new(game)?,
            graph: WorldGraph::new(),
            last_map: None,
            walk: None,
            task: None,
            forget: None,
            answer_no: false,
            menu_pick: None,
            slots: None,
            slot_bet: None,
            boulder_goal: None,
            frames: 0,
            free: ASK_AFTER_FRAMES,
            running: None,
            in_battle: false,
            battle_was_up: false,
            card_key_presses: Vec::new(),
            turn_back_watch: None,
            turned_back: Vec::new(),
            square: None,
            text: PokemonTextReader::untorn(),
            trade_give: None,
            pending_pickup: None,
            task_blocked: 0,
            coast: false,
            hall_of_fame_teams: None,
            outputs: None,
            since_asked: MachineCycles::ZERO,
            stuck_after: policy.stuck_timeout().filter(|timeout| !timeout.is_zero()).map(MachineCycles::from_duration),
            stuck_reported_at: MachineCycles::ZERO,
            manual: VecDeque::new(),
            policy,
        })
    }

    /// Keep every frame's audio writes and every event for [`Self::drain_audio`] and
    /// [`Self::drain_events`].
    pub fn hosted(mut self) -> Self {
        self.outputs = Some(Outputs::default());
        self
    }

    /// The register writes of every frame run since the last drain, in order.
    pub fn drain_audio(&mut self) -> Vec<Write> {
        self.outputs.as_mut().map(|outputs| std::mem::take(&mut outputs.audio)).unwrap_or_default()
    }

    pub fn drain_events(&mut self) -> Vec<AgentEvent> {
        self.outputs.as_mut().map(|outputs| std::mem::take(&mut outputs.events)).unwrap_or_default()
    }

    /// What the game asked of the host's save slots since the last take.
    pub fn take_slot_requests(&mut self) -> Vec<pokered::save_slots::SlotRequest> {
        self.outputs.as_mut().map(|outputs| std::mem::take(&mut outputs.slot_requests)).unwrap_or_default()
    }

    /// Start `game` afresh under the same policy, which is told the run it now writes to.
    pub fn restart(&mut self, game: Game, run_dir: Option<&std::path::Path>) -> Result<(), String> {
        let placeholder: Box<dyn Policy> = Box::new(crate::pokemon::policy::RandomPolicy::seeded(0));
        let policy = std::mem::replace(&mut self.policy, placeholder);
        let hosted = self.outputs.is_some();
        *self = Self::new(game, policy)?;
        if hosted {
            self.outputs = Some(Outputs::default());
        }
        self.policy.restart(run_dir);
        Ok(())
    }

    /// `POST /api/clear`, straight through to the policy.
    pub fn clear_conversation(&mut self, run_dir: Option<&std::path::Path>) -> Result<(), String> {
        self.policy.clear_conversation(run_dir)
    }

    /// A frame the host drives itself, through the credits say, kept as the agent's own are.
    pub fn frame(&mut self, input: Input) -> pokered::Frame {
        let frame = self.play(input);
        self.check_hall_of_fame();
        frame
    }

    fn play(&mut self, input: Input) -> pokered::Frame {
        // A slot machine's texts are its commentary, which the session's report sums up.
        let slots = self.at_the_slots();
        let mut frame = self.native.game_mut().frame(input);
        if !slots && !self.at_the_slots() {
            for printed in &frame.printed {
                self.text.read(Some(crate::pokemon::native::message_box_text(printed)));
            }
            if self.printing() {
                self.text.read(self.native.message_box_text());
            }
        }
        let battle_up = self.battle_is_up();
        if battle_up && !self.battle_was_up {
            self.say_text();
        }
        self.battle_was_up = battle_up;
        if let Some(outputs) = self.outputs.as_mut() {
            outputs.audio.append(&mut frame.audio);
            outputs.slot_requests.extend(frame.slot.take());
        }
        frame
    }

    fn at_the_slots(&self) -> bool {
        self.native.game().modes().iter().any(|mode| matches!(mode, Mode::SlotMachine(_)))
    }

    /// A menu, the naming grid, a battle's own boxes and an animation's tiles are each drawn by a
    /// mode of their own, so only a printing mode's rows are a message.
    fn printing(&self) -> bool {
        matches!(self.native.game().modes().last(), Some(Mode::TextBox(_) | Mode::Evolution(_)))
    }

    /// Notice the game being beaten, and say so once: the ceremony's first frame counts the team,
    /// with the winning party still in the world.
    fn check_hall_of_fame(&mut self) {
        let teams = self.native.game().world().hall_of_fame_teams;
        match self.hall_of_fame_teams.replace(teams) {
            Some(seen) if teams > seen => {}
            _ => return,
        }
        use crate::pokemon::observe::{playtime, playtime_seconds};
        // Not `game_state`: the ceremony has replaced the overworld it needs.
        let party = self.native.party()
            .map(|party| party.iter().map(|mon| mon.nickname.to_default_string()).collect())
            .unwrap_or_default();
        self.event(AgentEvent::HallOfFame {
            teams,
            playtime: playtime(&self.native),
            playtime_seconds: playtime_seconds(&self.native),
            badges: self.native.game().world().badges,
            party,
        });
    }

    pub fn game(&self) -> &Game {
        self.native.game()
    }

    pub fn game_mut(&mut self) -> &mut Game {
        self.native.game_mut()
    }

    pub fn native(&self) -> &NativeGame {
        &self.native
    }

    /// The host has taken the screen, as a harness takes it through the credits: whatever was in
    /// flight will never report back, so it is forgotten.
    pub fn host_took_the_screen(&mut self) {
        self.running = None;
        self.walk = None;
        self.task = None;
        self.boulder_goal = None;
    }

    /// Nothing is in flight and the overworld is waiting on the player: the moment a host may
    /// edit the world without a mode or a command reading it half-changed.
    pub fn is_free(&self) -> bool {
        self.running.is_none() && self.task.is_none()
            && self.native.game().status() == Status::Waiting(Decision::Overworld)
    }

    /// A task still on its way to the square it faces, or out of a lift, as the emulated agent's
    /// drivers walk it with the pad held from step to step and from the floor menu into the first.
    fn task_walking(&self) -> bool {
        match self.task {
            Some(Task::Talk(TalkUse { opened, answered, what, .. })) =>
                !opened || answered && matches!(what, Talk::Elevator { .. }),
            Some(Task::Pc(PcUse { opened, .. }) | Task::Mart(MartVisit { opened, .. })) => !opened,
            Some(Task::Bag(BagUse { face, opened, .. })) => face.is_some() && !opened,
            _ => false,
        }
    }

    fn spinning(&self) -> bool {
        matches!(self.native.game().modes().last(), Some(Mode::Overworld(overworld)) if overworld.is_spinning())
    }

    fn poll_offered(&self) -> bool {
        matches!(self.native.game().modes().last(), Some(Mode::Overworld(overworld)) if overworld.poll_offered())
    }

    /// The game state with the squares this visit was walked back off walled.
    pub fn game_state(&self) -> Result<GameState, String> {
        let mut state = self.native.game_state()?;
        for &at in &self.turned_back {
            let index = at.x as usize + at.y as usize * state.map.width;
            if index < state.map.meta_tiles.len() {
                state.map.meta_tiles[index] = MetaTile::Obstacle;
            }
        }
        Ok(state)
    }

    pub fn policy(&self) -> &dyn Policy {
        self.policy.as_ref()
    }

    /// What the agent is doing, for a heartbeat: the command in flight, else what the game waits on.
    pub fn state_debug(&self) -> String {
        match &self.running {
            Some(command) => format!("{command:?}"),
            None => format!("{:?}", self.native.game().status()),
        }
    }

    /// One frame: the command in flight carried on, or the next one chosen and handed in. The frame
    /// is played whatever the agent makes of it, as the cartridge runs on under a failed tick.
    pub fn tick(&mut self) -> Result<(), String> {
        let before = self.native.game().frames();
        let decided = self.decide();
        if decided.is_err() && self.native.game().frames() == before {
            let input = self.idle_input();
            self.play(input);
        }
        decided
    }

    /// Raw presses to deliver ahead of the state machine, which forget whatever was in flight.
    pub fn queue_manual_input(&mut self, buttons: impl IntoIterator<Item = JoypadButton>) {
        for button in buttons {
            if self.manual.len() >= MANUAL_INPUT_CAPACITY * (MANUAL_HOLD_FRAMES + 1) { break; }
            self.manual.extend(std::iter::repeat_n(pad(button), MANUAL_HOLD_FRAMES));
            self.manual.push_back(Joypad::empty());
        }
        self.host_took_the_screen();
    }

    /// Frames of raw presses not yet played; zero means the state machine runs.
    pub fn manual_input_pending(&self) -> usize {
        self.manual.len()
    }

    /// Emulated time without the policy being asked anything, whatever it answered.
    pub fn since_last_policy_poll(&self) -> std::time::Duration {
        self.since_asked.to_duration()
    }

    /// The policy is being asked to decide, which is what the watchdog waits for.
    fn asked(&mut self) {
        self.since_asked = MachineCycles::ZERO;
        self.stuck_reported_at = MachineCycles::ZERO;
    }

    /// The watchdog: ask the policy for a nudge when nothing has asked it anything, a decision this
    /// agent has no answer for included.
    fn run_watchdog(&mut self) {
        let Some(after) = self.stuck_after.filter(|after| self.since_asked >= *after) else { return };
        let Ok(state) = self.game_state() else { return };
        let agent_state = self.state_debug();
        let stuck_for = self.since_asked.to_duration();
        if self.since_asked >= self.stuck_reported_at + after {
            self.stuck_reported_at = self.since_asked;
            self.event(AgentEvent::WatchdogFired { agent_state: agent_state.clone(), stuck_for });
        }
        self.policy.service_tools(&state, &self.native, &self.graph);
        self.policy.pick_unstick(&state, Jam { agent_state: &agent_state, stuck_for });
    }

    fn decide(&mut self) -> Result<(), String> {
        self.frames += 1;
        self.since_asked += MachineCycles::PER_FRAME;
        self.check_hall_of_fame();
        // The policy's escape hatch.
        let queued = self.policy.take_manual_input();
        if !queued.is_empty() {
            self.queue_manual_input(queued);
        }
        if let Some(held) = self.manual.pop_front() {
            self.play(Input::Buttons(held));
            return Ok(());
        }
        self.run_watchdog();
        let location = &self.native.game().world().location;
        let square = Some((location.map, Point8 { x: location.x, y: location.y }));
        if let Some((map, left)) = std::mem::replace(&mut self.square, square).filter(|&left| Some(left) != square)
            && let Some(walk) = self.walk.as_mut().filter(|walk| walk.map == map)
        {
            walk.came_from = Some(left);
        }
        self.flush_text();
        // A text the overworld opened and printed inside one frame never puts a text box on top,
        // and it stops the walk as the emulated agent's text box does.
        if self.running.is_some() && self.walk.is_some() && self.task.is_none() && self.boulder_goal.is_none() && !self.spinning()
            && self.native.game_mode() == GameMode::TextBox
        {
            self.end_walk_off_the_map()?;
        }
        if self.running.is_some() {
            // A pad's destination is arrived at even when the player only passes over it, on a pad
            // of its own, as the emulated agent sees it between two of its ticks.
            if let Some(Walk { destination: MetaTile::Warp { to_map, to_position }, .. }) = self.walk
                && let location = &self.native.game().world().location
                && (location.map, location.x, location.y) == (to_map, to_position.x, to_position.y)
            {
                self.complete();
            }
            let input = self.idle_input();
            let frame = self.play(input);
            self.finish(&frame.events);
            return Ok(());
        }
        self.notice_battle_end();
        if self.learning().is_none() {
            self.forget = None;
        }
        // A walk, or a task's walk up to what it faces, takes the poll the overworld offers before
        // it runs, and so carries on as a held direction does. Nothing else does: the poll runs the
        // map's script first.
        let status = match self.native.game().status() {
            Status::Busy if (self.walk.is_some() || self.task_walking()) && self.poll_offered() =>
                Status::Waiting(Decision::Overworld),
            status => status,
        };
        self.free = match status == Status::Waiting(Decision::Overworld) && self.walk.is_none() && self.task.is_none() {
            true => self.free.saturating_add(1),
            false => 0,
        };
        // Every decision point, as the emulated agent polls: a policy's tools are answered here.
        if matches!(status, Status::Waiting(_)) && let Ok(state) = self.game_state() {
            if status == Status::Waiting(Decision::Overworld) {
                self.note_map(&state);
            }
            self.policy.service_tools(&state, &self.native, &self.graph);
        }
        if self.task.is_some() {
            if !self.battle_is_up() {
                let command = self.task_step(status)?;
                return self.issue_or_wait(command);
            }
            // A battle out of a step behind a boulder, or the step onto the water. A boulder goal
            // outlives it, as the emulated agent's does, and is taken up again on the same floor.
            self.task = None;
        }
        // An arrow tile carries the walk on rather than taking the screen from it, as the emulated
        // agent's walk outlasts the short script.
        if self.walk.is_some() && status != Status::Waiting(Decision::Overworld) && self.boulder_goal.is_none() && !self.spinning() {
            self.end_walk_off_the_map()?;
        }
        // Said after the walk it stopped, as the emulated agent says it.
        if !self.in_battle && self.battle_is_up() {
            self.in_battle = true;
            // A trainer's text before the fight is not a script walking the player back.
            self.turn_back_watch = None;
            self.event(AgentEvent::BattleStarted);
        }
        if status != Status::Waiting(Decision::CursorMenu) {
            self.slot_bet = None;
        }
        let command = match status {
            Status::Busy | Status::Idle => None,
            Status::Waiting(Decision::Text) => Some(Command::Advance),
            Status::Waiting(Decision::TwoOption | Decision::ForgetMove) if self.learning().is_some() => self.learn_answer(&status)?,
            Status::Waiting(Decision::TwoOption) => Some(Command::ChooseOption(std::mem::take(&mut self.answer_no) as u8)),
            Status::Waiting(Decision::NamingScreen) => self.name_answer()?,
            Status::Waiting(Decision::CursorMenu) if let Some(bet) = self.slot_bet() => Some(match bet {
                Some(row) => Command::ChooseOption(row),
                None => Command::CancelOption,
            }),
            Status::Waiting(Decision::SlotWheels) => Some(Command::StopWheel),
            Status::Waiting(Decision::CursorMenu) if self.native.bike_shop_offer() => self.bike_shop_answer()?,
            Status::Waiting(Decision::CursorMenu) => Some(match self.menu_pick.take() {
                Some(row) => Command::ChooseOption(row),
                None => Command::CancelOption,
            }),
            Status::Waiting(Decision::PartyMenu) if !self.battle_is_up() && let Some(give) = self.trade_give.take() =>
                Some(self.hand_over(give)),
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
            Status::Waiting(Decision::PartyMenu) if self.battle_is_up() => match self.forced_switch()? {
                Some(slot) => Some(Command::ChooseOption(slot)),
                None => self.battle()?,
            },
            Status::Waiting(decision) => return Err(format!("no native answer yet for {decision:?}")),
        };
        self.issue_or_wait(command)
    }

    /// The slot machine's bet, when its bet menu is up: decided once each time it comes up, by the
    /// row being played or, with none, to leave, as the emulated agent decides it.
    fn slot_bet(&mut self) -> Option<Option<u8>> {
        let modes = self.native.game().modes();
        let under = modes.len().checked_sub(2).map(|below| &modes[below]);
        if !matches!(under, Some(Mode::SlotMachine(_))) {
            return None;
        }
        if let Some(bet) = self.slot_bet {
            return Some(bet);
        }
        let coins = bcd(&self.native.game().world().coins);
        if self.slots.is_none() {
            self.say(SLOTS_LEFT_UNPLAYED);
        }
        Some(*self.slot_bet.insert(self.slots.as_mut().and_then(|session| session.bet(coins))))
    }

    /// What a frame with nothing to press holds. `JoypadOverworld` coasts a rider on Route 17 south
    /// whenever no direction and neither A nor B is held, so B is held there, as the emulated agent
    /// holds it, unless the slope is wanted to turn the rider down.
    fn idle_input(&mut self) -> Input {
        let game = self.native.game();
        let slope = match game.modes().last() {
            Some(Mode::Overworld(overworld)) => overworld.on_cycling_road_slope(game.world()),
            _ => false,
        };
        match slope && !std::mem::take(&mut self.coast) {
            true => Input::Buttons(pokered::input::Joypad::B),
            false => Input::None,
        }
    }

    fn issue_or_wait(&mut self, command: Option<Command>) -> Result<(), String> {
        match command {
            None => {
                let input = self.idle_input();
                self.play(input);
                Ok(())
            }
            Some(command) => self.issue(command),
        }
    }

    fn issue(&mut self, command: Command) -> Result<(), String> {
        let frame = self.play(Input::Command(command.clone()));
        match frame.reply {
            Some(Reply::Accepted) => {
                self.task_blocked = 0;
                if let Some(walk) = self.walk.as_mut() {
                    walk.pressed_a = command == Command::Interact;
                    walk.route_lost = 0;
                    walk.route_lost_to_people = false;
                }
                self.running = Some(command);
                self.finish(&frame.events);
                Ok(())
            }
            // Whoever the walk was for stepped away between the route and the press: ask again.
            Some(Reply::Refused(Refusal::Invalid(reason))) if command == Command::Interact && self.walk.is_some() =>
                self.lose_route(&reason),
            // Someone stepped into the way between the route and the step: ask again next frame.
            Some(Reply::Refused(Refusal::Invalid(reason))) if matches!(command, Command::Step(_)) && self.walk.is_some() => {
                if let Command::Step(direction) = command && let Some(door) = self.card_key_door(direction)? {
                    self.card_key_presses.push(door);
                    self.issue(Command::Interact)?;
                    // The door's text is not what the row was for.
                    if let Some(walk) = self.walk.as_mut() {
                        walk.pressed_a = false;
                    }
                    return Ok(());
                }
                self.lose_route(&reason)
            }
            Some(Reply::Refused(Refusal::Invalid(reason))) if matches!(command, Command::Step(_)) && let Some(task) = self.task =>
                self.task_blocked(task, format!("gave up walking: {reason}")),
            Some(Reply::Refused(Refusal::Busy)) => Ok(()),
            other => Err(format!("{command:?} was not taken: {other:?}")),
        }
    }

    fn finish(&mut self, events: &[Event]) {
        for event in events {
            match event {
                Event::CommandDone(done) if Some(done) == self.running.as_ref() => {
                    self.running = None;
                    // The executor calls a press done only once something answered it, so an answer
                    // over by the time the overworld is free again was a talk with no page to wait
                    // on: Pewter's Jigglypuff sings and goes quiet.
                    if *done == Command::Interact && self.native.game().status() == Status::Waiting(Decision::Overworld)
                        && let Some(walk) = self.walk.filter(|walk| walk.pressed_a)
                    {
                        self.walk = None;
                        self.interacted(walk);
                    }
                }
                Event::CommandInterrupted { command, .. } if Some(command) == self.running.as_ref() => self.running = None,
                _ => {}
            }
        }
    }

    fn event(&mut self, event: AgentEvent) {
        if !event.is_worth_reporting() {
            return;
        }
        println!("{event:?}");
        self.policy.on_event(&event);
        if let Some(outputs) = self.outputs.as_mut() {
            outputs.events.push(event);
        }
    }

    /// Report what `play` read as one message once the overworld has the screen back, a script's
    /// included, or a battle's menu is up, as the emulated agent reports a text box or a battle's
    /// turn. A question inside a text, the nurse's HEAL/CANCEL say, is part of it, and a walk the
    /// text stopped is said to have ended first.
    fn flush_text(&mut self) {
        let game = self.native.game();
        let overworld = matches!(game.modes().last(), Some(Mode::Overworld(_))) && game.status() == Status::Busy
            && self.walk.is_none();
        if overworld || matches!(game.status(),
                     Status::Waiting(Decision::Overworld | Decision::BattleMenu | Decision::BattleMoves) | Status::Idle) {
            self.say_text();
        }
    }

    fn say_text(&mut self) {
        let message = self.text.take();
        self.event(AgentEvent::TextBox { message });
    }

    /// A map stood on for the first time since the last: before the policy's poll, which is where
    /// it takes the arrival it tells the model about.
    fn note_map(&mut self, state: &GameState) {
        if self.last_map != Some(state.map.map) {
            self.last_map = Some(state.map.map);
            self.turned_back.clear();
            let location = &self.native.game().world().location;
            self.graph.observe(state.map.map, Point8 { x: location.x, y: location.y }, &state.map);
        } else {
            let location = &self.native.game().world().location;
            self.graph.refresh(&state.map, Point8 { x: location.x, y: location.y });
        }
    }

    fn battle_is_up(&self) -> bool {
        self.native.game().modes().iter().any(|mode| matches!(mode, Mode::Battle(_)))
    }

    fn notice_battle_end(&mut self) {
        if self.in_battle && !self.battle_is_up() {
            self.in_battle = false;
            // The battle's last box is its own, as the emulated agent says it.
            self.say_text();
            self.event(AgentEvent::BattleEnded);
        }
    }

    // ---- The overworld ----

    fn overworld(&mut self) -> Result<Option<Command>, String> {
        self.trade_give = None;
        let state = self.game_state()?;
        // A ball picked up is hidden; one still showing was refused, a full bag say.
        if let Some(name) = self.pending_pickup.take()
            && state.map.sprites.iter().any(|sprite| sprite.name == name && !sprite.hidden)
        {
            self.event(AgentEvent::OverworldPickupFailed { target: MetaTile::Sprite(name) });
        }
        self.note_map(&state);
        if let Some((map, tile, came_from)) = self.turn_back_watch.take()
            && map == state.map.map && self.player_at() == Some(came_from)
        {
            self.turned_back.push(tile);
            self.event(AgentEvent::TextBox { message: format!(
                "the game walked you back off {tile}, so it is being treated as a wall until you \
                 leave {map} and come back") });
            return Ok(None);
        }
        if let Some(walk) = self.walk {
            if self.boulder_goal.is_some() && self.task.is_none() {
                if state.map.map == walk.map {
                    return self.goal_step(&state);
                }
                self.boulder_goal = None;
                self.abort(OverworldActionAbortedReason::WrongMap(state.map.map), None);
                return Ok(None);
            }
            return self.walk_on(walk, &state);
        }
        // Armed for a talk that asked nothing.
        self.menu_pick = None;
        let coins = bcd(&self.native.game().world().coins);
        if let Some(report) = self.slots.take().and_then(|session| session.report(coins)) {
            self.say(&report);
        }
        if self.free < ASK_AFTER_FRAMES {
            return Ok(None);
        }
        self.asked();
        if let Some(field_move) = self.policy.pick_field_move(&state) {
            return self.field_move(field_move, &state);
        }
        let Some(action) = self.policy.pick_overworld_action(&state, &self.graph) else { return Ok(None) };
        self.answer_no = matches!(action.tile, MetaTile::Switch { object: HiddenObject::Quiz { yes: false }, .. });
        self.menu_pick = match action.tile {
            MetaTile::Switch { object: HiddenObject::VendingMachine, ordinal } => Some(ordinal - 1),
            _ => None,
        };
        self.slots = (action.tile == MetaTile::Slots).then(SlotSession::default);
        self.event(AgentEvent::StartedOverworldAction { destination: action.tile, id: action.id() });
        let walk = Walk {
            destination: action.tile, map: action.map, pressed_a: false, route_lost: 0, route_lost_to_people: false, came_from: None,
            closest: usize::MAX, stale: 0, last_at: None,
        };
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
        if let Some(walk) = self.walk.as_mut()
            && walk.last_at.replace(map.player_position) != Some(map.player_position)
        {
            if row.route.len() < walk.closest {
                walk.closest = row.route.len();
                walk.stale = 0;
            } else {
                walk.stale += 1;
            }
            if walk.stale > MAX_STALE_STEPS {
                self.abort(OverworldActionAbortedReason::Unknown, Some(map.player_position));
                return Ok(None);
            }
        }
        match row.route.first() {
            Some(JoypadButton::A) => Ok(Some(Command::Interact)),
            Some(&button) => {
                let direction = direction(button).ok_or_else(|| format!("a route pressed {button:?}"))?;
                let onto = step_pos(map.player_position, button).and_then(|at| map.tile_at_checked(at));
                // A fishing row casts from the shore.
                if !map.surfing && !matches!(destination, MetaTile::Fish { .. })
                    && matches!(onto, Some(MetaTile::Water | MetaTile::ConnectionWater(_)))
                    && let Some((slot, _)) = field_move_carrier(state, PokemonMoveName::Surf)
                {
                    let at = step_pos(map.player_position, button);
                    return self.start_field_move(FieldMoveUse::new(slot, PokemonMoveName::Surf, Then::Report)
                        .facing(direction).at(at));
                }
                // A press against what the row is for, a tree or a person, is a turn, which a step
                // would be refused as.
                self.step_or_face(button)
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

    /// `PrintCardKeyText`'s door, faced and closed, with the Card Key to open it and not yet given
    /// up on. The doors are drawn by load code, so only the live screen has them.
    fn card_key_door(&self, direction: Direction) -> Result<Option<(Map, Point8)>, String> {
        let game = self.native.game();
        let location = &game.world().location;
        let Some(Mode::Overworld(overworld)) = game.modes().last() else { return Ok(None) };
        if !crate::pokemon::map_metadata::map_has_card_key_doors(location.map)
            || !game.world().bag.items.iter().any(|slot| slot.id == ItemId::CardKey)
            || location.facing != direction.facing()
        {
            return Ok(None);
        }
        let front = collision::in_front(&overworld.view().tile_map(), location.x, location.y, direction.facing());
        let door = front.tile == 0x18 || front.tile == 0x24 || (front.tile == 0x5e && location.map == Map::SilphCo11F);
        let ahead = (location.map, Point8 { x: front.x, y: front.y });
        let presses = self.card_key_presses.iter().filter(|&&pressed| pressed == ahead).count();
        Ok((door && presses < CARD_KEY_PRESSES).then_some(ahead))
    }

    /// A task no square on the map reaches, given up at once, as every emulated driver but the prize
    /// counter's gives it up.
    fn give_up(&mut self, why: &str) {
        self.task_blocked = 0;
        self.task = None;
        self.say(why);
    }

    /// Wait out a task's refused step, up to [`MAX_TASK_BLOCKED_POLLS`], then give the task up saying
    /// `why`. A prize counter with no route is waited on as long as the emulated agent waits.
    fn task_blocked(&mut self, task: Task, why: String) -> Result<(), String> {
        const PRIZE_BLOCKED_POLLS: u32 = (crate::pokemon::postgame::game_corner::BLOCKED_TICKS as u64
            * crate::pokemon::agent::AGENT_RESOLUTION.m_cycles() / gb::cycles::MachineCycles::PER_FRAME.m_cycles()) as u32;
        let bound = match task {
            Task::Talk(TalkUse { what: Talk::Prize(_), .. }) => PRIZE_BLOCKED_POLLS,
            _ => MAX_TASK_BLOCKED_POLLS,
        };
        self.wait_out(task, why, bound)
    }

    /// A task no square faces the target of: waited on while people are what stands in the way, as
    /// long as the emulated agent's `face_or_wait` waits, and given up at once otherwise.
    fn out_of_reach(&mut self, task: Task, people: bool, why: String) -> Result<(), String> {
        match people {
            true => self.wait_out(task, why, MAX_ROUTE_BLOCKED_POLLS),
            false => {
                self.give_up(&why);
                Ok(())
            }
        }
    }

    /// Whether a boulder push held up waits this poll: only while `people` says people are all that
    /// is in its way, and for at most [`MAX_ROUTE_BLOCKED_POLLS`] in a row, as the emulated agent's
    /// `wait_on_people` waits. A command taken starts the count over.
    fn wait_on_people(&mut self, people: impl FnOnce() -> bool) -> bool {
        let wait = self.task_blocked < MAX_ROUTE_BLOCKED_POLLS && people();
        self.task_blocked = if wait { self.task_blocked + 1 } else { 0 };
        wait
    }

    fn wait_out(&mut self, task: Task, why: String, bound: u32) -> Result<(), String> {
        self.task_blocked += 1;
        if self.task_blocked > bound {
            self.task_blocked = 0;
            self.task = None;
            self.say(&why);
        } else {
            self.task = Some(task);
        }
        Ok(())
    }

    /// Wait out a route that has gone, up to a bound, then give the walk up.
    fn lose_route(&mut self, _why: &str) -> Result<(), String> {
        let Some(mut walk) = self.walk else { return Ok(()) };
        walk.route_lost += 1;
        if walk.route_lost == MAX_ROUTE_LOST_POLLS + 1 {
            walk.route_lost_to_people = self.native.game_state()?.map.row_blocked_by_people(walk.destination);
        }
        self.walk = Some(walk);
        let bound = if walk.route_lost_to_people { MAX_ROUTE_BLOCKED_POLLS } else { MAX_ROUTE_LOST_POLLS };
        if walk.route_lost > bound {
            let at = self.player_at();
            self.abort(OverworldActionAbortedReason::NoRoute(walk.destination), at);
        }
        self.play(Input::None);
        Ok(())
    }

    /// Something took the screen from the walk: a talk the walk was for, or a battle, a script or a
    /// text box that stopped it.
    fn end_walk_off_the_map(&mut self) -> Result<(), String> {
        let Some(walk) = self.walk else { return Ok(()) };
        let state = self.native.game_state()?;
        // The landing that ends a boulder goal's walk starts the goal, even when it draws a battle
        // before the next poll, as the emulated agent's does.
        if let MetaTile::BoulderGoal { boulder, at, hole } = walk.destination && self.battle_is_up()
            && state.map.map == walk.map && state.map.actions().iter()
                .any(|row| row.tile.is_same_row_as(&walk.destination) && row.route.is_empty())
        {
            self.boulder_goal = Some(BoulderGoal { which: boulder, target: at, hole, pushes: 0, best: usize::MAX, stale: 0 });
            return Ok(());
        }
        let mode = state.mode;
        if walk.pressed_a && mode == GameMode::TextBox {
            self.walk = None;
            self.interacted(walk);
        } else {
            let at = self.player_at();
            if let (Some(at), Some(came_from)) = (at, walk.came_from) && at != came_from {
                self.turn_back_watch = Some((walk.map, state.map.player_position, came_from));
            }
            self.abort(OverworldActionAbortedReason::from_game_mode(mode), at);
        }
        Ok(())
    }

    fn player_at(&self) -> Option<Point8> {
        let location = &self.native.game().world().location;
        Some(Point8 { x: location.x, y: location.y })
    }

    /// A talk the walk was for has landed.
    fn interacted(&mut self, walk: Walk) {
        if FIRST_ROW_TALKS.iter().any(|&(map, who)| walk.map == map && walk.destination == MetaTile::Sprite(who)) {
            self.menu_pick = Some(0);
        }
        if let MetaTile::Sprite(name) = walk.destination
            && self.native.game_state().is_ok_and(|state| state.map.sprites.iter()
                .any(|sprite| sprite.name == name && sprite.picture_id == crate::pokemon::sprite::PictureId::PokeBall))
        {
            self.pending_pickup = Some(name);
        }
        if let MetaTile::Sprite(who) = walk.destination
            && let Some(trade) = crate::pokemon::postgame::trades::trade_at(walk.map, who)
        {
            self.trade_give = Some(trade.give);
        }
        self.event(AgentEvent::OverworldInteractionCompleted { target: walk.destination });
    }

    /// The trader's party menu: the mon they asked for, or backed out of rather than offer another.
    fn hand_over(&mut self, give: PokemonSpecies) -> Command {
        match self.native.game().world().party.iter().position(|named| named.mon.mon.species == give) {
            Some(slot) => {
                self.event(AgentEvent::TextBox { message: format!("handed over the {give:?} in party slot {}", slot + 1) });
                Command::ChooseOption(slot as u8)
            }
            None => {
                self.event(AgentEvent::TextBox { message: format!(
                    "the trade wants a {give:?} and there is none in the party, so nothing was handed over") });
                Command::CancelOption
            }
        }
    }

    fn complete(&mut self) {
        if let Some(walk) = self.walk.take() {
            self.event(AgentEvent::OverworldActionCompleted { destination: walk.destination });
        }
    }

    fn abort(&mut self, reason: OverworldActionAbortedReason, at: Option<Point8>) {
        // A slots row given up on plays nothing, whatever machine is later pressed.
        self.slots = None;
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
                self.task = Some(Task::Switch(SwitchUse { slot, at: slot, opened: false, stage: 0 }));
                self.task_step(self.native.game().status())
            }
            FieldMove::SellToMart { item, clerk } => {
                if let Some(why) = crate::pokemon::postgame::game_corner::sale_refusal(item, bag_quantity(self.native.game().world(), item.id)) {
                    self.say(&format!("sell: {why}"));
                    return Ok(None);
                }
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
                    Some(button) if talk.answered => self.step_or_face(button)?,
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
                    Some(&[button, ..]) => self.step_or_face(button)?,
                    None => {
                        let why = match talk.what {
                            Talk::Party { .. } => format!("party-script: can't reach the NPC at {}", talk.at),
                            Talk::Elevator { .. } => format!("Can't reach elevator panel at {}", talk.at),
                            Talk::Prize(prize) => format!("prize: can't reach the {prize:?} vendor at {}", talk.at),
                            Talk::Press => {
                                let what = state.map.tile_at_checked(talk.at)
                                    .map_or_else(|| "a square that is not on this map".to_string(), |tile| format!("{tile}"));
                                format!("Could not get next to {what} at {} to face it", talk.at)
                            }
                        };
                        match talk.what {
                            Talk::Prize(_) => self.task_blocked(Task::Talk(talk), why)?,
                            // The emulated lift's budget counts every tick, so it never waits.
                            Talk::Elevator { .. } => self.give_up(&why),
                            _ => self.out_of_reach(Task::Talk(talk), state.map.face_blocked_by_people(talk.at, talk.facing), why)?,
                        }
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
                    Some(Command::ChooseOption(switch.at))
                }
                2 => {
                    switch.at -= 1;
                    switch.stage = if switch.at == 0 { 3 } else { 0 };
                    Some(Command::ChooseOption(switch.at))
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
                // The question is said before it is asked, as the emulated agent's reader is
                // flushed when the naming takes over.
                self.say_text();
                self.asked();
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
                    Some(&[button, ..]) => self.step_or_face(button)?,
                    None => {
                        let people = state.map.face_blocked_by_people(at, Some(facing));
                        self.out_of_reach(Task::Mart(mart), people, format!("sell: can't reach the clerk at {at}"))?;
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
                    self.asked();
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

    /// The Bike Shop's menu, asked as a mart whose stock is the Bicycle. The Bicycle is chosen
    /// untrimmed, so the clerk's own refusal is what is said; anything else is CANCEL.
    fn bike_shop_answer(&mut self) -> Result<Option<Command>, String> {
        self.say_text();
        let state = self.native.game_state()?;
        self.asked();
        let Some(want) = self.policy.pick_mart_purchase(&state) else { return Ok(None) };
        Ok(Some(match want.filter(|item| item.id == ItemId::Bicycle) {
            Some(item) => {
                let before = bag_quantity(self.native.game().world(), item.id);
                self.task = Some(Task::BikeShop(BikeShopBuy { item, before }));
                Command::ChooseOption(0)
            }
            None => Command::ChooseOption(1),
        }))
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
                    Some(&[button, ..]) => self.step_or_face(button)?,
                    None => {
                        let why = match pc.job {
                            PcJob::Items { .. } => format!("Can't reach the PC at {}", pc.at),
                            PcJob::Box(_) => format!("PC box: can't reach the PC at {}", pc.at),
                        };
                        let people = state.map.face_blocked_by_people(pc.at, Some(PlayerFacingDirection::Up));
                        self.out_of_reach(Task::Pc(pc), people, why)?;
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
            // Release's question.
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
                        Some(&[button, ..]) => self.step_or_face(button)?,
                        None => {
                            let people = state.map.face_blocked_by_people(target, None);
                            self.out_of_reach(Task::Bag(bag), people, format!("Can't reach the field-item target at {target}"))?;
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
                self.asked();
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

    /// A press toward a row's target: a step, or a turn where a step would be refused. On the
    /// Cycling Road nothing turns in place, and the slope turns a rider down by itself.
    fn step_or_face(&mut self, button: JoypadButton) -> Result<Option<Command>, String> {
        let direction = direction(button).ok_or_else(|| format!("a route pressed {button:?}"))?;
        let world = self.native.game().world();
        let blocked = match self.native.game().modes().last() {
            Some(Mode::Overworld(overworld)) => overworld.step_refusal(direction, world).is_some(),
            _ => false,
        };
        let turn = blocked && world.location.facing != direction.facing();
        Ok(match (turn, world.location.map == Map::Route17) {
            (true, true) if direction == Direction::Down => {
                self.coast = true;
                None
            }
            (true, false) => Some(Command::Face(direction)),
            _ => Some(Command::Step(direction)),
        })
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
            Some(Task::BikeShop(buy)) => match status {
                Status::Waiting(Decision::Overworld) => {
                    self.task = None;
                    self.mart_report(MartWant::Buy(buy.item), buy.before);
                    Ok(None)
                }
                Status::Waiting(Decision::Text) => Ok(Some(Command::Advance)),
                Status::Busy | Status::Idle => Ok(None),
                Status::Waiting(decision) => Err(format!("the bike shop met {decision:?}")),
            },
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
            // People in the way are waited on, with the plan asked again every poll.
            None if self.wait_on_people(|| state.map.goal_blocked_by_people(goal.which, goal.target)) => {
                self.boulder_goal = Some(goal);
                Ok(None)
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
            if self.wait_on_people(|| map.push_blocked_by_people(push.boulder, push.dir)) {
                return Ok(None);
            }
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
            None if self.wait_on_people(|| map.push_blocked_by_people(push.boulder, push.dir)) => Ok(None),
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

    /// The party menu a faint opens is the agent's to answer, as it is on the emulated side: the
    /// first member still standing, and no question to the policy.
    fn forced_switch(&self) -> Result<Option<u8>, String> {
        let state = self.native.game_state()?;
        if state.battle.as_ref().is_none_or(|battle| battle.player.current_hp != 0) {
            return Ok(None);
        }
        Ok(state.pokemon.iter().position(|mon| mon.current_hp > 0).map(|slot| slot as u8))
    }

    fn battle(&mut self) -> Result<Option<Command>, String> {
        let state = self.native.game_state()?;
        self.asked();
        let Some(action) = self.policy.pick_battle_action(&state) else { return Ok(None) };
        self.event(AgentEvent::battle_action_started(&state, action));
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

/// The recreation's pad bit for a button.
fn pad(button: JoypadButton) -> Joypad {
    match button {
        JoypadButton::Up => Joypad::UP,
        JoypadButton::Down => Joypad::DOWN,
        JoypadButton::Left => Joypad::LEFT,
        JoypadButton::Right => Joypad::RIGHT,
        JoypadButton::A => Joypad::A,
        JoypadButton::B => Joypad::B,
        JoypadButton::Select => Joypad::SELECT,
        JoypadButton::Start => Joypad::START,
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
        /// Each mart stock the readout gave, priced, as it changed.
        stocks: Vec<Vec<(ItemId, Option<u32>)>>,
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

        fn pick_nickname(&mut self, species: PokemonSpecies) -> Option<Option<String>> {
            self.log.borrow_mut().events.push(format!("asked for a nickname for {species:?}"));
            Some(None)
        }

        fn pick_mart_purchase(&mut self, _state: &GameState) -> Option<Option<BagItem>> {
            Some(self.next_mart_purchase())
        }

        fn service_tools(&mut self, _state: &GameState, readout: &dyn crate::pokemon::observe::Readout, _graph: &WorldGraph) {
            let stock: Vec<_> = readout.mart_stock().into_iter().map(|item| (item, readout.price(item))).collect();
            let mut log = self.log.borrow_mut();
            if log.stocks.last() != Some(&stock) {
                log.stocks.push(stock);
            }
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

    /// The Game Corner with `coins` and, with `case`, a Coin Case, the player at `(x, y)`.
    fn game_corner(x: u8, y: u8, coins: [u8; 2], case: bool) -> Game {
        game_at(Map::GameCorner, x, y, |world| {
            world.location.facing = poke_core::sprite::SpriteFacing::Right;
            world.coins = coins;
            if case {
                world.bag.add(ItemId::CoinCase, 1);
            }
        })
    }

    fn at_a_machine(agent: &NativeAgent) -> bool {
        agent.game().modes().iter().any(|mode| matches!(mode, Mode::SlotMachine(_)))
    }

    /// The slots are a row only when `AbleToPlaySlotsCheck` would let them be played, and one row
    /// for the whole floor, whichever machine is nearest.
    #[test]
    fn the_slots_are_one_row_offered_only_with_a_coin_case_and_a_coin() {
        let slots = |game: Game| NativeGame::new(game).unwrap().game_state().unwrap().map.actions().into_iter()
            .filter(|row| row.tile == MetaTile::Slots).map(|row| row.id()).collect::<Vec<_>>();
        assert_eq!(slots(game_corner(15, 15, [0x00, 0x50], true)), ["GameCorner:Slots"]);
        assert!(slots(game_corner(15, 15, [0x00, 0x50], false)).is_empty(), "no Coin Case");
        assert!(slots(game_corner(15, 15, [0x00, 0x00], true)).is_empty(), "no coins");
    }

    /// The row walks to a machine, bets three coins a spin, stops the wheels, takes every win and
    /// leaves after its spins, saying what it did.
    #[test]
    fn a_slots_row_plays_its_spins_and_leaves_the_machine() {
        let (mut agent, log) = agent(game_corner(15, 15, [0x01, 0x00], true), vec![Goal::Row(|tile| *tile == MetaTile::Slots)]);
        let reported = |log: &Log| log.events.iter().any(|event| event.contains("at the slot machine"));
        run(&mut agent, &log, |agent, log| settled(agent, log) && reported(log));
        let events = log.borrow().events.clone();
        assert!(!at_a_machine(&agent), "{events:#?}");
        let coins = bcd(&agent.game().world().coins);
        let spins = crate::pokemon::postgame::game_corner::SLOT_SPINS;
        assert!(says(&events, &format!("played {spins} spins at the slot machine: 100 coins became {coins}")), "{events:#?}");
        assert!(!says(&events, crate::pokemon::postgame::game_corner::SLOTS_LEFT_UNPLAYED), "{events:#?}");
    }

    /// Two coins bet two: the `×3` the cursor starts on would be refused for ever.
    #[test]
    fn a_slots_row_bets_what_fewer_than_three_coins_allow() {
        let (mut agent, log) = agent(game_corner(15, 15, [0x00, 0x02], true), vec![Goal::Row(|tile| *tile == MetaTile::Slots)]);
        let held = RefCell::new(vec![2]);
        run(&mut agent, &log, |agent, log| {
            let coins = bcd(&agent.game().world().coins);
            let mut held = held.borrow_mut();
            if held.last() != Some(&coins) {
                held.push(coins);
            }
            settled(agent, log) && !at_a_machine(agent)
        });
        assert_eq!(held.borrow()[..2], [2, 0], "the first bet is both coins");
        let said = messages(&log.borrow().events).join(" | ");
        assert!(!said.contains("Not enough coins"), "{said}");
    }

    /// A machine the agent finds itself in without a slots row, as a stray A leaves it, is left at
    /// the bet with nothing spent, rather than played until the coins run out.
    #[test]
    fn a_slot_machine_nobody_chose_is_left_at_the_bet() {
        let mut game = game_corner(17, 12, [0x00, 0x10], true);
        while game.status() != Status::Waiting(Decision::Overworld) {
            game.frame(Input::None);
        }
        assert_eq!(game.frame(Input::Command(Command::Interact)).reply, Some(Reply::Accepted));
        let (mut agent, log) = agent(game, vec![]);
        run(&mut agent, &log, |agent, _| at_a_machine(agent));
        run(&mut agent, &log, |agent, _| !at_a_machine(agent) && agent.game().status() == Status::Waiting(Decision::Overworld));
        assert_eq!(agent.game().world().coins, [0x00, 0x10], "{:#?}", log.borrow().events);
        assert!(says(&log.borrow().events, crate::pokemon::postgame::game_corner::SLOTS_LEFT_UNPLAYED), "{:#?}", log.borrow().events);
    }

    /// The ceremony's count going up is the win, said once, whoever is driving the frame; a game
    /// that was already a champion when it was loaded says nothing.
    #[test]
    fn the_hall_of_fame_is_announced_once_when_the_count_moves() {
        let game = game_at(Map::PalletTown, 5, 6, |world| world.hall_of_fame_teams = 1);
        let mut agent = NativeAgent::new(game, Box::new(crate::pokemon::policy::RandomPolicy::seeded(1)))
            .unwrap()
            .hosted();
        let wins = |agent: &mut NativeAgent| agent.drain_events().into_iter()
            .filter_map(|event| match event {
                AgentEvent::HallOfFame { teams, party, .. } => Some((teams, party)),
                _ => None,
            })
            .collect::<Vec<_>>();
        agent.frame(Input::None);
        assert_eq!(wins(&mut agent), vec![], "a loaded champion was announced");

        agent.game_mut().world_mut().hall_of_fame_teams = 2;
        agent.frame(Input::None);
        assert_eq!(wins(&mut agent), vec![(2, vec!["MEWTWO".to_string()])]);
        agent.tick().unwrap();
        assert_eq!(wins(&mut agent), vec![], "the same win twice");
    }

    /// The League's own script counts the team with the ceremony already in the overworld's place,
    /// and the party it names is still the one being shown.
    #[test]
    fn the_hall_of_fame_names_the_party_the_ceremony_shows() {
        let game = game_at(Map::HallOfFame, 5, 7, |_| {});
        let mut agent = NativeAgent::new(game, Box::new(crate::pokemon::policy::RandomPolicy::seeded(1)))
            .unwrap()
            .hosted();
        let mut won = None;
        for _ in 0..20_000 {
            let input = match agent.game().status() {
                Status::Waiting(Decision::Text) => Input::Command(Command::Advance),
                _ => Input::None,
            };
            agent.frame(input);
            won = agent.drain_events().into_iter().find_map(|event| match event {
                AgentEvent::HallOfFame { teams, party, .. } => Some((teams, party)),
                _ => None,
            });
            if won.is_some() {
                break;
            }
        }
        assert!(!agent.game().modes().iter().any(|mode| matches!(mode, Mode::Overworld(_))),
                "the ceremony was counted under the overworld, not in its place");
        assert_eq!(won, Some((1, vec!["MEWTWO".to_string()])));
    }

    /// Wakes up after a second, and answers the first wake-up with B.
    #[derive(Default)]
    struct Nudges {
        jams: Rc<RefCell<Vec<std::time::Duration>>>,
        nudge: Vec<JoypadButton>,
        overworld_asks: Rc<RefCell<u32>>,
    }

    impl Policy for Nudges {
        fn name(&self) -> &'static str { "nudges" }

        fn stuck_timeout(&self) -> Option<std::time::Duration> {
            Some(std::time::Duration::from_secs(1))
        }

        fn pick_unstick(&mut self, _: &GameState, jam: Jam<'_>) {
            let mut jams = self.jams.borrow_mut();
            if jams.is_empty() {
                self.nudge = vec![JoypadButton::B];
            }
            jams.push(jam.stuck_for);
        }

        fn take_manual_input(&mut self) -> Vec<JoypadButton> {
            std::mem::take(&mut self.nudge)
        }

        fn pick_overworld_action(&mut self, _: &GameState, _: &WorldGraph) -> Option<OverworldAction> {
            *self.overworld_asks.borrow_mut() += 1;
            None
        }

        fn pick_battle_action(&mut self, _: &GameState) -> Option<BattleAction> {
            None
        }
    }

    /// A decision the agent has no answer for is the watchdog's: the game plays on, the policy is
    /// woken once the timeout has passed, and its press gets back to something the agent answers.
    #[test]
    fn the_watchdog_wakes_the_policy_at_a_decision_the_agent_cannot_answer() {
        let until = |game: &mut Game, decision: Decision| {
            for _ in 0..600 {
                if game.status() == Status::Waiting(decision.clone()) {
                    return;
                }
                game.frame(Input::None);
            }
            panic!("never reached {decision:?}: {:?}", game.status());
        };
        let mut game = pallet_town();
        game.push(Mode::PokemonMenu(pokered::modes::pokemon_menu::PokemonMenu::new()));
        until(&mut game, Decision::PartyMenu);
        game.frame(Input::Command(Command::ChooseOption(0)));
        until(&mut game, Decision::FieldMoveMenu);

        let policy = Nudges::default();
        let (jams, overworld_asks) = (policy.jams.clone(), policy.overworld_asks.clone());
        let mut agent = NativeAgent::new(game, Box::new(policy)).unwrap().hosted();
        let started = agent.game().frames();
        let (mut ticks, mut failed) = (0, 0);
        for _ in 0..3 * 60 {
            ticks += 1;
            failed += usize::from(agent.tick().is_err());
            if *overworld_asks.borrow() > 0 {
                break;
            }
        }
        assert!(failed > 0, "the agent answered the field-move menu itself");
        assert_eq!(agent.game().frames() - started, ticks, "a failed tick still plays its frame");
        let jams = jams.borrow();
        assert!(jams.first().is_some_and(|stuck| *stuck >= std::time::Duration::from_secs(1)), "never woken: {jams:?}");
        let fired = agent.drain_events().into_iter().filter(|event| matches!(event, AgentEvent::WatchdogFired { .. })).count();
        assert_eq!(fired, 1, "reported once per timeout");
        assert!(*overworld_asks.borrow() > 0, "the nudge never got back to the overworld: {:?}", agent.game().status());
    }

    /// The modes a step reads as, frame by frame, until the game waits on something again.
    fn modes_of_a_step_up(mut game: Game) -> Vec<GameMode> {
        while game.status() != Status::Waiting(Decision::Overworld) {
            game.frame(Input::None);
        }
        assert_eq!(game.frame(Input::Command(Command::Step(Direction::Up))).reply, Some(Reply::Accepted));
        let mut native = NativeGame::new(game).unwrap();
        let mut modes = Vec::new();
        for _ in 0..600 {
            modes.push(native.game_mode());
            if native.game().status() != Status::Busy && modes.len() > 1 {
                break;
            }
            native.game_mut().frame(Input::None);
        }
        modes
    }

    /// Oak stops the player at the grass, as the emulated agent reads it: his "Hey! Wait!" is a
    /// message of its own, said after the walk it stopped, though at `Instant` it opens, prints and
    /// ends inside one frame; and nothing is asked in the lab before Blue speaks, which leaves the
    /// player free for less time than the emulated agent waits before it asks.
    #[test]
    fn oak_stopping_the_player_reads_as_it_does_on_the_cartridge() {
        let game = game_at(Map::PalletTown, 10, 2, |world|
            world.events.clear(poke_core::symbols::pokered_events::EVENT_FOLLOWED_OAK_INTO_LAB));
        let (mut agent, log) = agent(game, vec![Goal::Enter(Map::Route1)]);
        run(&mut agent, &log, |_, log| log.events.iter().any(|event| event.contains("fed up with waiting")));
        let events = log.borrow().events.clone();
        assert_eq!(messages(&events)[..2], ["OAK: Hey! Wait! Don't go out!",
            "OAK: It's unsafe! Wild POKéMON live in tall grass! You need your own POKéMON for your protection. \
             I know! Here, come with me!"], "{events:#?}");
        let stopped = events.iter().position(|event| event.starts_with("OverworldActionAborted")).expect("the walk was stopped");
        let said = events.iter().position(|event| event.starts_with("TextBox")).unwrap();
        assert!(stopped < said, "{events:#?}");
    }

    /// The player walking reads as the overworld, as the emulated reading has it with nothing in
    /// `wJoyIgnore`, and a script that takes the pad reads as one.
    #[test]
    fn a_step_reads_as_the_overworld_and_a_script_as_a_script() {
        let walked = modes_of_a_step_up(pallet_town());
        assert!(walked.iter().all(|&mode| mode == GameMode::Overworld), "{walked:?}");

        let stopped = game_at(Map::PalletTown, 10, 2, |world|
            world.events.clear(poke_core::symbols::pokered_events::EVENT_FOLLOWED_OAK_INTO_LAB));
        let stopped = modes_of_a_step_up(stopped);
        assert!(stopped.contains(&GameMode::Script), "Oak's stopping the player: {stopped:?}");
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

    /// The Bike Shop's clerk talked to without a voucher, by a policy that buys `purchases`.
    fn bike_shop(purchases: Vec<BagItem>) -> (NativeAgent, Rc<RefCell<Log>>) {
        let game = game_at(Map::BikeShop, 6, 3, |world| {
            world.location.facing = poke_core::sprite::SpriteFacing::Up;
            world.money = [0x00, 0x30, 0x00];
        });
        let log = Rc::new(RefCell::new(Log { goals_left: 1, ..Log::default() }));
        let policy = Goals { goals: vec![Goal::Talk("Clerk")], log: log.clone(), forget: None, purchases };
        let mut agent = NativeAgent::new(game, Box::new(policy)).unwrap();
        run(&mut agent, &log, settled);
        (agent, log)
    }

    #[test]
    fn the_bike_shop_s_menu_reads_as_a_mart_selling_the_bicycle_for_a_million() {
        let (_, log) = bike_shop(Vec::new());
        assert_eq!(log.borrow().stocks, [vec![], vec![(ItemId::Bicycle, Some(1_000_000))], vec![]]);
    }

    #[test]
    fn buying_the_bicycle_hears_the_clerk_refuse_it() {
        let (agent, log) = bike_shop(vec![BagItem::new(ItemId::Bicycle, 1)]);
        let events = log.borrow().events.clone();
        let refused = events.iter().position(|event| event.contains("Sorry! You can't afford it!"));
        let reported = events.iter().position(|event| event.contains("Bought no Bicycle"));
        assert!(refused.is_some() && refused < reported, "{events:#?}");
        assert_eq!(quantity(&agent, ItemId::Bicycle), 0);
        assert_eq!(money(&agent), 3000);
    }

    /// CANCEL rather than B, which leaves every text after it printing at once.
    #[test]
    fn declining_the_bicycle_leaves_through_cancel() {
        let (agent, log) = bike_shop(Vec::new());
        let events = log.borrow().events.clone();
        assert!(events.iter().any(|event| event.contains("Come back again some time!")), "{events:#?}");
        assert!(!events.iter().any(|event| event.contains("afford")), "{events:#?}");
        assert!(!agent.game().world().no_text_delay);
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

    /// A square no walk reaches, for a PC, a clerk or a trader somebody has walled in.
    const OUT_OF_REACH: Point8 = Point8 { x: 0, y: 0 };

    /// A field move that cannot be done is given up within a few frames, saying `why`, as the
    /// emulated driver gives it up.
    fn given_up_at_once(game: Game, field_move: FieldMove, why: &str) -> NativeAgent {
        let (mut agent, log) = agent(game, vec![Goal::Field(field_move)]);
        let start = agent.game().frames();
        run(&mut agent, &log, settled);
        let (frames, events) = (agent.game().frames() - start, log.borrow().events.clone());
        assert!(frames < 10, "given up after {frames} frames: {events:#?}");
        assert!(says(&events, why), "{events:#?}");
        agent
    }

    #[test]
    fn a_pc_out_of_reach_is_given_up_at_once() {
        let (game, _) = pokecenter(|world| { world.bag.add(ItemId::Potion, 5); world.party.push(level(PokemonSpecies::Pidgey, 5)); });
        let deposit = FieldMove::UsePcBox { op: PcBoxOp::Deposit { slot: 1 }, pc: OUT_OF_REACH };
        let agent = given_up_at_once(game.clone(), deposit, "PC box: can't reach the PC at");
        assert_eq!(agent.game().world().party.len(), 2);
        let items = FieldMove::UseItemPc { op: PcItemOp::Deposit, item: ItemId::Potion, qty: 1, pc: OUT_OF_REACH };
        given_up_at_once(game, items, "Can't reach the PC at");
    }

    /// Celadon's Pokémon Centre with the Beauty, who wanders, standing on the one square that faces
    /// the PC. People move only on the screen around the player, so she is waited for from a few
    /// squares along the counter.
    fn beauty_in_front_of_the_pc() -> (Game, Point8) {
        let pc = crate::pokemon::tile_map::pc_locations_for(Map::CeladonPokecenter)[0];
        let below = Point8 { x: pc.x, y: pc.y + 1 };
        for x in (pc.x - 5..pc.x).rev() {
            let game = game_at(Map::CeladonPokecenter, x, below.y, |world| world.party.push(level(PokemonSpecies::Pidgey, 5)));
            let mut native = NativeGame::new(game).unwrap();
            for _ in 0..60 * 60 * 10 {
                native.game_mut().frame(Input::None);
                let state = native.game_state().unwrap();
                if state.map.sprites.iter().any(|sprite| sprite.name == "Beauty" && sprite.position == below) {
                    assert_eq!(state.map.route_to_face_dir(pc, Some(PlayerFacingDirection::Up)), None);
                    return (native.game().clone(), pc);
                }
            }
        }
        panic!("the Beauty never stood in front of the PC");
    }

    /// Someone standing where the PC is used from is waited on rather than taken for a wall.
    #[test]
    fn a_pc_someone_stands_in_front_of_is_used_once_they_move_on() {
        let (game, pc) = beauty_in_front_of_the_pc();
        let (agent, events) = ask(game, FieldMove::UsePcBox { op: PcBoxOp::Deposit { slot: 1 }, pc }, None);
        assert_eq!(agent.game().world().party.len(), 1, "{events:#?}");
        assert!(says(&events, "PC box: Deposit { slot: 1 } done"), "{events:#?}");
    }

    /// A person who never moves off the only square is waited on as long as a walk waits on people,
    /// and the PC is then given up in the words it is given up in at once.
    #[test]
    fn a_pc_someone_stands_in_front_of_for_good_is_waited_on_before_it_is_given_up() {
        let (game, _) = pokecenter(|world| world.party.push(level(PokemonSpecies::Pidgey, 5)));
        // The Cooltrainer stands for good below the counter at (4, 2).
        let counter = Point8 { x: 4, y: 2 };
        let (mut agent, log) = agent(game, vec![Goal::Field(FieldMove::UsePcBox { op: PcBoxOp::Deposit { slot: 1 }, pc: counter })]);
        let start = agent.game().frames();
        run(&mut agent, &log, settled);
        let (frames, events) = (agent.game().frames() - start, log.borrow().events.clone());
        assert!(frames > MAX_ROUTE_BLOCKED_POLLS as u64, "given up after {frames} frames: {events:#?}");
        assert!(says(&events, "PC box: can't reach the PC at (4, 2)"), "{events:#?}");
        assert_eq!(agent.game().world().party.len(), 2);
    }

    #[test]
    fn a_clerk_out_of_reach_is_given_up_at_once() {
        let game = pewter_mart([0; 3], |world| { world.bag.add(ItemId::Potion, 4); });
        let sale = FieldMove::SellToMart { item: BagItem::new(ItemId::Potion, 1), clerk: (OUT_OF_REACH, PlayerFacingDirection::Up) };
        let agent = given_up_at_once(game, sale, "sell: can't reach the clerk at");
        assert_eq!(quantity(&agent, ItemId::Potion), 4);
    }

    #[test]
    fn a_key_item_is_not_carried_to_the_clerk() {
        let game = pewter_mart([0; 3], |world| { world.bag.add(ItemId::TownMap, 1); });
        let state = NativeGame::new(game.clone()).unwrap().game_state().unwrap();
        let sale = crate::pokemon::postgame::game_corner::pick_sale(&state, BagItem::new(ItemId::TownMap, 1)).expect("a clerk to sell to");
        let agent = given_up_at_once(game, sale, "sell: can't sell TownMap: TownMap is a key item");
        assert_eq!(quantity(&agent, ItemId::TownMap), 1);
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

    fn turning(agent: &NativeAgent) -> bool {
        matches!(agent.game().modes().last(), Some(Mode::Overworld(overworld)) if overworld.is_turning())
    }

    /// A task walks up to what it faces with the pad held from step to step, and out of a lift from
    /// the floor menu into its first step, as the emulated agent's drivers do: nothing between arms
    /// a turn, which is an encounter check the cartridge never makes.
    #[test]
    fn a_lift_is_walked_up_to_and_out_of_without_a_turn() {
        let game = game_at(Map::SilphCoElevator, 1, 2, |world| world.location.last_map = Map::SilphCo1F);
        let (mut agent, log) = agent(game, vec![Goal::Field(FieldMove::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 4 })]);
        let turns = std::cell::Cell::new(0);
        run(&mut agent, &log, |agent, log| {
            let moved = agent.game().world().location.x != 1 || agent.game().world().location.y != 2;
            turns.set(turns.get() + (moved && turning(agent)) as u32);
            settled(agent, log)
        });
        assert_eq!(agent.game().world().location.map, Map::SilphCo5F, "{:#?}", log.borrow().events);
        assert_eq!(turns.get(), 0, "{:#?}", log.borrow().events);
    }

    /// A step pressed on landing on an arrow tile is refused, as the arrow is about to carry the
    /// player: the walk rides it out and goes on from where it stops.
    #[test]
    fn a_walk_rides_an_arrow_tile_it_lands_on_and_carries_on() {
        let game = game_at(Map::RocketHideoutB3F, 17, 16, |_| {});
        let (mut agent, log) = agent(game, vec![Goal::Talk("Rare Candy")]);
        run(&mut agent, &log, |agent, _| quantity(agent, ItemId::RareCandy) == 1);
        let events = &log.borrow().events;
        assert!(!events.iter().any(|event| event.starts_with("OverworldActionAborted")), "{events:#?}");
    }

    /// A prize counter somebody stands in front of is given up on after the five seconds the
    /// emulated agent waits, or the two part ways on whoever stands there.
    #[test]
    fn a_blocked_prize_counter_is_given_up_when_the_emulated_agent_gives_it_up() {
        let (mut agent, _) = agent(game_at(Map::GameCornerPrizeRoom, 4, 6, |_| {}), vec![]);
        let talk = TalkUse { at: Prize::Abra.vendor_tile(), facing: None, what: Talk::Prize(Prize::Abra), opened: false,
                             answered: false, before: TalkBefore { coins: 0, party: 1, nick: None } };
        let polls = (1..).find(|_| {
            agent.task_blocked(Task::Talk(talk), String::new()).unwrap();
            agent.task.is_none()
        });
        assert_eq!(polls, Some(299));
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
    fn a_trader_out_of_reach_is_given_up_at_once() {
        let trade = crate::pokemon::postgame::trades::trade_for(PokemonSpecies::Abra);
        let game = game_at(trade.at, 2, 5, |world| world.party.push(level(PokemonSpecies::Abra, 5)));
        let swap = FieldMove::UsePartyScript { script: trade.script(), slot: 1, npc: (OUT_OF_REACH, PlayerFacingDirection::Up) };
        let agent = given_up_at_once(game, swap, "party-script: can't reach the NPC at");
        assert_eq!(agent.game().world().party[1].mon.mon.species, PokemonSpecies::Abra);
    }

    #[test]
    fn a_trash_can_out_of_reach_is_given_up_at_once() {
        let game = game_at(Map::VermilionGym, 4, 16, |_| {});
        given_up_at_once(game, FieldMove::CheckTrashCan { target: OUT_OF_REACH, facing: None }, "Could not get next to");
    }

    #[test]
    fn an_elevator_panel_out_of_reach_is_given_up_at_once() {
        let game = game_at(Map::SilphCoElevator, 1, 2, |world| world.location.last_map = Map::SilphCo1F);
        let off_the_map = Point8 { x: 20, y: 20 };
        let agent = given_up_at_once(game, FieldMove::UseElevator { panel: off_the_map, floor: 4 }, "Can't reach elevator panel at");
        assert_eq!(agent.game().world().location.map, Map::SilphCoElevator);
    }

    #[test]
    fn a_field_item_target_out_of_reach_is_given_up_at_once() {
        let game = with_bag(&[(ItemId::PokeFlute, 1)], |_| {});
        let flute = FieldMove::UseFieldItem { item: ItemId::PokeFlute, target: OUT_OF_REACH };
        let agent = given_up_at_once(game, flute, "Can't reach the field-item target at");
        assert_eq!(quantity(&agent, ItemId::PokeFlute), 1);
    }

    #[test]
    fn switch_brings_a_mon_to_the_front_and_keeps_the_rest_in_order() {
        let game = game_at(Map::PalletTown, 5, 6, |world| {
            world.party.push(level(PokemonSpecies::Pidgey, 5));
            world.party.push(level(PokemonSpecies::Rattata, 5));
        });
        let (agent, events) = ask(game, FieldMove::ReorderParty { slot: 2 }, None);
        let party: Vec<_> = agent.game().world().party.iter().map(|named| named.mon.mon.species).collect();
        assert_eq!(party, [PokemonSpecies::Rattata, PokemonSpecies::Mewtwo, PokemonSpecies::Pidgey], "{events:#?}");
        assert!(says(&events, "Moved party slot 2 to the front"), "{events:#?}");
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

    fn messages(events: &[String]) -> Vec<&str> {
        events.iter().filter_map(|event| event.strip_prefix("TextBox { message: \"")?.strip_suffix("\" }")).collect()
    }

    /// Only what a text box printed is a message: the battle's own menus, the move list, the naming
    /// grid and the party menu's boxes each have a mode of their own, and a line a text box printed
    /// and took back within one frame is still said.
    #[test]
    fn a_message_is_what_a_text_box_printed_and_never_a_menu() {
        let game = game_at(Map::Route1, 10, 30, |world| {
            world.location.repel_steps = 1;
            world.badges = 0xFF;
        });
        let (mut agent, log) = agent(game, vec![Goal::Row(|tile| *tile == MetaTile::Grass)]);
        run(&mut agent, &log, |_, log| log.events.iter().any(|event| event == "BattleEnded"));
        let events = log.borrow().events.clone();
        let said = messages(&events).join(" | ");
        assert!(said.contains("REPEL's effect wore off."), "{said}");
        assert!(said.contains("Wild RATTATA appeared! Go! MEWTWO!"), "{said}");
        assert!(said.contains("MEWTWO used SWIFT!"), "{said}");
        assert!(!said.contains("FIGHT") && !said.contains("PSYCHIC BARRIER"), "{said}");

        let game = game_at(Map::GameCornerPrizeRoom, 4, 6, |world| {
            world.coins = [0x05, 0x00];
            world.bag.add(ItemId::CoinCase, 1);
        });
        let (_, events) = ask(game, FieldMove::RedeemPrize { prize: Prize::Abra }, None);
        let said = messages(&events).join(" | ");
        assert!(said.contains("So, you want ABRA? RED got ABRA! Do you want to give a nickname to ABRA?"), "{said}");
        assert!(!said.contains("lower case"), "{said}");

        let game = game_at(Map::PalletTown, 5, 6, |world| {
            world.party.push(level(PokemonSpecies::Pidgey, 5));
            world.party.push(level(PokemonSpecies::Rattata, 5));
        });
        let (_, events) = ask(game, FieldMove::ReorderParty { slot: 2 }, None);
        assert_eq!(messages(&events), ["Moved party slot 2 to the front"]);
    }

    /// The naming screen's question is said before the policy is asked it, as the emulated agent
    /// says a text box before the naming takes it over.
    #[test]
    fn a_nickname_is_asked_after_the_question_is_said() {
        let game = game_at(Map::GameCornerPrizeRoom, 4, 6, |world| {
            world.coins = [0x05, 0x00];
            world.bag.add(ItemId::CoinCase, 1);
        });
        let (_, events) = ask(game, FieldMove::RedeemPrize { prize: Prize::Abra }, None);
        let asked = events.iter().position(|event| event.starts_with("asked for a nickname")).expect("a nickname asked for");
        let said = events.iter().position(|event| event.contains("Do you want to give a nickname to ABRA?"));
        assert!(said.is_some_and(|said| said < asked), "{events:#?}");
    }

    #[test]
    fn a_vending_row_buys_the_drink_it_names() {
        let game = game_at(Map::CeladonMartRoof, 10, 4, |world| world.money = [0x00, 0x10, 0x00]);
        let (mut agent, log) = agent(game, vec![Goal::Row(|tile| matches!(tile,
            MetaTile::Switch { object: HiddenObject::VendingMachine, ordinal: 2 }))]);
        run(&mut agent, &log, settled);
        assert_eq!(quantity(&agent, ItemId::SodaPop), 1, "{:#?}", log.borrow().events);
    }

    /// The roof girl wanders onto the square a machine is pressed from while a turn is being
    /// decided: the row moves to another side, and the id the turn was rendered with still names it.
    #[test]
    fn a_person_in_front_of_a_vending_machine_leaves_its_row_id_alone() {
        let state = NativeGame::new(game_at(Map::CeladonMartRoof, 10, 4, |_| {})).unwrap().game_state().unwrap();
        let mut blocked = state.map.clone();
        let machine = |map: &crate::pokemon::tile_map::MetaTileMap| map.actions().into_iter()
            .find(|row| row.tile == MetaTile::Switch { object: HiddenObject::VendingMachine, ordinal: 1 })
            .map(|row| (row.destination, row.id()))
            .expect("a row for the first machine");
        let (free, id) = machine(&state.map);
        blocked.meta_tiles[free.x as usize + free.y as usize * blocked.width] = MetaTile::Sprite("Little Girl");
        let (moved, moved_id) = machine(&blocked);
        assert_ne!(free, moved, "the row is pressed from another side");
        assert_eq!(id, moved_id);
    }

    /// Victory Road 1F's first boulder, at (5, 15), with the player at (8, 16) and Strength to arm,
    /// and the Cooltrainer who stands at (7, 5) moved to (5, 16), the one square it is pushed up
    /// from: standing for good, or wandering off it when `wanders`.
    fn someone_below_the_first_boulder(wanders: bool) -> Game {
        let below = Point8 { x: 5, y: 16 };
        let game = game_at(Map::VictoryRoad1F, 8, 16, |world| {
            world.badges = 0xFF;
            world.party[0] = named(PokemonSpecies::Mewtwo, [None, Some(PokemonMoveName::Strength), None, None]);
            world.location.repel_steps = 250;
            world.events.set(poke_core::symbols::pokered_events::EVENT_BEAT_VICTORY_ROAD_1_TRAINER_0);
        });
        let mut native = NativeGame::new(game).unwrap();
        while native.game().status() != Status::Waiting(Decision::Overworld) {
            native.game_mut().frame(Input::None);
        }
        let Some(Mode::Overworld(overworld)) = native.game().modes().last() else { panic!("not on the overworld") };
        let mut sprites = *overworld.sprites();
        let cooltrainer = &mut sprites[1];
        assert_eq!((cooltrainer.map_x, cooltrainer.map_y), (7 + 4, 5 + 4), "the Cooltrainer's slot");
        (cooltrainer.map_x, cooltrainer.map_y) = (below.x + 4, below.y + 4);
        // The player's sprite is drawn at (0x40, 0x3C), and every square away is 16 pixels.
        (cooltrainer.x_pixels, cooltrainer.y_pixels) = (0x40 - 3 * 16, 0x3C);
        if wanders {
            // `WALK` and `ANY_DIR`.
            (cooltrainer.movement1, cooltrainer.movement2) = (0xFE, 0x00);
        }
        let standing = pokered::modes::overworld::Standing { destination_warp: 0xFF, check_for_180_degree_turn: 1, ..Default::default() };
        let mut game = Game::new(native.game().world().clone(), GameRng::seeded(11), Pacing::Instant);
        game.push(Mode::Overworld(Overworld::standing(sprites, overworld.num_sprites(), standing)));
        let state = NativeGame::new(game.clone()).unwrap().game_state().unwrap();
        assert_eq!(state.map.tile_at(below), MetaTile::Sprite("Cooltrainer Female"));
        game
    }

    const FIRST_BOULDER: Point8 = Point8 { x: 5, y: 15 };

    fn boulder_at(agent: &NativeAgent, at: Point8) -> bool {
        agent.native.game_state().unwrap().map.boulders().contains(&at)
    }

    /// Someone standing where a boulder is pushed from is waited on, and the push made once they
    /// wander off.
    #[test]
    fn a_boulder_someone_stands_behind_is_pushed_once_they_move_on() {
        let push = FieldMove::PushBoulder { boulder: FIRST_BOULDER, dir: JoypadButton::Up };
        let (agent, events) = ask(someone_below_the_first_boulder(true), push, None);
        assert!(boulder_at(&agent, Point8 { x: 5, y: 14 }), "{events:#?}");
    }

    /// A person who never moves off the square is waited on as long as a walk waits on people,
    /// and the push is then given up in the words it is given up in at once.
    #[test]
    fn a_boulder_someone_stands_behind_for_good_is_waited_on_before_it_is_given_up() {
        let push = FieldMove::PushBoulder { boulder: FIRST_BOULDER, dir: JoypadButton::Up };
        let (mut agent, log) = agent(someone_below_the_first_boulder(false), vec![Goal::Field(push)]);
        let start = agent.game().frames();
        run(&mut agent, &log, settled);
        let (frames, events) = (agent.game().frames() - start, log.borrow().events.clone());
        assert!(frames > MAX_ROUTE_BLOCKED_POLLS as u64, "given up after {frames} frames: {events:#?}");
        assert!(says(&events, "Boulder 1 at (5, 15) will not push up: Cooltrainer Female is standing at (5, 16)"), "{events:#?}");
        assert!(boulder_at(&agent, FIRST_BOULDER));
    }

    /// A push into a wall is given up at once, whoever else is standing about.
    #[test]
    fn a_boulder_push_nobody_can_make_is_given_up_at_once() {
        let push = FieldMove::PushBoulder { boulder: FIRST_BOULDER, dir: JoypadButton::Right };
        let agent = given_up_at_once(someone_below_the_first_boulder(false), push, "Boulder 1 at (5, 15) will not push right");
        assert!(boulder_at(&agent, FIRST_BOULDER));
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
