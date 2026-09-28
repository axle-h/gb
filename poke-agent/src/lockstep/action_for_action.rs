//! Action for action: the cartridge and the recreation re-made from where the cartridge stands, each
//! given the same decisions from there until it asks for its next overworld action, and the
//! `GameState` a policy would be shown compared there.
//!
//! A tour records a [`Segment`] per overworld action through [`Recording`]; [`replay`] plays one.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use gb::cycles::MachineCycles;
use gb::game_boy::{GameBoy, Stop};
use gb::ram::{RAM, ROM};
use poke_core::bag::BagItem;
use poke_core::battle::BattleAction;
use poke_core::move_name::{PokemonMove, PokemonMoveName};
use poke_core::species::PokemonSpecies;
use poke_core::symbols::pokered_local_labels as local;
use pokered::mode::Mode;
use pokered::rng::GameRng;
use pokered::{Game, Pacing};
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::agent::{PokemonAgent, AGENT_RESOLUTION};
use crate::pokemon::map_metadata::MapMetadataCache;
use crate::pokemon::native_agent::NativeAgent;
use crate::pokemon::observe::Readout;
use crate::pokemon::policy::{FieldMove, Jam, Policy};
use crate::pokemon::symbols::pokered_symbols as sym;
use crate::pokemon::world_graph::WorldGraph;
use crate::pokemon::{GameState, PokemonApi, PokemonApiTrait};
use super::breakpoint;
use super::game_state::{compared, differences};
use super::overworld::Cartridge;

/// One answer a policy gave. An overworld row is kept by id, so each side takes it off its own menu.
#[derive(Debug, Clone)]
pub(crate) enum Answer {
    Overworld(String),
    FieldMove(FieldMove),
    Battle(BattleAction),
    Nickname(Option<String>),
    Mart(Option<BagItem>),
    NextMart(BagItem),
    Forget(Option<usize>),
}

/// Where the cartridge stood before an overworld action, and every answer from it to the next.
#[derive(Clone)]
pub(crate) struct Segment {
    pub state: Vec<u8>,
    pub sram: Vec<u8>,
    pub answers: Vec<Answer>,
}

impl Segment {
    pub fn describe(&self) -> String {
        format!("{:?}", self.answers.first())
    }
}

/// What a [`Recording`] shares with the harness driving it.
#[derive(Default)]
pub(crate) struct Log {
    /// An action taken from the policy and not yet handed on, until the harness has saved the game.
    held: Option<Answer>,
    held_action: Option<OverworldAction>,
    released: bool,
    answers: Vec<Answer>,
    open: Option<(Vec<u8>, Vec<u8>)>,
    pub segments: Vec<Segment>,
}

impl Log {
    /// Between two agent ticks: if an action is being held back, save the game where it stands,
    /// close the segment before it and let the action go.
    pub fn after_tick(&mut self, gb: &GameBoy) {
        let Some(first) = self.held.clone() else { return };
        if self.released {
            return;
        }
        let snapshot = (gb.save_state().expect("a save state"), gb.dump_sram());
        if let Some((state, sram)) = self.open.replace(snapshot) {
            self.segments.push(Segment { state, sram, answers: std::mem::take(&mut self.answers) });
        }
        self.answers = vec![first];
        self.released = true;
    }
}

/// A policy, with each overworld action held back a tick for [`Log::after_tick`] and every answer
/// written down.
pub(crate) struct Recording {
    inner: Box<dyn Policy>,
    log: Arc<Mutex<Log>>,
}

impl Recording {
    pub fn new(inner: Box<dyn Policy>, log: Arc<Mutex<Log>>) -> Self {
        Self { inner, log }
    }

    fn note(&self, answer: Answer) {
        self.log.lock().unwrap().answers.push(answer);
    }
}

impl Policy for Recording {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn pick_overworld_action(&mut self, state: &GameState, graph: &WorldGraph) -> Option<OverworldAction> {
        let mut log = self.log.lock().unwrap();
        if log.held.is_some() {
            if log.released && matches!(log.held, Some(Answer::Overworld(_))) {
                log.held = None;
                log.released = false;
                return log.held_action.take();
            }
            return None;
        }
        drop(log);
        let action = self.inner.pick_overworld_action(state, graph)?;
        let mut log = self.log.lock().unwrap();
        log.held = Some(Answer::Overworld(action.id()));
        log.held_action = Some(action);
        None
    }

