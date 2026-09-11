//! The model's battle script: a sandboxed program that decides battle turns without a request.
//! ```text
//! $GB_RUN_DIR/<run-id>/battle-script.json      { source, armed, last_failure }
//! ```

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rhai::{Array, Dynamic, Engine, EvalAltResult, Map, Position};

use crate::pokemon::GameState;
use crate::pokemon::battle::{is_ghost_battle, BattleAction, BattleType};
use crate::pokemon::damage::expected_damage;
use crate::pokemon::pokemon::PokemonSummary;
use crate::pokemon::policy::battle_options;
use crate::run::files;

/// The docs `get_battle_script_docs` answers with, sent verbatim.
pub const DOCS: &str = include_str!("battle_script/DOCS.md");

/// The deterministic policy's battle strategy, written as a battle script.
pub const DETERMINISTIC: &str = include_str!("battle_script/DETERMINISTIC.rhai");

/// The script every run starts with and an unset returns to: it only calls `battle.ask()`.
pub const DEFAULT: &str = include_str!("battle_script/DEFAULT.rhai");

/// Bounded because `read_battle_script` and every validation failure quote the source whole.
pub const MAX_SOURCE: usize = 6_000;

/// How long a script's stated purpose may be.
pub const MAX_PURPOSE: usize = 200;

/// How much of a disarm reason rides on the overworld turn.
pub const MAX_FAILURE: usize = 240;

/// The fuel: rhai stops at this many operations, the guard `catch_unwind` cannot give on a loop.
pub const MAX_OPERATIONS: u64 = 20_000;

/// The wall-clock abort, checked from rhai's progress hook.
pub const MAX_RUNTIME: Duration = Duration::from_millis(50);

/// How many `print` lines are carried back to the model.
pub const MAX_PRINTS: usize = 12;
/// How long one `print` line may be before it is truncated.
pub const MAX_PRINT_LEN: usize = 160;

/// What the engine reports when a script never chose anything.
const NO_ACTION: &str = "the script ran to the end without calling an action. Every path through it \
                         has to reach one of `battle.fight`, `battle.switch_to`, `battle.use_item`, \
                         `battle.run` or `battle.ask`.";

/// What one evaluation decided.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A legal action, already resolved against `battle_options`.
    Action(BattleAction),
    /// `battle.ask()`: the model answers this turn, and the script stays armed.
    Ask,
    /// The script did not produce an action.
    Failed(String),
}

/// One evaluation: what it decided, and everything it printed on the way.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub outcome: Outcome,
    pub prints: Vec<String>,
}

impl Evaluation {
    fn failed(why: impl Into<String>, prints: Vec<String>) -> Self {
        Self { outcome: Outcome::Failed(why.into()), prints }
    }
}

/// A choice the script made, before it has been checked against the game.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Choice {
    Fight(Ref),
    Switch(Ref),
    Item(String),
    Run,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ref {
    Slot(i64),
    Name(String),
}

impl Ref {
    /// A reference from what the script passed: one of our objects, an integer, or a name.
    fn of(value: &Dynamic) -> Result<Self, String> {
        if let Some(map) = value.read_lock::<Map>() {
            if let Some(slot) = map.get("slot").and_then(|slot| slot.as_int().ok()) {
                return Ok(Ref::Slot(slot));
            }
            if let Some(name) = map.get("name").and_then(|name| name.clone().into_string().ok()) {
                return Ok(Ref::Name(name));
            }
            return Err("that object has no `slot` and no `name`".to_string());
        }
        if let Ok(slot) = value.as_int() {
            return Ok(Ref::Slot(slot));
        }
        // `()` gets its own sentence, because it is the mistake a model actually makes.
        if value.is_unit() {
            return Err(
                "it was given `()`. That is what `battle.best_move` is when nothing you know can \
                 damage the foe, and what a variable you never assigned is. Check for it before \
                 passing it on: `if battle.best_move == () { ... }`"
                    .to_string(),
            );
        }
        match value.clone().into_string() {
            Ok(name) => Ok(Ref::Name(name)),
            Err(actual) => Err(format!("it expected a move, a Pokémon, a name or a slot, got a {actual}")),
        }
    }

    fn describe(&self) -> String {
        match self {
            Ref::Slot(slot) => format!("slot {slot}"),
            Ref::Name(name) => format!("`{name}`"),
        }
    }
}

/// Names are compared with the punctuation and the case taken out.
fn normalised(name: &str) -> String {
    name.chars()
        // `é` is alphanumeric, so it is mapped here or `POKÉBALL` never equals `POKEBALL`.
        .map(|c| match c {
            'é' | 'É' | 'è' | 'È' => 'E',
            other => other,
        })
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_uppercase)
        .collect()
}

/// The `battle` global.
#[derive(Clone)]
struct Battle {
    facts: Rc<Map>,
    choice: Rc<RefCell<Option<Choice>>>,
}

impl Battle {
    fn get(&self, key: &str) -> Dynamic {
        self.facts.get(key).cloned().unwrap_or(Dynamic::UNIT)
    }

    /// Record a choice and stop the script.
    fn commit(&self, choice: Choice) -> Result<(), Box<EvalAltResult>> {
        let mut cell = self.choice.borrow_mut();
        if cell.is_none() {
            *cell = Some(choice);
        }
        Err(EvalAltResult::ErrorTerminated(Dynamic::UNIT, Position::NONE).into())
    }

    fn action(&self, kind: fn(Ref) -> Choice, value: Dynamic) -> Result<(), Box<EvalAltResult>> {
        match Ref::of(&value) {
            Ok(reference) => self.commit(kind(reference)),
            Err(why) => Err(EvalAltResult::ErrorRuntime(Dynamic::from(why), Position::NONE).into()),
            // `describe` prefixes nothing, so `why` names the value itself.
        }
    }
}

/// The engine every script is compiled and run by.
fn engine(deadline: Instant, prints: Rc<RefCell<Vec<String>>>) -> Engine {
    let mut engine = Engine::new();

    engine.set_max_operations(MAX_OPERATIONS);
    engine.set_max_call_levels(32);
    engine.set_max_expr_depths(64, 32);
    engine.set_max_string_size(8 * 1024);
    engine.set_max_array_size(512);
    engine.set_max_map_size(512);
    // A nested `eval` is a second parser that the limits above do not account for.
    engine.disable_symbol("eval");

    engine.on_progress(move |_| match Instant::now() >= deadline {
        true => Some(Dynamic::UNIT),
        false => None,
    });

    let captured = Rc::clone(&prints);
    engine.on_print(move |line| {
        let mut lines = captured.borrow_mut();
        if lines.len() < MAX_PRINTS {
            lines.push(truncated(line, MAX_PRINT_LEN));
        }
    });
    // `debug` is rhai's other output channel and a model will reach for it.
    let captured = Rc::clone(&prints);
    engine.on_debug(move |line, _, _| {
        let mut lines = captured.borrow_mut();
        if lines.len() < MAX_PRINTS {
            lines.push(truncated(line, MAX_PRINT_LEN));
        }
    });

    engine
        .register_type_with_name::<Battle>("Battle")
        .register_indexer_get(|battle: &mut Battle, key: &str| battle.get(key))
        .register_get("kind", |battle: &mut Battle| battle.get("kind"))
        .register_get("turn", |battle: &mut Battle| battle.get("turn"))
        .register_get("me", |battle: &mut Battle| battle.get("me"))
        .register_get("foe", |battle: &mut Battle| battle.get("foe"))
        .register_get("party", |battle: &mut Battle| battle.get("party"))
        .register_get("bag", |battle: &mut Battle| battle.get("bag"))
        .register_get("moves", |battle: &mut Battle| battle.get("moves"))
        .register_get("best_move", |battle: &mut Battle| battle.get("best_move"))
        .register_get("can_run", |battle: &mut Battle| battle.get("can_run"))
        .register_fn("fight", |battle: &mut Battle, value: Dynamic| battle.action(Choice::Fight, value))
        // `switch_to`, because `switch` is a reserved word in rhai.
        .register_fn("switch_to", |battle: &mut Battle, value: Dynamic| battle.action(Choice::Switch, value))
        .register_fn("use_item", |battle: &mut Battle, value: Dynamic| {
            match value.clone().into_string() {
                Ok(name) => battle.commit(Choice::Item(name)),
                Err(actual) => Err(EvalAltResult::ErrorRuntime(
                    Dynamic::from(format!("`use_item` takes an item name, got a {actual}")),
                    Position::NONE,
                )
                .into()),
            }
        })
        .register_fn("run", |battle: &mut Battle| battle.commit(Choice::Run))
        .register_fn("ask", |battle: &mut Battle| battle.commit(Choice::Ask));

    engine
}