    fn pick_field_move(&mut self, state: &GameState) -> Option<FieldMove> {
        let mut log = self.log.lock().unwrap();
        if log.held.is_some() {
            if let (true, Some(Answer::FieldMove(field_move))) = (log.released, log.held.clone()) {
                log.held = None;
                log.released = false;
                return Some(field_move);
            }
            return None;
        }
        drop(log);
        let field_move = self.inner.pick_field_move(state)?;
        self.log.lock().unwrap().held = Some(Answer::FieldMove(field_move));
        None
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        let action = self.inner.pick_battle_action(state)?;
        self.note(Answer::Battle(action.clone()));
        Some(action)
    }

    fn player_name(&self) -> Option<String> {
        self.inner.player_name()
    }

    fn pick_nickname(&mut self, species: PokemonSpecies) -> Option<Option<String>> {
        let name = self.inner.pick_nickname(species)?;
        self.note(Answer::Nickname(name.clone()));
        Some(name)
    }

    fn pick_mart_purchase(&mut self, state: &GameState) -> Option<Option<BagItem>> {
        let item = self.inner.pick_mart_purchase(state)?;
        self.note(Answer::Mart(item));
        Some(item)
    }

    fn next_mart_purchase(&mut self) -> Option<BagItem> {
        let item = self.inner.next_mart_purchase()?;
        self.note(Answer::NextMart(item));
        Some(item)
    }

    fn pick_move_to_forget(&mut self, party_slot: usize, current_moves: &[PokemonMove], new_move: PokemonMoveName)
        -> Option<Option<usize>>
    {
        let forget = self.inner.pick_move_to_forget(party_slot, current_moves, new_move)?;
        self.note(Answer::Forget(forget));
        Some(forget)
    }

    fn on_event(&mut self, event: &crate::pokemon::agent::AgentEvent) {
        self.inner.on_event(event)
    }

    fn service_tools(&mut self, state: &GameState, readout: &dyn Readout, graph: &WorldGraph) {
        self.inner.service_tools(state, readout, graph)
    }

    fn stuck_timeout(&self) -> Option<std::time::Duration> {
        self.inner.stuck_timeout()
    }

    fn pick_unstick(&mut self, state: &GameState, jam: Jam<'_>) {
        self.inner.pick_unstick(state, jam)
    }

    fn restart(&mut self, run_dir: Option<&std::path::Path>) {
        self.inner.restart(run_dir)
    }

    fn take_manual_input(&mut self) -> Vec<gb::joypad::JoypadButton> {
        self.inner.take_manual_input()
    }

    fn is_exhausted(&self) -> bool {
        self.inner.is_exhausted()
    }
}

/// A segment's answers handed out again, each kind in its own order, until the overworld asks for
/// more than there were.
struct ReplayPolicy {
    overworld: VecDeque<Answer>,
    rest: VecDeque<Answer>,
    done: Arc<AtomicBool>,
    missing: Arc<Mutex<Option<String>>>,
    /// While set, the overworld's first ask is answered with nothing and `asked` raised.
    hold: Arc<AtomicBool>,
    asked: Arc<AtomicBool>,
}

impl ReplayPolicy {
    fn new(answers: &[Answer]) -> (Self, Arc<AtomicBool>, Arc<Mutex<Option<String>>>) {
        let (overworld, rest) = answers.iter().cloned()
            .partition(|answer| matches!(answer, Answer::Overworld(_) | Answer::FieldMove(_)));
        let (done, missing) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(None)));
        let policy = Self {
            overworld, rest, done: Arc::clone(&done), missing: Arc::clone(&missing),
            hold: Arc::new(AtomicBool::new(false)), asked: Arc::new(AtomicBool::new(false)),
        };
        (policy, done, missing)
    }

    /// Whether the first ask is still being held back, noting that it came.
    fn holding(&self) -> bool {
        let holding = self.hold.load(Ordering::Relaxed);
        if holding {
            self.asked.store(true, Ordering::Relaxed);
        }
        holding
    }

    fn take<T>(&mut self, pick: impl Fn(&Answer) -> Option<T>) -> Option<T> {
        let at = self.rest.iter().position(|answer| pick(answer).is_some())?;
        pick(&self.rest.remove(at)?)
    }
}