/// One move, as the script sees it.
fn move_map(slot: usize, battle_move: &crate::pokemon::move_name::PokemonMove, me: &PokemonSummary, turn: Turn, usable: bool) -> Map {
    let metadata = battle_move.name.metadata();
    let mut map = Map::new();
    map.insert("slot".into(), Dynamic::from(slot as i64));
    map.insert("name".into(), Dynamic::from(battle_move.name.to_string()));
    // `move_type`, because `type` is a reserved word in rhai.
    map.insert("move_type".into(), Dynamic::from(metadata.move_type.to_string()));
    map.insert("power".into(), Dynamic::from(metadata.power.unwrap_or(0) as i64));
    map.insert("accuracy".into(), Dynamic::from(metadata.accuracy as i64));
    map.insert("pp".into(), Dynamic::from(battle_move.pp as i64));
    map.insert("max_pp".into(), Dynamic::from(metadata.pp as i64));
    map.insert("damage".into(), Dynamic::from(turn.damage(me, battle_move.name) as i64));
    map.insert("effectiveness".into(), Dynamic::from(match turn.ghost {
        true => 0.0,
        false => crate::pokemon::damage::type_multiplier(battle_move.name, turn.foe),
    }));
    map.insert("usable".into(), Dynamic::from(usable));
    map
}

/// What this turn lets a move do, shared by every Pokémon the script reads.
#[derive(Clone, Copy)]
struct Turn<'a> {
    foe: &'a PokemonSummary,
    ghost: bool,
}

impl Turn<'_> {
    /// Expected damage against the foe, or 0 when nothing can damage it at all.
    fn damage(&self, me: &PokemonSummary, battle_move: crate::pokemon::move_name::PokemonMoveName) -> u16 {
        match self.ghost {
            true => 0,
            false => expected_damage(me, battle_move, self.foe).unwrap_or(0),
        }
    }
}

/// One Pokémon, as the script sees it.
fn pokemon_map(slot: usize, name: &str, mon: &PokemonSummary, turn: Turn, fight_slots: Option<&[usize]>) -> Map {
    let mut map = Map::new();
    map.insert("slot".into(), Dynamic::from(slot as i64));
    map.insert("name".into(), Dynamic::from(name.to_string()));
    map.insert("species".into(), Dynamic::from(mon.species.to_string()));
    map.insert("level".into(), Dynamic::from(mon.level as i64));
    map.insert("hp".into(), Dynamic::from(mon.current_hp as i64));
    map.insert("max_hp".into(), Dynamic::from(mon.stats.hp as i64));
    map.insert("hp_frac".into(), Dynamic::from(match mon.stats.hp {
        0 => 0.0,
        max => mon.current_hp as f64 / max as f64,
    }));
    map.insert("status".into(), Dynamic::from(status_word(mon.status)));
    map.insert("fainted".into(), Dynamic::from(mon.current_hp == 0));
    let mut types: Array = Vec::new();
    types.push(Dynamic::from(mon.types[0].to_string()));
    if mon.types[1] != mon.types[0] {
        types.push(Dynamic::from(mon.types[1].to_string()));
    }
    map.insert("types".into(), Dynamic::from(types));

    let moves: Array = mon
        .moves
        .iter()
        .enumerate()
        .filter_map(|(index, battle_move)| {
            let battle_move = battle_move.as_ref()?;
            let offered = match fight_slots {
                Some(slots) => slots.contains(&index),
                None => !turn.ghost,
            };
            let usable = offered && battle_move.pp > 0 && mon.disabled_move_slot != Some(index as u8);
            Some(Dynamic::from(move_map(index, battle_move, mon, turn, usable)))
        })
        .collect();
    map.insert("moves".into(), Dynamic::from(moves));
    map
}

/// A healthy Pokémon says `""`, not `"None"`.
fn status_word(status: crate::pokemon::status::PokemonStatus) -> String {
    use crate::pokemon::status::PokemonStatus::*;
    match status {
        None => "",
        Paralyzed => "paralyzed",
        Frozen => "frozen",
        Burned => "burned",
        Poisoned => "poisoned",
        Asleep { .. } => "asleep",
    }
    .to_string()
}

/// Everything the script can read, built once per evaluation.
fn facts(state: &GameState, turn: u32, options: &[BattleAction]) -> Option<Map> {
    let battle = state.battle.as_ref()?;
    let me = &battle.player;
    let foe = &battle.enemy;
    let ghost = is_ghost_battle(state.map.map, &state.bag, battle.battle_type);
    let against_foe = Turn { foe, ghost };
    let fight_slots: Vec<usize> = options
        .iter()
        .filter_map(|action| match action {
            BattleAction::Fight { slot, .. } => Some(*slot as usize),
            _ => None,
        })
        .collect();

    let mut map = Map::new();
    map.insert("kind".into(), Dynamic::from(match battle.battle_type {
        BattleType::Wild => "wild",
        BattleType::Trainer => "trainer",
        BattleType::Safari => "safari",
    }
    .to_string()));
    map.insert("turn".into(), Dynamic::from(turn as i64));
    map.insert("can_run".into(), Dynamic::from(options.contains(&BattleAction::Run)));
    map.insert("ghost".into(), Dynamic::from(ghost));
    map.insert("trapped".into(), Dynamic::from(battle.enemy_trapping));
    map.insert("catch_rate".into(), Dynamic::from(battle.enemy_catch_rate as i64));

    let active = battle.active_party_slot as usize;
    let my_name = state
        .pokemon
        .iter()
        .nth(active)
        .map(|mon| mon.nickname.to_default_string())
        .unwrap_or_else(|| me.species.to_string());
    let me_map = pokemon_map(active, &my_name, me, against_foe, Some(&fight_slots));
    let my_moves = me_map.get("moves").cloned().unwrap_or(Dynamic::UNIT);
    map.insert("me".into(), Dynamic::from(me_map));
    // The foe's `damage` is scored against itself and meaningless, but must exist to be ignored.
    map.insert("foe".into(), Dynamic::from(pokemon_map(usize::MAX, &foe.species.to_string(), foe, Turn { foe: me, ghost }, None)));
    map.insert("moves".into(), my_moves);

    let party: Array = state
        .pokemon
        .iter()
        .enumerate()
        .map(|(slot, mon)| {
            let fight_slots = (slot == active).then_some(fight_slots.as_slice());
            Dynamic::from(pokemon_map(slot, &mon.nickname.to_default_string(), &mon.summary(), against_foe, fight_slots))
        })
        .collect();
    map.insert("party".into(), Dynamic::from(party));

    let bag: Array = state
        .bag
        .iter()
        .filter(|item| item.quantity > 0)
        .map(|item| {
            let mut entry = Map::new();
            entry.insert("name".into(), Dynamic::from(item.id.to_string()));
            entry.insert("count".into(), Dynamic::from(item.quantity as i64));
            Dynamic::from(entry)
        })
        .collect();
    map.insert("bag".into(), Dynamic::from(bag));

    // The highest-damage usable move, which most scripts want.
    let best = me
        .moves
        .iter()
        .enumerate()
        .filter_map(|(index, battle_move)| {
            let battle_move = battle_move.as_ref()?;
            if !fight_slots.contains(&index) || battle_move.pp == 0 || me.disabled_move_slot == Some(index as u8) {
                return None;
            }
            let damage = against_foe.damage(me, battle_move.name);
            (damage > 0).then(|| (damage, move_map(index, battle_move, me, against_foe, true)))
        })
        .max_by_key(|(damage, _)| *damage);
    map.insert("best_move".into(), match best {
        Some((_, best)) => Dynamic::from(best),
        None => Dynamic::UNIT,
    });

    Some(map)
}

/// Turn what the script said into an action the game will actually accept, or say why not.
fn resolve(choice: Choice, state: &GameState, options: &[BattleAction]) -> Outcome {
    match choice {
        Choice::Ask => Outcome::Ask,
        Choice::Run => match options.contains(&BattleAction::Run) {
            true => Outcome::Action(BattleAction::Run),
            false => Outcome::Failed(
                "`battle.run` was called in a trainer battle, and there is no running from one. \
                 Check `battle.can_run` first."
                    .to_string(),
            ),
        },
        Choice::Fight(reference) => {
            let found = options.iter().find(|action| match (action, &reference) {
                (BattleAction::Fight { slot, .. }, Ref::Slot(wanted)) => *slot as i64 == *wanted,
                (BattleAction::Fight { battle_move, .. }, Ref::Name(wanted)) => {
                    normalised(&battle_move.name.to_string()) == normalised(wanted)
                }
                _ => false,
            });
            match found {
                Some(action) => Outcome::Action(*action),
                None => Outcome::Failed(format!(
                    "`battle.fight` was given {}, which is not a move that can be used this turn. \
                     Usable now: {}.",
                    reference.describe(),
                    list(options.iter().filter_map(|action| match action {
                        BattleAction::Fight { battle_move, .. } => Some(battle_move.name.to_string()),
                        _ => None,
                    })),
                )),
            }
        }
        Choice::Switch(reference) => {
            let found = options.iter().find(|action| match (action, &reference) {
                (BattleAction::SwitchPokemon { slot, .. }, Ref::Slot(wanted)) => *slot as i64 == *wanted,
                (BattleAction::SwitchPokemon { slot, pokemon }, Ref::Name(wanted)) => {
                    let nickname = state.pokemon.get(*slot as usize).map(|mon| mon.nickname.to_default_string());
                    nickname.map(|name| normalised(&name) == normalised(wanted)).unwrap_or(false)
                        || normalised(&pokemon.species.to_string()) == normalised(wanted)
                }
                _ => false,
            });
            match found {
                Some(action) => Outcome::Action(*action),
                None => Outcome::Failed(format!(
                    "`battle.switch_to` was given {}, which is not a Pokémon that can be sent out this \
                     turn: the active one and any that have fainted cannot. Available now: {}.",
                    reference.describe(),
                    list(options.iter().filter_map(|action| match action {
                        BattleAction::SwitchPokemon { slot, .. } => state
                            .pokemon
                            .iter()
                            .nth(*slot as usize)
                            .map(|mon| mon.nickname.to_default_string()),
                        _ => None,
                    })),
                )),
            }
        }
        Choice::Item(name) => {
            let found = options.iter().find(|action| match action {
                BattleAction::UseItem { item, .. } => normalised(&item.id.to_string()) == normalised(&name),
                _ => false,
            });
            match found {
                Some(action) => Outcome::Action(action.clone()),
                None => Outcome::Failed(format!(
                    "`battle.use_item` was given `{name}`, which is not in the bag. In it now: {}.",
                    list(options.iter().filter_map(|action| match action {
                        BattleAction::UseItem { item, .. } => Some(item.id.to_string()),
                        _ => None,
                    })),
                )),
            }
        }
    }
}

fn list(names: impl Iterator<Item = String>) -> String {
    let names: Vec<String> = names.collect();
    match names.is_empty() {
        true => "nothing".to_string(),
        false => names.join(", "),
    }
}

/// Evaluate `source` against the turn `state` describes, and resolve whatever it chose.
pub fn run(source: &str, state: &GameState, turn: u32) -> Evaluation {
    let Some(options) = battle_options(state) else {
        return Evaluation::failed("there is no battle to decide", Vec::new());
    };
    let Some(facts) = facts(state, turn, &options) else {
        return Evaluation::failed("there is no battle to decide", Vec::new());
    };

    let prints = Rc::new(RefCell::new(Vec::new()));
    let choice = Rc::new(RefCell::new(None));
    let deadline = Instant::now() + MAX_RUNTIME;

    let battle = Battle { facts: Rc::new(facts), choice: Rc::clone(&choice) };
    let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let engine = engine(deadline, Rc::clone(&prints));
        let mut scope = rhai::Scope::new();
        scope.push_constant("battle", battle);
        engine.run_with_scope(&mut scope, source)
    }));

    let prints = prints.borrow().clone();
    let taken = choice.borrow_mut().take();

    // The cell is read before the error, always.
    if let Some(choice) = taken {
        return Evaluation { outcome: resolve(choice, state, &options), prints };
    }

    match evaluated {
        Ok(Ok(())) => Evaluation::failed(NO_ACTION, prints),
        Ok(Err(failure)) => Evaluation::failed(describe(&failure), prints),
        // A panic would unwind the emulator thread and lose the checkpoint, so it is a failure.
        Err(_) => Evaluation::failed("the script made the sandbox panic", prints),
    }
}

/// Rhai's own error text, with the two limits said in words a model can act on.
fn describe(failure: &EvalAltResult) -> String {
    match failure {
        EvalAltResult::ErrorTooManyOperations(position) => format!(
            "the script used more than {MAX_OPERATIONS} operations and was stopped at {position}. \
             A battle turn is a scan of six Pokémon, not a search.",
        ),
        EvalAltResult::ErrorTerminated(..) => format!(
            "the script ran for longer than {} ms and was stopped.",
            MAX_RUNTIME.as_millis(),
        ),
        // Our own refusals come back here, which rhai's `Display` prefixes with "Runtime error:".
        EvalAltResult::ErrorRuntime(message, position) => match message.clone().into_string() {
            Ok(sentence) => format!("{sentence} (at {position})"),
            Err(_) => failure.to_string(),
        },
        other => other.to_string(),
    }
}

/// A disarm reason, cut to [`MAX_FAILURE`] for the overworld turn that carries it every turn.
fn standing_failure(why: &str) -> String {
    match why.char_indices().nth(MAX_FAILURE) {
        Some((at, _)) => format!("{}…", &why[..at]),
        None => why.to_string(),
    }
}