impl Policy for ReplayPolicy {
    fn name(&self) -> &'static str {
        "replay"
    }

    fn pick_overworld_action(&mut self, state: &GameState, _graph: &WorldGraph) -> Option<OverworldAction> {
        if self.holding() {
            return None;
        }
        match self.overworld.front() {
            None => {
                self.done.store(true, Ordering::Relaxed);
                None
            }
            Some(Answer::Overworld(id)) => {
                let id = id.clone();
                self.overworld.pop_front();
                let row = state.map.actions().into_iter().find(|action| action.id() == id);
                if row.is_none() {
                    *self.missing.lock().unwrap() = Some(format!("no row {id} on this side's menu"));
                    self.done.store(true, Ordering::Relaxed);
                }
                row
            }
            Some(_) => None,
        }
    }

    fn pick_field_move(&mut self, _state: &GameState) -> Option<FieldMove> {
        if self.holding() {
            return None;
        }
        match self.overworld.front() {
            Some(&Answer::FieldMove(field_move)) => {
                self.overworld.pop_front();
                Some(field_move)
            }
            _ => None,
        }
    }

    /// The logged answer, or with none left the first move that can be used, as the tour's own
    /// brain fights, and in the Safari Zone a run.
    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        self.take(|answer| match answer { Answer::Battle(action) => Some(action.clone()), _ => None })
            .or_else(|| {
                let battle = state.battle.as_ref()?;
                if battle.battle_type == poke_core::battle::BattleType::Safari {
                    return Some(BattleAction::Run);
                }
                battle.player.available_battle_moves().into_iter().next()
            })
    }

    /// The logged answer, or with none left no nickname: a fixture's row logs nothing past itself.
    fn pick_nickname(&mut self, _species: PokemonSpecies) -> Option<Option<String>> {
        Some(self.take(|answer| match answer { Answer::Nickname(name) => Some(name.clone()), _ => None }).flatten())
    }

    /// The logged answer, or with none left nothing bought.
    fn pick_mart_purchase(&mut self, _state: &GameState) -> Option<Option<BagItem>> {
        Some(self.take(|answer| match answer { Answer::Mart(item) => Some(*item), _ => None }).flatten())
    }

    fn next_mart_purchase(&mut self) -> Option<BagItem> {
        self.take(|answer| match answer { Answer::NextMart(item) => Some(*item), _ => None })
    }

    /// The logged answer, or with none left the new move not learned.
    fn pick_move_to_forget(&mut self, _slot: usize, _moves: &[PokemonMove], _new: PokemonMoveName) -> Option<Option<usize>> {
        Some(self.take(|answer| match answer { Answer::Forget(forget) => Some(*forget), _ => None }).flatten())
    }
}

/// The emulator run as [`GameBoy::run`] runs it, with every random byte the game's logic draws
/// written down in the order the recreation draws them: `Random`'s return to every caller but
/// VBlank, which only stirs the bytes, and the two the wild-encounter check reads for itself.
#[derive(Default)]
struct Tape {
    bytes: Vec<u8>,
    /// The people's, which `UpdateNPCSprite` and the routines after it draw.
    wander: Vec<u8>,
    frames: u64,
    /// The frame the overworld began polling with nothing held and nothing moving, unbroken since.
    idle_since: Option<u64>,
    last_idle_poll: u64,
}

/// A read of `hRandomAdd` or `hRandomSub` with no `call Random` before it, which the recreation takes
/// as a draw of its own.
struct Read {
    at: gb::game_boy::Breakpoint,
    bytes: &'static [u16],
}

/// The `ldh a, [byte]` first after `label`.
fn ldh_after(label: crate::pokemon::symbols::DmgPointer, byte: u16) -> gb::game_boy::Breakpoint {
    let mut at = breakpoint(label);
    let rom = crate::pokemon::roms::POKERED;
    let offset = |address: u16| if address < 0x4000 { address as usize }
        else { at.bank as usize * 0x4000 + (address - 0x4000) as usize };
    while rom[offset(at.address)..][..2] != [0xF0, byte as u8] {
        at.address += 1;
        assert!(at.address < label.address + 0x80, "no ldh a, [${byte:04X}] after {label}");
    }
    at
}

fn reads() -> Vec<Read> {
    let (add, sub) = (sym::hRandomAdd.address, sym::hRandomSub.address);
    vec![
        // The encounter check reads both, and the recreation takes both whether it needs the second.
        Read { at: ldh_after(local::TryDoWildEncounter::CanEncounter, add), bytes: &[0xFFD3, 0xFFD4] },
        Read { at: ldh_after(sym::SSAnneKitchenCook7Text, add), bytes: &[0xFFD3] },
        Read { at: ldh_after(sym::CeruleanCityCooltrainerF1Text, add), bytes: &[0xFFD3] },
        Read { at: ldh_after(sym::CeruleanCitySlowbroText, add), bytes: &[0xFFD3] },
        // After a `call Random`, whose own byte is `hRandomAdd`.
        Read { at: ldh_after(sym::VermilionCity_Script, sub), bytes: &[0xFFD4] },
    ]
}

impl Tape {
    fn take(&mut self, gb: &GameBoy, read: &Read) {
        let mmu = gb.core().mmu();
        if std::env::var_os("GB_A4A_DEBUG").is_some() {
            println!("[a4a] cartridge read at ({}, {}) facing {} direction {}", mmu.read(sym::wXCoord.address), mmu.read(sym::wYCoord.address),
                     mmu.read(sym::wSpriteStateData1.address + 9), mmu.read(sym::wPlayerDirection.address));
        }
        self.bytes.extend(read.bytes.iter().map(|&at| mmu.read(at)));
    }

    /// How many frames the overworld has stood idle, polling with nothing held, up to now; `None`
    /// when it is not polling at all.
    fn idle_frames(&self) -> Option<u64> {
        self.idle_since.filter(|_| self.frames - self.last_idle_poll <= 2).map(|since| self.frames - since)
    }

    fn run(&mut self, gb: &mut GameBoy, budget: MachineCycles) -> MachineCycles {
        assert_eq!((sym::hRandomAdd.address, sym::hRandomSub.address), (0xFFD3, 0xFFD4));
        let random = breakpoint(sym::Random);
        let idle_poll = breakpoint(local::OverworldLoopLessDelay::noDirectionButtonsPressed);
        let reads = reads();
        let mut points = vec![random, idle_poll];
        points.extend(reads.iter().map(|read| read.at));
        let vblank = sym::VBlank.address..sym::DelayFrame.address;
        let people = sym::UpdateNPCSprite.address..sym::DoScriptedNPCMovement.address;
        let mut ran = MachineCycles::ZERO;
        while ran < budget {
            let (stop, cycles) = gb.run_until(&points, budget - ran);
            ran += cycles;
            match stop {
                Stop::Breakpoint(hit) if hit == idle_poll => {
                    let mmu = gb.core().mmu();
                    // Nothing ignored, as the recreation's overworld is only free to poll with none.
                    let still = mmu.read(sym::wWalkCounter.address) == 0
                        && mmu.read(sym::wStatusFlags5.address) & (1 << 7 | 1 << 5) == 0
                        && mmu.read(sym::wJoyIgnore.address) == 0;
                    if !still {
                        self.idle_since = None;
                    } else if self.idle_since.is_none() || self.frames - self.last_idle_poll > 2 {
                        self.idle_since = Some(self.frames);
                    }
                    self.last_idle_poll = self.frames;
                }
                Stop::Breakpoint(hit) if hit == random => {
                    let from = gb.return_address();
                    let wandering = people.contains(&from)
                        && gb.core().mmu().rom_bank() as u8 == sym::UpdateNPCSprite.bank.id();
                    if vblank.contains(&from) {
                        self.frames += 1;
                    } else {
                        let (stop, cycles) = gb.run_to_return(MachineCycles::PER_FRAME);
                        assert!(matches!(stop, Stop::Returned { .. }), "Random did not return: {stop:?}");
                        ran += cycles;
                        let byte = gb.core().registers().a;
                        if std::env::var_os("GB_A4A_DEBUG").is_some() && !wandering {
                            println!("[a4a] cartridge Random from {:02X}:{from:04X}", gb.core().mmu().rom_bank());
                        }
                        if std::env::var_os("GB_A4A_DEBUG").is_some() && wandering {
                            println!("[a4a] cartridge people draw {} for sprite {:02X} on frame {}", self.wander.len(),
                                     gb.core().mmu().read(sym::hCurrentSpriteOffset.address) >> 4, self.frames);
                        }
                        if wandering { self.wander.push(byte) } else { self.bytes.push(byte) }
                        // A read the call returns onto: running on from here would pass its breakpoint.
                        let bank = if from < 0x4000 { 0 } else { gb.core().mmu().rom_bank() as u8 };
                        if let Some(read) = reads.iter().find(|read| read.at == gb::game_boy::Breakpoint::new(bank, from)) {
                            self.take(gb, read);
                        }
                    }
                }
                Stop::Breakpoint(hit) => {
                    let read = reads.iter().find(|read| read.at == hit).expect("one of the reads");
                    self.take(gb, read);
                }
                Stop::Budget => {}
                stop => panic!("{stop:?}"),
            }
        }
        ran
    }
}