fn truncated(text: &str, limit: usize) -> String {
    match text.len() <= limit {
        true => text.to_string(),
        false => text
            .chars()
            .scan(0usize, |used, c| {
                *used += c.len_utf8();
                (*used <= limit).then_some(c)
            })
            .collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Saved {
    #[serde(default)]
    source: Option<String>,
    /// Whether the policy should consult it.
    #[serde(default)]
    armed: bool,
    #[serde(default)]
    last_failure: Option<String>,
    /// The model's one line on what it wrote this for, said back on every overworld turn.
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    decided: u32,
}

/// A run always has a script, and the one it starts with is [`DEFAULT`].
impl Default for Saved {
    fn default() -> Self {
        Self {
            source: Some(DEFAULT.to_string()),
            armed: true,
            last_failure: None,
            purpose: None,
            decided: 0,
        }
    }
}

/// The script on disk and the tool calls against it, answered on the worker thread.
pub struct BattleScript {
    /// `None` for a run with no directory.
    path: Option<PathBuf>,
    saved: Saved,
}

impl BattleScript {
    /// Never fails: an unreadable file starts on [`DEFAULT`].
    pub fn open(run_dir: Option<&Path>) -> Self {
        let Some(run_dir) = run_dir else {
            return Self { path: None, saved: Saved::default() };
        };
        let path = run_dir.join(files::BATTLE_SCRIPT);
        let mut saved: Saved = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        if saved.source.is_none() {
            saved = Saved::default();
        }
        Self { path: Some(path), saved }
    }

    pub fn source(&self) -> Option<&str> {
        self.saved.source.as_deref()
    }

    /// Whether the script the model wrote is deciding battle turns.
    pub fn armed(&self) -> bool {
        self.saved.armed && self.saved.source.is_some() && !self.is_default()
    }

    /// Whether [`DEFAULT`] is installed, compared trimmed as [`Self::set`] stores it.
    pub fn is_default(&self) -> bool {
        self.saved.source.as_deref().map(str::trim) == Some(DEFAULT.trim())
    }

    pub fn last_failure(&self) -> Option<&str> {
        self.saved.last_failure.as_deref()
    }

    /// The purpose, the turns decided, and why it stopped if it has; empty for the default.
    pub fn standing(&self) -> ScriptStanding {
        ScriptStanding {
            purpose: self.saved.purpose.clone(),
            decided: self.saved.decided,
            // Only while it is the current state.
            failure: (!self.armed()).then(|| self.saved.last_failure.as_deref().map(standing_failure)).flatten(),
        }
    }

    /// Persist the count the policy keeps in [`Live`], drained at the top of a turn: one writer
    /// per run directory.
    pub fn record_decided(&mut self, decided: u32) {
        if self.saved.decided == decided { return }
        self.saved.decided = decided;
        self.persist();
    }

    /// What [`Live`] should hold: the source only while armed, since the policy runs what it gets.
    pub fn live_source(&self) -> Option<String> {
        self.armed().then(|| self.saved.source.clone()).flatten()
    }

    /// Three states, because "still the default" and "broke" want opposite sentences and neither
    /// reaches [`Live`] with a source.
    pub fn state(&self) -> ScriptState {
        match (self.armed(), self.is_default()) {
            (true, _) => ScriptState::Armed,
            // Only reachable through a real failure, which keeps the source it failed on.
            (false, false) => ScriptState::Disarmed,
            (false, true) => ScriptState::Unedited,
        }
    }

    /// `set_battle_script`. Validates before arming, and the answer is the validation table.
    pub fn set(&mut self, source: Option<&str>, purpose: Option<&str>) -> String {
        let Some(source) = source.map(str::trim).filter(|source| !source.is_empty()) else {
            self.saved = Saved::default();
            self.persist();
            return "ok, back to the default script, which hands you every battle turn. They cost \
                    you a request each again."
                .to_string();
        };
        if source.len() > MAX_SOURCE {
            return format!(
                "That script is {} bytes and the limit is {MAX_SOURCE}. Nothing was changed.",
                source.len(),
            );
        }

        match validate(source) {
            Ok(table) => {
                self.saved = Saved {
                    source: Some(source.to_string()),
                    armed: true,
                    last_failure: None,
                    purpose: purpose.map(str::trim).filter(|p| !p.is_empty()).map(|p| {
                        // `char_indices`, not a byte slice.
                        match p.char_indices().nth(MAX_PURPOSE) {
                            Some((at, _)) => format!("{}…", &p[..at]),
                            None => p.to_string(),
                        }
                    }),
                    // Zeroed rather than carried over.
                    decided: 0,
                };
                self.persist();
                format!("ok, armed. Validated on {} scenarios:\n{table}", SCENARIOS.len())
            }
            Err(why) => format!("Not armed, and nothing was changed. {why}"),
        }
    }

    /// `read_battle_script`.
    pub fn read(&self) -> String {
        let Some(source) = self.source() else {
            return "There is no battle script, which should not be possible. \
                    `set_battle_script` will install one."
                .to_string();
        };
        let state = match (self.armed(), self.is_default(), self.last_failure()) {
            // The same two facts the standing line carries, from the same accessor.
            (true, _, _) => {
                let standing = self.standing();
                let mut armed = "Armed. This is deciding your battle turns.".to_string();
                if let Some(purpose) = standing.purpose.as_deref() {
                    armed.push_str(&format!(" You installed it for: \"{purpose}\"."));
                }
                armed.push_str(&match standing.decided {
                    0 => " It has not decided a battle turn yet.".to_string(),
                    1 => " It has decided 1 battle turn since.".to_string(),
                    n => format!(" It has decided {n} battle turns since."),
                });
                armed
            }
            (false, true, _) => "**This is the default script and it decides nothing** — it hands \
                                 every battle turn back to you, so each one costs a request. \
                                 Replace it with `set_battle_script`."
                .to_string(),
            (false, false, Some(why)) => format!("**Disarmed** after it failed: {why}\n\nFix it and call `set_battle_script` again, or pass `null` to go back to the default."),
            (false, false, None) => "Not armed.".to_string(),
        };
        format!("{state}\n\n```rhai\n{source}\n```")
    }

    /// Keeps the script for the model to edit, deciding nothing until it is armed again.
    pub fn disarm(&mut self, why: &str) {
        if !self.saved.armed && self.saved.last_failure.as_deref() == Some(why) {
            return;
        }
        self.saved.armed = false;
        self.saved.last_failure = Some(why.to_string());
        self.persist();
    }

    fn persist(&self) {
        let Some(path) = self.path.as_ref() else { return };
        let Ok(json) = serde_json::to_vec_pretty(&self.saved) else { return };
        if let Err(failure) = crate::run::write_atomically(path, &json) {
            eprintln!("battle-script: {failure}");
        }
    }
}

/// Whether a script is deciding battle turns, as the battle turn reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScriptState {
    /// [`DEFAULT`] is installed, untouched, and decides nothing.
    #[default]
    Unedited,
    /// Armed, so a battle turn carrying this is one it did not decide.
    Armed,
    /// One was written and has stopped deciding turns.
    Disarmed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptStanding {
    /// The model's own words from `set_battle_script`; `None` for the default.
    pub purpose: Option<String>,
    /// Battle turns decided since this script was armed.
    pub decided: u32,
    /// Why it stopped, capped at [`MAX_FAILURE`]; `Some` exactly while [`ScriptState::Disarmed`].
    pub failure: Option<String>,
}

/// The armed script, written by the worker thread and run by the emulator thread.
#[derive(Debug, Default)]
pub struct Live {
    inner: std::sync::Mutex<LiveInner>,
}

#[derive(Debug, Default)]
struct LiveInner {
    source: Option<String>,
    state: ScriptState,
    failure: Option<String>,
    standing: ScriptStanding,
}

impl Live {
    /// Point the policy at a script or none, after a successful `set_battle_script` or a restart.
    pub fn arm(&self, source: Option<String>, state: ScriptState, standing: ScriptStanding) {
        let mut inner = self.locked();
        inner.source = source;
        inner.state = state;
        inner.failure = None;
        inner.standing = standing;
    }

    /// What the policy should run this turn, if anything.
    pub fn source(&self) -> Option<String> {
        self.locked().source.clone()
    }

    /// Survives [`Self::take_failure`]: the reason is reported once, the broken script stays.
    pub fn state(&self) -> ScriptState {
        self.locked().state
    }

    /// What the overworld turn should say about it beyond the bare state.
    pub fn standing(&self) -> ScriptStanding {
        self.locked().standing.clone()
    }

    /// The script decided a battle turn. Called on the emulator thread, once per decision.
    pub fn decided_one(&self) {
        let mut inner = self.locked();
        inner.standing.decided = inner.standing.decided.saturating_add(1);
    }

    /// Drops the source at once, so nothing consults it before the worker persists `why`; the
    /// policy never writes the file.
    pub fn failed(&self, why: &str) {
        let mut inner = self.locked();
        inner.source = None;
        inner.state = ScriptState::Disarmed;
        inner.failure = Some(why.to_string());
        inner.standing.failure = Some(standing_failure(why));
    }

    /// Taken by the worker at the top of a turn, once.
    pub fn take_failure(&self) -> Option<String> {
        self.locked().failure.take()
    }

    /// A poisoned lock is recovered: everything held across it is a cloned `String`.
    fn locked(&self) -> std::sync::MutexGuard<'_, LiveInner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The scenarios every script is put through before it is armed.
const SCENARIOS: &[(&str, fn() -> GameState)] = &[
    ("full-hp wild", scenarios::healthy_wild),
    ("low-hp wild", scenarios::hurt_wild),
    ("low-hp trainer, bench healthy", scenarios::hurt_trainer),
    ("last one standing", scenarios::last_mon),
    ("no damaging pp left", scenarios::out_of_pp),
    ("weakened wild, balls in the bag", scenarios::catchable_wild),
    ("ghost in Pokemon Tower, no Silph Scope", scenarios::ghost),
];

/// Compile and run a script through every scenario.
fn validate(source: &str) -> Result<String, String> {
    let mut table = String::new();
    for (name, scenario) in SCENARIOS {
        let state = scenario();
        let evaluation = run(source, &state, 1);
        let what = match &evaluation.outcome {
            // The report's verb phrase, not `BattleAction`'s `Display`.
            Outcome::Action(action) => crate::llm::battle_report::intent(action),
            Outcome::Ask => "hands the turn to you".to_string(),
            Outcome::Failed(why) => {
                let printed = match evaluation.prints.is_empty() {
                    true => String::new(),
                    false => format!("\n\nIt printed, before it stopped:\n{}", indented(&evaluation.prints)),
                };
                return Err(format!("On the `{name}` scenario, {why}{printed}"));
            }
        };
        table.push_str(&format!("  {name:<32} → {what}\n"));
    }
    Ok(table)
}

fn indented(lines: &[String]) -> String {
    lines.iter().map(|line| format!("  {line}\n")).collect()
}

/// A healthy lead in a wild battle, shared so the modules describing a battle cannot drift.
#[cfg(any(test))]
pub fn test_scenario() -> GameState {
    scenarios::healthy_wild()
}

/// The seven turns [`SCENARIOS`] puts a script through, hand-built so validation needs no emulator.
pub(crate) mod scenarios {
    use crate::pokemon::GameState;
    use crate::pokemon::bag::{Bag, BagItem};
    use crate::pokemon::battle::{BattleState, BattleType};
    use crate::pokemon::item::ItemId;
    use crate::pokemon::map::Map;
    use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
    use crate::pokemon::pokemon::Pokemon;
    use crate::pokemon::species::PokemonSpecies;

    /// The level is set through the experience and a `recalculate`, never by writing the field.
    fn mon(species: PokemonSpecies, nickname: &str, level: u8, moves: [PokemonMoveName; 4]) -> Pokemon {
        let mut mon = Pokemon::maxed(species, nickname, moves, "AI", 1);
        mon.experience = species.metadata().experience_group.experience_for_level(level);
        mon.recalculate();
        mon
    }

    /// Set current HP as a fraction of the maximum, which is what every scenario actually varies.
    fn at(mut mon: Pokemon, fraction: f64) -> Pokemon {
        mon.current_hp = ((mon.stats.hp as f64 * fraction).round() as u16).min(mon.stats.hp);
        mon
    }

    fn state(party: Vec<Pokemon>, bag: Vec<BagItem>, battle_type: BattleType, active: u8, foe: Pokemon) -> GameState {
        let mut state = GameState::default();
        for member in party {
            state.pokemon.push(member).expect("the scenario's party fits");
        }
        state.bag = Bag::new(bag);
        state.battle = Some(BattleState {
            battle_type,
            player: state.pokemon.get(active as usize).expect("an active member").summary(),
            enemy: foe.summary(),
            active_party_slot: active,
            enemy_trapping: false,
            enemy_catch_rate: 255,
        });
        state
    }

    fn starter(level: u8) -> Pokemon {
        mon(
            PokemonSpecies::Charmander,
            "SPARKY",
            level,
            [PokemonMoveName::Scratch, PokemonMoveName::Ember, PokemonMoveName::Growl, PokemonMoveName::Leer],
        )
    }

    fn bench(level: u8) -> Pokemon {
        mon(
            PokemonSpecies::Squirtle,
            "SHELLY",
            level,
            [PokemonMoveName::Tackle, PokemonMoveName::WaterGun, PokemonMoveName::Bubble, PokemonMoveName::TailWhip],
        )
    }

    fn rattata(level: u8) -> Pokemon {
        mon(
            PokemonSpecies::Rattata,
            "RATTATA",
            level,
            [PokemonMoveName::Tackle, PokemonMoveName::TailWhip, PokemonMoveName::QuickAttack, PokemonMoveName::HyperFang],
        )
    }

    pub fn healthy_wild() -> GameState {
        state(vec![starter(14), bench(12)], vec![BagItem::new(ItemId::Potion, 3)], BattleType::Wild, 0, rattata(6))
    }

    pub fn hurt_wild() -> GameState {
        state(vec![at(starter(14), 0.06), bench(12)], vec![BagItem::new(ItemId::Potion, 1)], BattleType::Wild, 0, rattata(11))
    }

    pub fn hurt_trainer() -> GameState {
        state(vec![at(starter(14), 0.06), bench(15)], vec![BagItem::new(ItemId::SuperPotion, 2)], BattleType::Trainer, 0, rattata(13))
    }

    /// One member, badly hurt, in a trainer battle: nothing to switch to and nowhere to run.
    pub fn last_mon() -> GameState {
        state(vec![at(starter(14), 0.08)], vec![], BattleType::Trainer, 0, rattata(13))
    }

    /// Every move out of PP: all offered, as Struggle, but none `usable` and `best_move` is `()`.
    pub fn out_of_pp() -> GameState {
        let mut lead = starter(14);
        lead.moves = lead.moves.map(|slot| slot.map(|battle_move| PokemonMove { pp: 0, ..battle_move }));
        state(vec![lead, bench(12)], vec![BagItem::new(ItemId::Potion, 1)], BattleType::Wild, 0, rattata(9))
    }

    /// A ghost without the Silph Scope: only `Run` is offered, every move is `usable: false` with
    /// `damage` 0, and `best_move` is `()`.
    pub fn ghost() -> GameState {
        let gastly = mon(
            PokemonSpecies::Gastly,
            "GASTLY",
            22,
            [PokemonMoveName::Lick, PokemonMoveName::ConfuseRay, PokemonMoveName::NightShade, PokemonMoveName::Hypnosis],
        );
        let mut state = state(vec![starter(30), bench(24)], vec![BagItem::new(ItemId::SuperPotion, 2)], BattleType::Wild, 0, gastly);
        state.map.map = Map::PokemonTower3F;
        state
    }

    pub fn catchable_wild() -> GameState {
        let mut foe = at(rattata(9), 0.15);
        foe.current_hp = foe.current_hp.max(1);
        state(
            vec![starter(16), bench(14)],
            vec![BagItem::new(ItemId::PokeBall, 4), BagItem::new(ItemId::Potion, 2)],
            BattleType::Wild,
            0,
            foe,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::Scratch;

    fn decide(source: &str, state: &GameState) -> Outcome {
        run(source, state, 1).outcome
    }

    fn wild() -> GameState {
        scenarios::healthy_wild()
    }

    /// The shortest script that decides every scenario, as arming requires.
    const ALWAYS_DECIDES: &str = "if battle.can_run { battle.run(); }\nbattle.ask();";

    /// `switch` is reserved in rhai even in method position.
    #[test]
    fn switching_parses_under_the_name_the_keyword_forced() {
        let outcome = decide("battle.switch_to(battle.party[1]);", &wild());
        assert!(
            matches!(outcome, Outcome::Action(BattleAction::SwitchPokemon { slot: 1, .. })),
            "got {outcome:?}",
        );
        assert!(
            matches!(decide(r#"battle.switch_to("SHELLY");"#, &wild()), Outcome::Action(BattleAction::SwitchPokemon { .. })),
            "by name too",
        );
    }

    #[test]
    fn a_script_decides_a_battle_turn() {
        let outcome = decide("battle.fight(battle.best_move);", &wild());
        let Outcome::Action(action) = outcome else { panic!("expected an action, got {outcome:?}") };
        assert!(
            battle_options(&wild()).unwrap().contains(&action),
            "the action has to be one the game offered: {action}",
        );
        assert!(matches!(action, BattleAction::Fight { .. }), "got {action}");
    }

    /// The model copies `DOCS.md`'s example verbatim.
    #[test]
    fn the_worked_example_in_the_docs_runs_on_every_scenario() {
        let example = DOCS
            .rsplit("```rhai")
            .next()
            .and_then(|tail| tail.split("```").next())
            .expect("the docs end with a worked example");
        for (name, scenario) in SCENARIOS {
            let evaluation = run(example, &scenario(), 1);
            assert!(
                !matches!(evaluation.outcome, Outcome::Failed(_)),
                "the documented example failed on `{name}`: {:?}",
                evaluation.outcome,
            );
        }
    }

    /// `run()` followed by more code must not fall through into it.
    #[test]
    fn an_action_ends_the_script() {
        // If `run` did not terminate, `fight` below would overwrite the choice.
        let outcome = decide("battle.run(); battle.fight(battle.best_move);", &wild());
        assert_eq!(outcome, Outcome::Action(BattleAction::Run));
    }

    /// Rhai has `try`/`catch`, so the abort can be swallowed.
    #[test]
    fn the_first_action_wins_even_when_the_abort_is_caught() {
        let outcome = decide(
            "try { battle.run(); } catch(e) { } battle.fight(battle.best_move);",
            &wild(),
        );
        assert_eq!(outcome, Outcome::Action(BattleAction::Run), "the caught abort still committed");
    }

    /// `mv.type` and `battle.switch(...)` are reserved-word parse errors in rhai.
    #[test]
    fn every_name_the_docs_use_is_one_the_parser_accepts() {
        let fields = ["kind", "turn", "can_run", "ghost", "trapped", "catch_rate", "me", "foe", "party", "moves", "best_move", "bag"];
        for field in fields {
            let outcome = decide(&format!("let x = battle.{field}; battle.ask();"), &wild());
            assert_eq!(outcome, Outcome::Ask, "`battle.{field}` does not parse or does not exist");
            assert!(DOCS.contains(&format!("battle.{field}")), "`battle.{field}` exists and is undocumented");
        }
        // Documented in its own section, not merely mentioned somewhere.
        let section = |heading: &str| -> String {
            let from = DOCS.split(heading).nth(1).unwrap_or_else(|| panic!("no `{heading}` section"));
            from.split("\n### ").next().unwrap_or(from).to_string()
        };
        let pokemon = section("### A Pokemon");
        for field in ["slot", "name", "species", "level", "hp", "max_hp", "hp_frac", "status", "types", "fainted", "moves"] {
            let outcome = decide(&format!("let x = battle.me.{field}; battle.ask();"), &wild());
            assert_eq!(outcome, Outcome::Ask, "`mon.{field}` does not parse or does not exist");
            assert!(pokemon.contains(&format!("`{field}`")), "`{field}` is missing from the Pokemon table");
        }
        let a_move = section("### A Move");
        for field in ["slot", "name", "move_type", "power", "accuracy", "pp", "max_pp", "damage", "effectiveness", "usable"] {
            let outcome = decide(&format!("let x = battle.me.moves[0].{field}; battle.ask();"), &wild());
            assert_eq!(outcome, Outcome::Ask, "`mv.{field}` does not parse or does not exist");
            assert!(a_move.contains(&format!("`{field}`")), "`{field}` is missing from the Move table");
        }
        // The actions, each in the position a script actually calls it from.
        for call in ["fight(battle.best_move)", "switch_to(battle.party[1])", r#"use_item("Potion")"#, "run()", "ask()"] {
            let outcome = decide(&format!("battle.{call};"), &wild());
            assert!(!matches!(outcome, Outcome::Failed(_)), "`battle.{call}` failed: {outcome:?}");
        }
    }

    /// A hang is the failure `catch_unwind` cannot catch.
    #[test]
    fn a_runaway_script_is_stopped_rather_than_hanging() {
        let started = Instant::now();
        let outcome = decide("let n = 0; loop { n += 1; }", &wild());
        let Outcome::Failed(why) = outcome else { panic!("expected a failure, got {outcome:?}") };
        assert!(why.contains("operations") || why.contains("ms"), "the reason has to be actionable: {why}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "it has to stop promptly, took {:?}",
            started.elapsed(),
        );
    }

    /// Recursion, which the call-depth limit reaches before the operation count.
    #[test]
    fn unbounded_recursion_is_stopped() {
        let outcome = decide("fn down(n) { down(n + 1) } down(0); battle.run();", &wild());
        assert!(matches!(outcome, Outcome::Failed(_)), "got {outcome:?}");
    }

    #[test]
    fn choosing_nothing_is_a_failure_that_says_so() {
        let Outcome::Failed(why) = decide("let x = 1 + 1;", &wild()) else { panic!("expected a failure") };
        assert!(why.contains("without calling an action"), "{why}");
        assert!(why.contains("battle.fight"), "the reason names the way out: {why}");
    }

    /// The script filters `battle_options`; it never invents.
    #[test]
    fn an_action_the_game_would_refuse_is_named_rather_than_taken() {
        let Outcome::Failed(why) = decide(r#"battle.fight("Hydro Pump");"#, &wild()) else {
            panic!("a move it does not know must not be accepted")
        };
        assert!(why.contains("Hydro Pump"), "{why}");
        assert!(why.contains("Ember"), "the reason lists what is actually usable: {why}");
    }

    /// Running from a trainer: the message points at `can_run` rather than at the tool.
    #[test]
    fn running_from_a_trainer_is_refused_with_the_reason() {
        let Outcome::Failed(why) = decide("battle.run();", &scenarios::hurt_trainer()) else {
            panic!("there is no running from a trainer battle")
        };
        assert!(why.contains("can_run"), "{why}");
    }

    /// The cartridge's spelling is not the one a model types.
    #[test]
    fn an_item_is_matched_however_the_model_spells_it() {
        let state = scenarios::catchable_wild();
        for spelling in [r#""POKE BALL""#, r#""poke ball""#, r#""PokeBall""#, r#""Poké Ball""#] {
            let outcome = decide(&format!("battle.use_item({spelling});"), &state);
            assert!(
                matches!(outcome, Outcome::Action(BattleAction::UseItem { .. })),
                "{spelling} did not reach a Poké Ball: {outcome:?}",
            );
        }
    }

    /// `battle.ask()` keeps the script armed and hands one turn back.
    #[test]
    fn asking_hands_the_turn_back_without_being_a_failure() {
        assert_eq!(decide("battle.ask();", &wild()), Outcome::Ask);
    }

    /// Prints are the model's only window into what its own script did.
    #[test]
    fn prints_are_captured_and_capped() {
        let evaluation = run(r#"print("hello"); battle.run();"#, &wild(), 1);
        assert_eq!(evaluation.prints, vec!["hello".to_string()]);

        let flooded = run(r#"for i in 0..500 { print("x"); } battle.run();"#, &wild(), 1);
        assert_eq!(flooded.prints.len(), MAX_PRINTS, "a script that prints in a loop is capped");
        assert_eq!(flooded.outcome, Outcome::Action(BattleAction::Run), "and still decides the turn");
    }

    /// The facts the script reads are the ones the turn was built from, and the two must agree.
    #[test]
    fn the_facts_are_the_ones_the_game_is_actually_in() {
        let state = scenarios::hurt_wild();
        let battle = state.battle.as_ref().unwrap();
        let expected = battle.player.current_hp as f64 / battle.player.stats.hp as f64;
        assert!(expected < 0.1, "the scenario is meant to be badly hurt, got {expected}");

        let outcome = decide(
            "if battle.me.hp_frac < 0.1 { battle.run(); } battle.fight(battle.best_move);",
            &state,
        );
        assert_eq!(outcome, Outcome::Action(BattleAction::Run));
    }

    /// A bench member's moves are scored against the foe that is out; the numbers exist and differ.
    #[test]
    fn a_bench_members_moves_are_scored_against_the_current_foe() {
        let outcome = decide(
            r#"
            let best = ();
            for mon in battle.party {
                for mv in mon.moves {
                    if best == () || mv.damage > best.damage { best = mv; }
                }
            }
            if best.damage > 0 { battle.run(); }
            battle.ask();
            "#,
            &wild(),
        );
        assert_eq!(outcome, Outcome::Action(BattleAction::Run), "some move in the party does damage");
    }

    #[test]
    fn no_usable_damaging_move_leaves_best_move_unset() {
        let outcome = decide(
            "if battle.best_move == () { battle.run(); } battle.fight(battle.best_move);",
            &scenarios::out_of_pp(),
        );
        assert_eq!(outcome, Outcome::Action(BattleAction::Run));
    }

    /// A ghost reads as hopeless from every field, not only from the options `resolve` checks.
    #[test]
    fn a_ghost_leaves_best_move_unset_and_every_move_unusable() {
        let state = scenarios::ghost();
        let guard = "if battle.best_move == () { if battle.can_run { battle.run(); } battle.ask(); }\n\
                     battle.fight(battle.best_move);";
        assert_eq!(decide(guard, &state), Outcome::Action(BattleAction::Run), "the documented guard has to work on a ghost");

        let every_field = "for mv in battle.moves { if mv.usable || mv.damage > 0 || mv.effectiveness > 0.0 { battle.fight(mv); } }\n\
                           for mon in battle.party { for mv in mon.moves { if mv.usable || mv.damage > 0 { battle.switch_to(mon); } } }\n\
                           if battle.ghost && battle.can_run { battle.run(); }\n\
                           battle.ask();";
        assert_eq!(decide(every_field, &state), Outcome::Action(BattleAction::Run), "no field may make a ghost look attackable");

        // An ordinary wild battle still has `usable` moves and a `best_move`.
        assert!(matches!(decide("battle.fight(battle.best_move);", &wild()), Outcome::Action(BattleAction::Fight { .. })));
        assert_eq!(decide("if battle.ghost { battle.run(); } battle.ask();", &wild()), Outcome::Ask);
    }

    /// `usable` is the options list, not the move's own PP.
    #[test]
    fn a_script_that_fights_whatever_has_pp_is_refused_by_the_ghost_scenario() {
        let mut script = BattleScript::open(None);
        let answer = script.set(Some("for mv in battle.moves { if mv.pp > 0 { battle.fight(mv); } }\nbattle.ask();"), Some("a test"));
        assert!(!script.armed(), "it must not arm: {answer}");
        assert!(answer.contains("ghost in Pokemon Tower"), "the answer names the scenario:\n{answer}");
        assert!(answer.contains("Usable now: nothing"), "and says nothing could be used:\n{answer}");
    }

    /// The sandbox has no way out.
    #[test]
    fn a_script_cannot_reach_the_machine() {
        for attempt in [
            r#"open("/etc/passwd");"#,
            r#"import "std" as std;"#,
            r#"eval("battle.run()");"#,
            r#"print(timestamp());"#,
        ] {
            let outcome = decide(attempt, &wild());
            assert!(matches!(outcome, Outcome::Failed(_)), "`{attempt}` must not work: {outcome:?}");
        }
    }

    /// The table shows the model what its own rules do on seven turns.
    #[test]
    fn validation_arms_a_good_script_and_shows_what_it_chose() {
        let mut script = BattleScript::open(None);
        let answer = script.set(Some(
            "if battle.me.hp_frac < 0.1 && battle.can_run { battle.run(); }\n\
             if battle.best_move == () { battle.ask(); }\n\
             battle.fight(battle.best_move);",
        ), Some("a test"));
        assert!(script.armed(), "a script that passes every scenario is armed: {answer}");
        for (name, _) in SCENARIOS {
            assert!(answer.contains(name), "the table is missing `{name}`:\n{answer}");
        }
        // The rows are the report's verb phrases, not `BattleAction`'s menu rows.
        assert!(answer.contains("tried to run"), "the low-hp wild scenario should have fled:\n{answer}");
    }

    /// Compiling is not evidence.
    #[test]
    fn a_script_that_only_works_sometimes_is_not_armed() {
        let mut script = BattleScript::open(None);
        let answer = script.set(Some("battle.fight(battle.best_move);"), Some("a test"));
        assert!(!script.armed(), "it must not arm: {answer}");
        assert!(answer.contains("no damaging pp left"), "the answer names the scenario:\n{answer}");
        assert!(script.is_default(), "a refused script must not be stored: the default is left where it was");
    }

    #[test]
    fn a_script_that_does_not_compile_says_why() {
        let mut script = BattleScript::open(None);
        let answer = script.set(Some("if battle.me.hp_frac < { battle.run("), Some("a test"));
        assert!(!script.armed());
        assert!(answer.starts_with("Not armed"), "{answer}");
        assert!(answer.len() > 40, "the parse error has to reach the model: {answer}");
    }

    /// One strike.
    #[test]
    fn a_failure_disarms_but_keeps_the_script() {
        let mut script = BattleScript::open(None);
        let answer = script.set(Some(ALWAYS_DECIDES), Some("a test"));
        assert!(script.armed(), "{answer}");

        script.disarm("it chose a move BULBASAUR does not know");
        assert!(!script.armed(), "one failure is enough");
        assert_eq!(script.source(), Some(ALWAYS_DECIDES), "the source is what the model has to edit");
        assert!(script.read().contains("Disarmed"), "read_battle_script says so: {}", script.read());
        assert!(script.read().contains("does not know"), "and why: {}", script.read());
    }

    /// A restarted process fights the next battle the way the model said.
    #[test]
    fn a_script_survives_the_process_that_wrote_it() {
        let scratch = Scratch::new("battle-script");
        let source = "if battle.can_run && battle.me.hp_frac < 0.2 { battle.run(); }\n\
                      if battle.best_move == () { battle.ask(); }\n\
                      battle.fight(battle.best_move);";

        let mut written = BattleScript::open(Some(&scratch.0));
        written.set(Some(source), Some("a test"));
        assert!(written.armed());

        written.record_decided(340);

        let reopened = BattleScript::open(Some(&scratch.0));
        assert_eq!(reopened.source(), Some(source), "byte for byte");
        assert!(reopened.armed(), "and still deciding turns");

        // The standing makes the trip too.
        assert_eq!(
            reopened.standing(),
            ScriptStanding { purpose: Some("a test".into()), decided: 340, failure: None },
            "the purpose and the tally survive the process that wrote them",
        );

        // A disarm is durable too, or a restart re-arms a script that is known to be broken.
        let mut written = BattleScript::open(Some(&scratch.0));
        written.disarm("it ran out of operations");
        let reopened = BattleScript::open(Some(&scratch.0));
        assert!(!reopened.armed());
        // A disarm keeps them.
        assert_eq!(reopened.standing().purpose.as_deref(), Some("a test"));
        assert_eq!(reopened.standing().decided, 340);
        // And the reason, which rides on every overworld turn while the script is broken.
        assert_eq!(reopened.standing().failure.as_deref(), Some("it ran out of operations"));
    }

    /// The disarm reason has two lifetimes and needs two carriers, which is the trap this guards.
    #[test]
    fn a_disarm_reason_outlives_the_worker_taking_it() {
        let live = Live::default();
        live.arm(Some(ALWAYS_DECIDES.to_string()), ScriptState::Armed, ScriptStanding {
            purpose: Some("a test".into()), decided: 12, failure: None,
        });

        live.failed("it named a move the Pokémon does not know");
        assert_eq!(live.state(), ScriptState::Disarmed);
        assert!(live.source().is_none(), "nothing may consult it again");

        // The worker drains it once, to persist it.
        assert_eq!(live.take_failure().as_deref(), Some("it named a move the Pokémon does not know"));
        assert_eq!(live.take_failure(), None, "taken once, not once a turn");

        // And every overworld turn after that still has it.
        let standing = live.standing();
        assert_eq!(standing.failure.as_deref(), Some("it named a move the Pokémon does not know"));
        // The purpose and the tally are kept rather than cleared.
        assert_eq!(standing.purpose.as_deref(), Some("a test"));
        assert_eq!(standing.decided, 12);

        // Arming a replacement clears it, since it is a fact about the script it replaced.
        live.arm(Some(ALWAYS_DECIDES.to_string()), ScriptState::Armed, ScriptStanding::default());
        assert_eq!(live.standing().failure, None);
    }

    /// Cut with an ellipsis, and only in the standing.
    #[test]
    fn a_long_disarm_reason_is_cut_for_the_turn_but_not_for_the_file() {
        let scratch = Scratch::new("battle-script");
        let mut script = BattleScript::open(Some(&scratch.0));
        script.set(Some(ALWAYS_DECIDES), Some("a test"));
        // `é` throughout: the game's own names are full of it and a byte slice here panics.
        let long = "é".repeat(MAX_FAILURE * 2);
        script.disarm(&long);

        assert_eq!(script.last_failure(), Some(long.as_str()), "the file keeps the whole of it");
        let cut = script.standing().failure.expect("the turn gets one");
        assert_eq!(cut.chars().count(), MAX_FAILURE + 1, "cut to the cap plus the ellipsis");
        assert!(cut.ends_with('…'), "and says that it was cut: {cut}");
    }

    /// Unsetting goes back to [`DEFAULT`], not to nothing.
    #[test]
    fn unsetting_goes_back_to_the_default() {
        let scratch = Scratch::new("battle-script");
        let mut script = BattleScript::open(Some(&scratch.0));
        script.set(Some(ALWAYS_DECIDES), Some("a test"));
        let answer = script.set(None, None);
        assert!(!script.armed(), "the default is never armed: it decides nothing");
        assert!(script.is_default());
        assert_eq!(script.state(), ScriptState::Unedited);
        assert!(script.live_source().is_none(), "and the policy is never handed it");
        assert!(answer.starts_with("ok"), "{answer}");
        let reopened = BattleScript::open(Some(&scratch.0));
        assert!(reopened.is_default(), "and on disk");
    }

    #[test]
    fn an_oversized_script_is_refused_without_disturbing_the_one_that_works() {
        let mut script = BattleScript::open(None);
        script.set(Some(ALWAYS_DECIDES), Some("a test"));
        let answer = script.set(Some(&"// padding\n".repeat(MAX_SOURCE)), Some("a test"));
        assert!(answer.contains(&MAX_SOURCE.to_string()), "{answer}");
        assert_eq!(script.source(), Some(ALWAYS_DECIDES), "the armed script is untouched");
        assert!(script.armed());
    }

    /// An unreadable file is the default script, not a refusal to play.
    #[test]
    fn a_corrupt_file_starts_on_the_default_rather_than_failing() {
        let scratch = Scratch::new("battle-script");
        std::fs::write(scratch.0.join(files::BATTLE_SCRIPT), b"{ not json").unwrap();
        let script = BattleScript::open(Some(&scratch.0));
        assert!(!script.armed());
        assert!(script.is_default());
    }

    /// A run written before there was a default is carried across it.
    #[test]
    fn a_run_from_before_the_default_is_brought_onto_it() {
        let scratch = Scratch::new("battle-script");
        let path = scratch.0.join(files::BATTLE_SCRIPT);
        std::fs::write(&path, br#"{"source":null,"armed":false,"last_failure":null}"#).unwrap();
        let script = BattleScript::open(Some(&scratch.0));
        assert!(script.is_default(), "an empty file is the default now");
        assert_eq!(script.state(), ScriptState::Unedited);

        std::fs::write(&path, br#"{"source":"battle.run();","armed":false,"last_failure":"it fled"}"#).unwrap();
        let broken = BattleScript::open(Some(&scratch.0));
        assert_eq!(broken.source(), Some("battle.run();"));
        assert_eq!(broken.state(), ScriptState::Disarmed);
        assert_eq!(broken.last_failure(), Some("it fled"));
    }

    /// `live_source` withholds the default, so only this test runs it.
    #[test]
    fn the_default_script_compiles_and_hands_every_turn_back() {
        let table = validate(DEFAULT).expect("the default script must validate");
        for (name, _) in SCENARIOS {
            assert!(table.contains(name), "the table is missing `{name}`:\n{table}");
        }
        assert_eq!(
            table.matches("hands the turn to you").count(),
            SCENARIOS.len(),
            "every scenario, not just most of them:\n{table}"
        );
    }

    /// The default never reaches the emulator thread.
    #[test]
    fn the_default_is_never_handed_to_the_policy() {
        let script = BattleScript::open(None);
        assert!(script.is_default(), "a fresh run starts on it");
        assert!(script.live_source().is_none(), "but the policy is handed nothing");
        assert!(!script.armed(), "and nothing tells the model or the page that battles are free");
        assert_eq!(script.state(), ScriptState::Unedited);

        // A script the model wrote is handed over.
        let mut script = BattleScript::open(None);
        script.set(Some(ALWAYS_DECIDES), Some("a test"));
        assert!(script.armed());
        assert_eq!(script.live_source().as_deref(), Some(ALWAYS_DECIDES));
        assert_eq!(script.state(), ScriptState::Armed);
    }

    /// `read_battle_script` always has a file to show: the model edits rather than invents.
    #[test]
    fn reading_a_fresh_runs_script_answers_with_the_default_source() {
        let answer = BattleScript::open(None).read();
        assert!(answer.contains("default script"), "{answer}");
        assert!(answer.contains("battle.ask()"), "the source itself, to edit: {answer}");
        assert!(answer.contains("set_battle_script"), "and what to do about it: {answer}");
    }

    /// The deterministic policy's strategy, the one script known to finish the game, must arm.
    #[test]
    fn the_deterministic_strategy_still_arms_and_still_plays() {
        let mut script = BattleScript::open(None);
        let answer = script.set(Some(DETERMINISTIC), Some("a test"));
        assert!(script.armed(), "the bundled strategy no longer arms:\n{answer}");

        for (scenario, expected) in [
            ("full-hp wild", "used Ember"),
            ("low-hp wild", "used a Potion"),
            ("low-hp trainer, bench healthy", "used a SuperPotion"),
            ("no damaging pp left", "tried to run"),
        ] {
            let row = answer
                .lines()
                .find(|line| line.trim_start().starts_with(scenario))
                .unwrap_or_else(|| panic!("no `{scenario}` row in:\n{answer}"));
            assert!(row.contains(expected), "`{scenario}` should have {expected}: {row}");
        }

        // The known-good strategy has to fit under the size cap.
        assert!(DETERMINISTIC.len() < MAX_SOURCE, "it is {} bytes against {MAX_SOURCE}", DETERMINISTIC.len());
    }

    /// The docs stay in the context once read, so they are bounded like a guide chapter.
    #[test]
    fn the_docs_stay_within_what_they_cost_to_carry() {
        assert!(DOCS.len() < 9_500, "the docs are {} bytes", DOCS.len());
        for name in ["battle.fight", "battle.switch", "battle.use_item", "battle.run", "battle.ask"] {
            assert!(DOCS.contains(name), "the docs never mention {name}");
        }
        assert!(DOCS.contains("hp_frac"), "the field every script branches on is undocumented");
        assert!(DOCS.contains("effectiveness"), "the reason no type chart is needed is undocumented");
    }

    /// The docs make claims about the language, and a wrong one costs a round trip.
    #[test]
    fn the_language_the_docs_promise_is_the_language_the_engine_runs() {
        let state = wild();
        for (what, code) in [
            ("for over an array", "for mon in battle.party { } battle.ask();"),
            ("while", "let i = 0; while i < 3 { i += 1; } battle.ask();"),
            ("loop and break", "let i = 0; loop { i += 1; if i > 2 { break; } } battle.ask();"),
            ("switch expression", r#"let t = switch battle.turn { 1 => "a", _ => "b" }; battle.ask();"#),
            ("array len and index", "let n = battle.party.len; let p = battle.party[0]; battle.ask();"),
            ("filter with a closure", "let a = battle.party.filter(|p| !p.fainted); battle.ask();"),
            ("map with a closure", "let a = battle.moves.map(|m| m.damage); battle.ask();"),
            ("reduce with a closure", "let n = battle.moves.reduce(|s, m| s + m.damage, 0); battle.ask();"),
            ("sort with a closure", "let a = battle.moves; a.sort(|x, y| y.damage - x.damage); battle.ask();"),
            ("push", "let a = []; a.push(1); battle.ask();"),
            ("object literal", "let m = #{ a: 1 }; let v = m.a; let w = m[\"a\"]; battle.ask();"),
            ("string concat and methods", r#"let s = "a" + 1; let c = s.contains("a"); battle.ask();"#),
            ("float division", "let f = 7.0 / 2.0; battle.ask();"),
            ("fn with an argument", "fn f(b) { b.turn } let x = f(battle); battle.ask();"),
            ("early return in a fn", "fn f(n) { if n > 0 { return 1; } 0 } let x = f(1); battle.ask();"),
            ("compound assignment", "let x = 3; x += 1; x *= 2; battle.ask();"),
        ] {
            assert_eq!(decide(code, &state), Outcome::Ask, "the docs promise `{what}` works");
        }

        // And the one the docs warn about, which has to keep failing.
        let Outcome::Failed(why) = decide("fn f() { battle.turn } let x = f(); battle.ask();", &state)
        else { panic!("a `fn` must not see `battle`; the docs devote a numbered point to it") };
        assert!(why.contains("battle"), "and the reason has to name it: {why}");
    }

    /// Every field the docs tabulate exists, and every field is tabulated.
    #[test]
    fn the_docs_tabulate_every_field_an_object_actually_has() {
        let state = scenarios::catchable_wild();
        for (path, field) in [("battle.bag[0]", "name"), ("battle.bag[0]", "count")] {
            assert_eq!(
                decide(&format!("let x = {path}.{field}; battle.ask();"), &state),
                Outcome::Ask,
                "`{path}.{field}` does not exist",
            );
            assert!(DOCS.contains(&format!("`{field}`")), "`{field}` on an Item is undocumented");
        }
        // The game's own names: `use_item` normalises, but a script comparing `item.name` does not.
        for name in ["Potion", "SuperPotion", "PokeBall"] {
            assert!(DOCS.contains(name), "the docs name no `{name}` for a script to compare against");
        }
        assert!(DOCS.contains("battle.bag"), "the bag is undocumented");
    }
    /// A new script's tally starts at zero.
    #[test]
    fn a_new_script_is_for_something_and_has_decided_nothing_yet() {
        let mut script = BattleScript { path: None, saved: Saved::default() };
        let answer = script.set(Some("if battle.best_move == () { battle.ask(); }\nbattle.fight(battle.best_move);"), Some("  Misty's Staryu  "));
        assert!(answer.starts_with("ok, armed"), "{answer}");

        let standing = script.standing();
        // Trimmed, and otherwise stored as written.
        assert_eq!(standing.purpose.as_deref(), Some("Misty's Staryu"));
        assert_eq!(standing.decided, 0, "a script that has not run has decided nothing");

        script.record_decided(340);
        assert_eq!(script.standing().decided, 340);

        // Replaced: the new one is for something else and has done nothing.
        script.set(Some("// a different one\nif battle.best_move == () { battle.ask(); }\nbattle.fight(battle.best_move);"), Some("fleeing everything in the tower"));
        assert_eq!(script.standing().decided, 0, "the tally belongs to the script, not to the run");
        assert_eq!(script.standing().purpose.as_deref(), Some("fleeing everything in the tower"));

        // Back to the default is not a script and is for nothing — both fields go with it.
        script.record_decided(12);
        script.set(None, None);
        assert_eq!(script.standing(), ScriptStanding::default());
    }

    /// Capped: it is re-sent on every overworld turn while the script is armed.
    #[test]
    fn a_purpose_that_runs_long_is_cut_rather_than_refused() {
        let mut script = BattleScript { path: None, saved: Saved::default() };
        // Multi-byte on purpose: the prose is full of `é` and a byte slice would panic here.
        let long = "é".repeat(MAX_PURPOSE * 2);
        let answer = script.set(Some("if battle.best_move == () { battle.ask(); }\nbattle.fight(battle.best_move);"), Some(&long));
        assert!(answer.starts_with("ok, armed"), "the script is still installed: {answer}");

        let purpose = script.standing().purpose.expect("a purpose was given");
        assert_eq!(purpose.chars().count(), MAX_PURPOSE + 1, "cut to the cap plus the ellipsis");
        assert!(purpose.ends_with('…'), "and says it was cut: {purpose}");
    }

}