/// How long one side may take to come back to the overworld.
const BUDGET_SECS: u64 = 600;
/// How long the cartridge stands on, drawing for the people who wander, once its agent has asked:
/// the recreation's agent may take longer to ask, and its people draw meanwhile.
const STAND_ON_SECS: u64 = 10;

/// Every person who wanders made to stand, on the cartridge before it is bridged. Where a wanderer
/// stands depends on how many frames a run has taken, and the two agents pace a walk differently, so
/// a wanderer re-routes the walk, the walk's steps move the encounter draws, and nothing after that
/// compares. Wandering itself is the overworld locksteps', frame for frame.
fn stand_still(gb: &mut GameBoy) {
    const MOVEMENT_BYTE_1: u16 = 6;
    const WALK: u8 = 0xFE;
    const STAY: u8 = 0xFF;
    for slot in 1..16u16 {
        let at = sym::wSpriteStateData2.address + slot * 16 + MOVEMENT_BYTE_1;
        if gb.core().mmu().read(at) == WALK {
            gb.core_mut().mmu_mut().write(at, STAY);
        }
    }
}

/// The cartridge and the recreation from where the segment starts, the one after the other, each
/// through its own agent, to the next overworld decision. `Err` names what differs.
pub(crate) fn replay(segment: &Segment) -> Result<(), String> {
    let mut cartridge = Cartridge::from_state(&segment.state);
    cartridge.gb.restore_sram(&segment.sram).unwrap();
    // Before the menu is read as well as before the bridge: a map a script loads meanwhile has people
    // of its own.
    stand_still(&mut cartridge.gb);

    let (policy, done, missing) = ReplayPolicy::new(&segment.answers);
    let (hold, asked) = (Arc::clone(&policy.hold), Arc::clone(&policy.asked));
    let mut agent = PokemonAgent::new(Box::new(policy));
    let mut cache = MapMetadataCache::default();
    // The emulated agent waits a while in the overworld before it asks, and can ask while a script
    // still holds the player, and the native one does neither: the recreation is made where the
    // cartridge stands once its agent has asked and the overworld polls free, at its next poll.
    hold.store(true, Ordering::Relaxed);
    let (mut settling, mut waited) = (Tape::default(), MachineCycles::ZERO);
    while !asked.load(Ordering::Relaxed) || settling.idle_frames().is_none() {
        if waited > MachineCycles::from_duration(std::time::Duration::from_secs(BUDGET_SECS)) {
            return Err("the cartridge never asked for its first overworld action".into());
        }
        let slice = settling.run(&mut cartridge.gb, AGENT_RESOLUTION);
        waited += slice;
        let mut api = PokemonApi::with_cache(&mut cartridge.gb, &mut cache);
        api.debug_set_options(&crate::pokemon::options::HEADLESS_OPTIONS);
        agent.update(&mut api, slice)?;
    }
    if !(0..600).any(|_| cartridge.frame()) {
        return Err("the cartridge asked and never polled again".into());
    }
    stand_still(&mut cartridge.gb);
    hold.store(false, Ordering::Relaxed);
    let (world, overworld) = (super::bridge::world(&cartridge.gb), super::bridge::overworld(&cartridge.gb));
    let mut gb = cartridge.gb;

    let mut tape = Tape::default();
    let mut ran = MachineCycles::ZERO;
    while !done.load(Ordering::Relaxed) {
        if ran > MachineCycles::from_duration(std::time::Duration::from_secs(BUDGET_SECS)) {
            return Err(format!("the cartridge never asked for its next overworld action: {:?}",
                               PokemonApi::with_cache(&mut gb, &mut cache).on_screen_text(false)));
        }
        let slice = tape.run(&mut gb, AGENT_RESOLUTION);
        ran += slice;
        if std::env::var_os("GB_A4A_TRACE").is_some() {
            let mmu = gb.core().mmu();
            println!("[a4a] cartridge at ({}, {}) map {} held {:02X} walk {}", mmu.read(sym::wXCoord.address), mmu.read(sym::wYCoord.address),
                     mmu.read(sym::wCurMap.address), mmu.read(sym::hJoyHeld.address), mmu.read(sym::wWalkCounter.address));
        }
        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        api.debug_set_options(&crate::pokemon::options::HEADLESS_OPTIONS);
        agent.update(&mut api, slice)?;
    }
    if let Some(missing) = missing.lock().unwrap().take() {
        return Err(format!("the cartridge: {missing}"));
    }
    // The emulated agent can ask while a map script still holds the player, waiting out a sound or
    // with the pad ignored, which it cannot tell from the overworld: the comparison is taken once the
    // overworld polls free.
    while tape.idle_frames().is_none() {
        if ran > MachineCycles::from_duration(std::time::Duration::from_secs(BUDGET_SECS)) {
            return Err("the cartridge never polled in the overworld after its agent asked".into());
        }
        let slice = tape.run(&mut gb, AGENT_RESOLUTION);
        ran += slice;
        let mut api = PokemonApi::with_cache(&mut gb, &mut cache);
        api.debug_set_options(&crate::pokemon::options::HEADLESS_OPTIONS);
        agent.update(&mut api, slice)?;
    }
    // What the agent shows a policy, trees it cut on this visit cleared, as the native agent's is.
    let theirs = compared(&agent.observe_state(&PokemonApi::with_cache(&mut gb, &mut cache))?);
    let asked = tape.bytes.len();
    let idle = tape.idle_frames().unwrap_or(0);
    tape.run(&mut gb, MachineCycles::from_duration(std::time::Duration::from_secs(STAND_ON_SECS)));

    let (drawn, people) = (tape.bytes.len(), tape.wander.len());
    if std::env::var_os("GB_A4A_DEBUG").is_some() {
        println!("[a4a] {} cartridge tape {:?}", segment.describe(), tape.bytes);
    }
    let mut game = Game::new(world, GameRng::split(tape.bytes, tape.wander), Pacing::Faithful);
    game.push(Mode::Overworld(overworld));
    let (policy, done, missing) = ReplayPolicy::new(&segment.answers);
    let mut agent = NativeAgent::new(game, Box::new(policy))?;
    // A tape that runs out is the recreation drawing more than the cartridge did: a divergence.
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut idled = 0;
        for frame in 0..BUDGET_SECS * 60 {
            // As long idle as the cartridge stood before its agent asked.
            if done.load(Ordering::Relaxed) {
                if idled >= idle {
                    return Ok(agent);
                }
                idled += 1;
            }
            let (before, wandered) = (agent.game().rng().drawn(), agent.game().rng().wandered());
            if std::env::var_os("GB_A4A_TRACE").is_some() {
                let location = &agent.game().world().location;
                println!("[a4a] frame {frame} at ({}, {}) {:?} status {:?}", location.x, location.y, location.map, agent.game().status());
            }
            agent.tick()?;
            if std::env::var_os("GB_A4A_DEBUG").is_some() && agent.game().rng().wandered() != wandered {
                let location = &agent.game().world().location;
                println!("[a4a] recreation people draw {:?} at ({}, {}) on frame {frame}", wandered, location.x, location.y);
            }
            if std::env::var_os("GB_A4A_DEBUG").is_some() && agent.game().rng().drawn() != before {
                let location = &agent.game().world().location;
                println!("[a4a] recreation drew to {:?} at ({}, {}) facing {:?}", agent.game().rng().drawn(), location.x, location.y, location.facing);
            }
        }
        Err("the recreation never asked for its next overworld action".to_string())
    }));
    let agent = match ran {
        Ok(ran) => ran?,
        Err(panic) => {
            let why = panic.downcast_ref::<String>().cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            return Err(format!("{why}: the cartridge drew {drawn} random bytes and {people} for the people"));
        }
    };
    if let Some(missing) = missing.lock().unwrap().take() {
        return Err(format!("the recreation: {missing}"));
    }
    // A person's row has no square in its id, because a person has none that holds still.
    let talked = matches!(segment.answers.first(), Some(Answer::Overworld(id)) if id.matches(':').count() == 1);
    let theirs = theirs.paced_apart(talked);
    let ours = compared(&agent.game_state()?).paced_apart(talked);
    let differ = differences(&ours, &theirs);
    if differ.is_empty() {
        return Ok(());
    }
    let taken = agent.game().rng().drawn().unwrap_or(0);
    Err(format!("the recreation drew {taken} random bytes, the cartridge {asked} before it asked\n{}",
                differ.join("\n")))
}

/// Every segment replayed across the machine's cores: `(index, why)` for each that differs.
pub(crate) fn replay_all(segments: &[Segment]) -> Vec<(usize, String)> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failed = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| for _ in 0..workers {
        scope.spawn(|| loop {
            let at = next.fetch_add(1, Ordering::Relaxed);
            let Some(segment) = segments.get(at) else { break };
            let result = std::panic::catch_unwind(|| replay(segment))
                .unwrap_or_else(|_| Err("panicked".into()));
            if let Err(why) = result {
                println!("[a4a] segment {at} {} differs: {}", segment.describe(), why.replace('\n', " | "));
                failed.lock().unwrap().push((at, why));
            }
        });
    });
    let mut failed = failed.into_inner().unwrap();
    failed.sort_by_key(|&(at, _)| at);
    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows the cartridge offers where the fixture stands, once it has polled.
    fn rows(state: &[u8]) -> Vec<String> {
        let mut cartridge = Cartridge::from_state(state);
        assert!((0..600).any(|_| cartridge.frame()));
        PokemonApi::new(&mut cartridge.gb).game_state().unwrap().map.actions().iter().map(|a| a.id()).collect()
    }

    fn segment(state: &[u8], answers: Vec<Answer>) -> Segment {
        let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
        gb.load_state(state).unwrap();
        Segment { state: state.to_vec(), sram: gb.dump_sram(), answers }
    }

    /// Every row on every fixture's menu, as far as a fixture polls in the overworld at all.
    #[cfg(feature = "slow-tests")]
    #[test]
    fn every_row_from_every_fixture_ends_where_the_cartridge_ends() {
        let dir = std::env::var("GB_A4A_DIR").map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pokemon/data"));
        let mut states: Vec<_> = std::fs::read_dir(&dir).unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|e| e == "bin"))
            .collect();
        states.sort();
        let only = std::env::var("GB_A4A_ONLY").ok();
        states.retain(|path| only.as_ref().is_none_or(|only| path.to_string_lossy().contains(only.as_str())));
        let next = std::sync::atomic::AtomicUsize::new(0);
        let (replayed, failed) = (Mutex::new(0usize), Mutex::new(Vec::new()));
        let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
        std::thread::scope(|scope| for _ in 0..workers {
            scope.spawn(|| while let Some(path) = states.get(next.fetch_add(1, Ordering::Relaxed)) {
                let state = std::fs::read(path).unwrap();
                let mut cartridge = Cartridge::from_state(&state);
                if !(0..300).any(|_| cartridge.frame()) {
                    continue;
                }
                let name = path.file_stem().unwrap().to_string_lossy().into_owned();
                let ids: Vec<String> = PokemonApi::new(&mut cartridge.gb).game_state().unwrap()
                    .map.actions().iter().map(|a| a.id()).collect();
                let row = std::env::var("GB_A4A_ROW").ok();
                for id in ids.into_iter().filter(|id| row.as_ref().is_none_or(|row| id == row)) {
                    let result = std::panic::catch_unwind(|| replay(&segment(&state, vec![Answer::Overworld(id.clone())])))
                        .unwrap_or_else(|_| Err("panicked".into()));
                    *replayed.lock().unwrap() += 1;
                    if let Err(why) = result {
                        println!("DIFFERS {name} {id}: {}", why.replace('\n', " | "));
                        failed.lock().unwrap().push(format!("{name} {id}"));
                    }
                }
            });
        });
        let (replayed, failed) = (replayed.into_inner().unwrap(), failed.into_inner().unwrap());
        assert!(replayed > 0, "no fixture polled with a row to replay");
        assert!(failed.is_empty(), "{} of {replayed} rows differ:\n{}", failed.len(), failed.join("\n"));
    }

    #[test]
    fn every_row_out_of_pallet_town_ends_where_the_cartridge_ends() {
        let state = include_bytes!("../pokemon/data/pallet-town-state.bin");
        let rows = rows(state);
        assert!(!rows.is_empty());
        let failed: Vec<String> = rows.iter()
            .filter_map(|id| replay(&segment(state, vec![Answer::Overworld(id.clone())]))
                .err().map(|why| format!("{id}:\n{why}")))
            .collect();
        assert!(failed.is_empty(), "{} of {} rows differ:\n{}", failed.len(), rows.len(), failed.join("\n"));
    }
}
