//! The model's battle script: a sandboxed program that decides battle turns without a request.
//!
//! ```text
//! $GB_RUN_DIR/<run-id>/battle-script.json      { source, armed, last_failure }
//! ```
//!
//! Battles are where this run's tokens go and where its play is worst. The deployed run of
//! 2026-08-26 made **204 battle decisions of which 31 were `run` and none was a Poké Ball**, and
//! reached Mt Moon on 92 minutes of cartridge time with a single Lv19 starter as its whole party.
//! Every one of those 204 decisions was a full request against a ~50 k-token history, to answer a
//! question that is usually mechanical: hit the thing with the move that does the most damage.
//!
//! So the model writes the mechanical answer down once and the policy runs it.
//! [`crate::pokemon::llm_policy::LlmPolicy::pick_battle_action`] evaluates the script **on the
//! emulator thread**, so a scripted turn returns on the first poll: no request, no round trip, no
//! latency. Paired with `resume_after_battle`, a wild encounter interrupting a walk costs nothing
//! at all.
//!
//! ⚠️ **The script filters [`crate::pokemon::policy::battle_options`]; it never invents an action.**
//! That function is the one legal-action chokepoint `RandomPolicy`, `DeterministicPolicy`,
//! `ConsolePolicy` and `tools::battle_menu` already share, and
//! `postgame::{safari,fishing,legendaries}::pick_battle_action(state, …, &actions)` is the existing
//! pluggable-strategy shape. A choice that is not on that list is a **script failure with a named
//! reason**, never a silently dropped turn — the same rule `tools::classify` follows for an id the
//! model invented.
//!
//! ⚠️ **The safety of this is the engine's limits, not the language.** The source is written by a
//! model and runs on the thread that owns the `GameBoy`, so [`engine`] sets an operation cap, a
//! wall-clock abort, string/array/map ceilings and a call-depth limit, and `Cargo.toml` never
//! enables rhai's `unchecked`. Rhai has no file, process or network API to disable. On top of that
//! [`run`] wraps the evaluation in `catch_unwind`, the way `web::audio` wraps the Opus encoder and
//! for the same reason: a panic on this thread would take the run's checkpoint with it.
//!
//! ⚠️ **Nothing here is cached and that is deliberate.** [`run`] builds an engine, compiles and
//! evaluates, every call. A cached `AST` would have to be invalidated when the source changed, and
//! the failure mode of getting that wrong is a run quietly fighting every battle with a script the
//! model replaced an hour ago. Measured against what it saves — a whole HTTP request — a compile of
//! a thirty-line script is not worth the invalidation bug.
//!
//! ⚠️ **The choice cell is the authority, not the abort.** Every action function records its choice
//! and then aborts evaluation, which is what makes "calling an action ends the script" true rather
//! than a convention the docs ask for. But rhai has `try`/`catch`, so a script *can* swallow that
//! abort — in which case the first recorded choice still stands and any later one is ignored. The
//! rule is "the first action wins", enforced by the cell, and it holds however the script is
//! written.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rhai::{Array, Dynamic, Engine, EvalAltResult, Map, Position};

use crate::pokemon::GameState;
use crate::pokemon::battle::{BattleAction, BattleType};
use crate::pokemon::damage::expected_damage;
use crate::pokemon::pokemon::PokemonSummary;
use crate::pokemon::policy::battle_options;
use crate::run::files;

/// The docs `get_battle_script_docs` answers with, sent **verbatim**.
///
/// Ordinary markdown, `include_str!`'d, with no template and nothing rendered — exactly what
/// [`crate::llm::guide`] does with a chapter, and for the same reason: what is on disk is what the
/// model is sent, so reviewing the file is reviewing the answer.
pub const DOCS: &str = include_str!("battle_script/DOCS.md");

/// The deterministic policy's battle strategy, written as a battle script.
///
/// ⚠️ **A worked example that is known to play the game, rather than one invented for the docs.** It
/// is `DeterministicPolicy::pick_battle_action` with the route-plan arms removed — the strategy
/// `full_playthrough` finishes the game with — and
/// `the_deterministic_strategy_still_arms_and_still_plays` keeps it compiling and choosing as this
/// module changes underneath it. It is deliberately **not** `include_str!`'d into `DOCS.md`: the
/// docs are carried in the model's context and this is 120 lines, so it is offered only if the
/// model asks for it and used here as a test.
pub const DETERMINISTIC: &str = include_str!("battle_script/DETERMINISTIC.rhai");

/// The script every run starts with, and the one a `set_battle_script` with no `script` goes back
/// to. It calls `battle.ask()` and nothing else, so it decides no turns and the run behaves exactly
/// as it did when the answer to "is there a script" was "no".
///
/// ⚠️ **The point is that the artefact exists, not that it does anything.** Two deployed runs never
/// called `set_battle_script` once — 207 battle turns and 22.3 M prompt tokens in the run of
/// 2026-08-27 — and `get_battle_script_docs` was never called either, so this was never weighed and
/// rejected, it was never reached. A blank page asks the model to invent a file; a default asks it
/// to edit one, which is a smaller step and the one `read_battle_script` can actually show it. That
/// tool used to answer "There is no battle script", which is a round trip spent learning nothing.
///
/// ⚠️ **It is deliberately not a strategy.** `DETERMINISTIC` is right here and is known to finish
/// the game, and shipping it as the default would make every run's battles somebody else's play
/// rather than the model's, which is the thing the run exists to measure. The comments point at the
/// docs instead of restating them, so the worked example cannot drift into a second copy.
///
/// ⚠️ **It never reaches [`Live`] and so is never evaluated.** `battle.ask()` and "no script" are
/// the same outcome, so [`BattleScript::live_source`] withholds it and the emulator thread's battle
/// path is byte-for-byte what it was: no engine built per battle turn, no failure surface, and — the
/// half that matters — no `self.note`, which would otherwise be set on every battle turn and
/// suppress the very `TurnContext::Battle` line this exists to keep showing.
pub const DEFAULT: &str = include_str!("battle_script/DEFAULT.rhai");

/// How long a script may be. Generous — it is written once and re-read only when the model asks for
/// it — but bounded, because it is sent back whole by `read_battle_script` and quoted in full in
/// every validation failure.
pub const MAX_SOURCE: usize = 6_000;

/// The fuel. Rhai counts operations and terminates when this is reached, which is the guard
/// `catch_unwind` cannot provide: a panic can be caught, a loop cannot.
///
/// ⚠️ **Sized against the emulator's tick budget, not against what a script needs.** A battle
/// decision is a scan of at most six party members and four moves each, so a few hundred
/// operations; 20 000 is two orders of magnitude of headroom for a model that likes writing
/// helpers. It matters that it is finite rather than what the number is.
pub const MAX_OPERATIONS: u64 = 20_000;

/// The wall-clock abort, checked from rhai's progress hook.
///
/// ⚠️ **Belt and braces on top of [`MAX_OPERATIONS`], and not redundant.** An operation count is a
/// bound on work, not on time: one operation on a very large string or array is not one operation's
/// worth of wall clock, and `host.rs`'s `MAX_CATCHUP` starts dropping emulated time at 250 ms.
pub const MAX_RUNTIME: Duration = Duration::from_millis(50);

/// How many `print` lines are carried back to the model, and how long each may be. A script that
/// prints in a loop is debugging itself at the model's expense.
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
    /// A legal action, already resolved against [`battle_options`].
    Action(BattleAction),
    /// `battle.ask()` — the script wants the model to answer this particular turn, and stays armed.
    Ask,
    /// The script did not produce an action. The string is shown to the model verbatim, so it says
    /// what went wrong rather than naming a variant.
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
///
/// Symbolic on purpose: the script names a move or a Pokémon, and [`resolve`] is what turns that
/// into a [`BattleAction`] that is actually on the menu this turn. Nothing the script says reaches
/// the game without going through that.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Choice {
    Fight(Ref),
    Switch(Ref),
    Item(String),
    Run,
    Ask,
}

/// How the script named something: by the `slot` off the object it was handed, or by name.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ref {
    Slot(i64),
    Name(String),
}

impl Ref {
    /// Read a reference out of whatever the script passed: one of our own objects (which carries
    /// its `slot`), a bare integer, or a name.
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
        // ⚠️ **`()` gets its own sentence, because it is the mistake a model actually makes.**
        // `battle.fight(battle.best_move)` is the first line anyone writes, and `best_move` is `()`
        // exactly when nothing in the moveset can hurt the foe — which is also the turn on which
        // getting it right matters most. Told only "got a ()" a model has to work out where the
        // unit came from; told this, it has the guard to add.
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
///
/// ⚠️ **Because the cartridge's spelling is not the one a model will type.** `ItemId`'s `Display` is
/// strum's, so a Poké Ball is `PokeBall`; a move is `Karate Chop`; a nickname is whatever the model
/// chose. Matching literally would refuse `"POKE BALL"`, `"poke ball"` and `"Pokeball"` — three
/// spellings of the thing the run most needs to start using.
fn normalised(name: &str) -> String {
    name.chars()
        // ⚠️ `é` is alphanumeric, so filtering on that alone keeps it and `POKÉBALL` never equals
        // `POKEBALL`. It is the only accent the cartridge has and the only one this needs.
        .map(|c| match c {
            'é' | 'É' | 'è' | 'È' => 'E',
            other => other,
        })
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_uppercase)
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The context the script sees
// ---------------------------------------------------------------------------------------------

/// The `battle` global. A handle onto the shared choice cell and nothing else — every *fact* is
/// pre-built into rhai maps at construction, so a getter is a clone rather than a read of live game
/// state. That is what keeps the script's view of the turn consistent with itself: `battle.me` read
/// twice is the same Pokémon, whatever the emulator did in between.
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
    ///
    /// The first call wins: a later one is dropped rather than overwriting, so a script that
    /// catches the abort and carries on still commits to the action it committed to first.
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
            // (the message stands alone: `describe` prefixes nothing, so `why` says which value)
        }
    }
}

/// The engine every script is compiled and run by.
///
/// ⚠️ **One builder, used by the live evaluation *and* by `set_battle_script`'s validation.** If
/// validation ran on a differently-configured engine it would be proving something about a program
/// that is never executed, which is worse than not validating at all.
fn engine(deadline: Instant, prints: Rc<RefCell<Vec<String>>>) -> Engine {
    let mut engine = Engine::new();

    engine.set_max_operations(MAX_OPERATIONS);
    engine.set_max_call_levels(32);
    engine.set_max_expr_depths(64, 32);
    engine.set_max_string_size(8 * 1024);
    engine.set_max_array_size(512);
    engine.set_max_map_size(512);
    // Nested `eval` of a string built at run time is a second parser invocation the limits above
    // are not accounted against, and nothing a battle script needs.
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
    // `debug` is rhai's other output channel and a model will reach for it. Sending it to the same
    // place costs nothing and beats a print that silently goes nowhere.
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
        // ⚠️ **`switch_to`, and it wanted to be `switch`.** `switch` is a statement keyword in rhai
        // and is reserved even in method position, so `battle.switch(x)` is a *parse* error that no
        // amount of registering can fix. Same trap as `move_type` above, and
        // `every_name_the_docs_use_is_one_the_parser_accepts` is what stops a third one shipping.
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

// ---------------------------------------------------------------------------------------------
// Building the facts
// ---------------------------------------------------------------------------------------------

/// One move, as the script sees it.
///
/// ⚠️ **`damage` and `effectiveness` are the whole reason this is affordable to write.** Without
/// them a script has to carry a type chart — hundreds of lines the model has to get right from
/// memory, in a file it is charged for storing and cannot test. Both are thin wrappers over
/// `damage::expected_damage` and `PokemonType::attack_effectiveness`, which the deterministic
/// policy has used since long before this existed.
fn move_map(slot: usize, battle_move: &crate::pokemon::move_name::PokemonMove, me: &PokemonSummary, foe: &PokemonSummary, disabled: bool) -> Map {
    let metadata = battle_move.name.metadata();
    let mut map = Map::new();
    map.insert("slot".into(), Dynamic::from(slot as i64));
    map.insert("name".into(), Dynamic::from(battle_move.name.to_string()));
    // ⚠️ **`move_type`, and it wanted to be `type`.** `type` is a reserved word in rhai, so
    // `mv.type` is a *parse* error — which a model writes on its first attempt and pays a whole
    // round trip to find out. The key is spelled the way the docs spell it, and the docs say why.
    map.insert("move_type".into(), Dynamic::from(metadata.move_type.to_string()));
    map.insert("power".into(), Dynamic::from(metadata.power.unwrap_or(0) as i64));
    map.insert("accuracy".into(), Dynamic::from(metadata.accuracy as i64));
    map.insert("pp".into(), Dynamic::from(battle_move.pp as i64));
    map.insert("max_pp".into(), Dynamic::from(metadata.pp as i64));
    map.insert("damage".into(), Dynamic::from(expected_damage(me, battle_move.name, foe).unwrap_or(0) as i64));
    map.insert("effectiveness".into(), Dynamic::from(crate::pokemon::damage::type_multiplier(battle_move.name, foe)));
    map.insert("usable".into(), Dynamic::from(battle_move.pp > 0 && !disabled));
    map
}

/// One Pokémon, as the script sees it. `moves` is scored against `foe`, so a bench member's moves
/// carry the damage they *would* do — which is what a coverage switch is decided on.
fn pokemon_map(slot: usize, name: &str, mon: &PokemonSummary, foe: &PokemonSummary) -> Map {
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
            let disabled = mon.disabled_move_slot == Some(index as u8);
            Some(Dynamic::from(move_map(index, battle_move, mon, foe, disabled)))
        })
        .collect();
    map.insert("moves".into(), Dynamic::from(moves));
    map
}

/// ⚠️ **A healthy Pokémon says `""`, not `"None"`.** `PokemonStatus`' `Display` is strum's derive,
/// which is what put `20/20 HP, None` in front of the model in every party line for months. A
/// script would compare against it, so the same trap is closed here rather than only in `prompt`.
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
fn facts(state: &GameState, turn: u32) -> Option<Map> {
    let battle = state.battle.as_ref()?;
    let me = &battle.player;
    let foe = &battle.enemy;

    let mut map = Map::new();
    map.insert("kind".into(), Dynamic::from(match battle.battle_type {
        BattleType::Wild => "wild",
        BattleType::Trainer => "trainer",
        BattleType::Safari => "safari",
    }
    .to_string()));
    map.insert("turn".into(), Dynamic::from(turn as i64));
    map.insert("can_run".into(), Dynamic::from(battle.battle_type == BattleType::Wild));
    map.insert("trapped".into(), Dynamic::from(battle.enemy_trapping));
    map.insert("catch_rate".into(), Dynamic::from(battle.enemy_catch_rate as i64));

    let active = battle.active_party_slot as usize;
    let my_name = state
        .pokemon
        .iter()
        .nth(active)
        .map(|mon| mon.nickname.to_default_string())
        .unwrap_or_else(|| me.species.to_string());
    let me_map = pokemon_map(active, &my_name, me, foe);
    let my_moves = me_map.get("moves").cloned().unwrap_or(Dynamic::UNIT);
    map.insert("me".into(), Dynamic::from(me_map));
    // The foe is scored against *itself* for `damage`, which is meaningless — but the field has to
    // exist or `battle.foe.moves[0].damage` is a hard error rather than a number to ignore. What
    // matters is that the moves and their PP are there: they are read out of `wEnemyMon`, so a
    // script can see what it is up against.
    map.insert("foe".into(), Dynamic::from(pokemon_map(usize::MAX, &foe.species.to_string(), foe, me)));
    map.insert("moves".into(), my_moves);

    let party: Array = state
        .pokemon
        .iter()
        .enumerate()
        .map(|(slot, mon)| Dynamic::from(pokemon_map(slot, &mon.nickname.to_default_string(), &mon.summary(), foe)))
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

    // The highest-damage usable move, which is what most scripts want and none should have to write
    // twice. `()` when nothing can damage the foe at all — a real state, and one the script has to
    // handle, since it is exactly when switching is the right answer.
    let best = me
        .moves
        .iter()
        .enumerate()
        .filter_map(|(index, battle_move)| {
            let battle_move = battle_move.as_ref()?;
            if battle_move.pp == 0 || me.disabled_move_slot == Some(index as u8) {
                return None;
            }
            let damage = expected_damage(me, battle_move.name, foe)?;
            (damage > 0).then(|| (damage, move_map(index, battle_move, me, foe, false)))
        })
        .max_by_key(|(damage, _)| *damage);
    map.insert("best_move".into(), match best {
        Some((_, best)) => Dynamic::from(best),
        None => Dynamic::UNIT,
    });

    Some(map)
}

// ---------------------------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------------------------

/// Turn what the script said into an action the game will actually accept, or say why not.
///
/// ⚠️ **Every arm names what was wrong and what was available.** This message is the only thing the
/// model gets back before the script is disarmed, so "no such move" is a dead end and "`SURF` is
/// not one of BULBASAUR's moves; it knows Tackle, Growl, Vine Whip" is a fix.
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

// ---------------------------------------------------------------------------------------------
// Running one
// ---------------------------------------------------------------------------------------------

/// Evaluate `source` against the turn `state` describes, and resolve whatever it chose.
///
/// Never panics and never blocks: see the module's third and fourth ⚠️.
pub fn run(source: &str, state: &GameState, turn: u32) -> Evaluation {
    let Some(options) = battle_options(state) else {
        return Evaluation::failed("there is no battle to decide", Vec::new());
    };
    let Some(facts) = facts(state, turn) else {
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

    // ⚠️ **The cell is read before the error, always.** An action aborts evaluation on purpose, so
    // the *expected* path through here has an `Err` beside a perfectly good choice.
    if let Some(choice) = taken {
        return Evaluation { outcome: resolve(choice, state, &options), prints };
    }

    match evaluated {
        Ok(Ok(())) => Evaluation::failed(NO_ACTION, prints),
        Ok(Err(failure)) => Evaluation::failed(describe(&failure), prints),
        // The audio encoder's rule, one thread over: a panic here would unwind the emulator and
        // take the run's checkpoint with it, so it is caught and reported as an ordinary failure.
        // The script is disarmed by the caller either way, which is what makes it unrepeatable.
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
        // ⚠️ **Our own refusals come back through here, and rhai's `Display` prefixes them with
        // "Runtime error:".** They are sentences written for the model, not diagnostics, so the
        // prefix is dropped and only the position is kept — that is the half it cannot work out.
        EvalAltResult::ErrorRuntime(message, position) => match message.clone().into_string() {
            Ok(sentence) => format!("{sentence} (at {position})"),
            Err(_) => failure.to_string(),
        },
        other => other.to_string(),
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

// ---------------------------------------------------------------------------------------------
// The persisted script
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Saved {
    #[serde(default)]
    source: Option<String>,
    /// Whether the policy should consult it. A script that failed once is kept but not armed —
    /// see [`BattleScript::disarm`].
    #[serde(default)]
    armed: bool,
    #[serde(default)]
    last_failure: Option<String>,
}

/// ⚠️ **A run always has a script, and the one it starts with is [`DEFAULT`].** The empty state is
/// gone: there is no longer a difference between "no script" and "a script that asks every turn",
/// so the file always has a source and the model always has something to edit rather than something
/// to invent. ⚠️ **Per-field `#[serde(default)]` does not reach this** — it uses each field's own
/// `Default`, so a file written before this change deserialises with `source: None`, which is what
/// [`BattleScript::open`] normalises.
impl Default for Saved {
    fn default() -> Self {
        Self { source: Some(DEFAULT.to_string()), armed: true, last_failure: None }
    }
}

/// The script on disk, and the tool calls against it. Answered on the **worker thread**: validation
/// runs the script six times and none of it needs the emulator.
pub struct BattleScript {
    /// `None` for a run with no directory — the tests, and the in-process worker in `LlmPolicy`.
    path: Option<PathBuf>,
    saved: Saved,
}

impl BattleScript {
    /// Open the script in a run directory. Never fails: an unreadable file starts on [`DEFAULT`],
    /// on `TodoList::open`'s argument that refusing to play is worse than losing the thing.
    ///
    /// ⚠️ **A missing source is normalised to the default rather than left empty**, which is what
    /// carries a run written before there was a default across the change: `{"source": null}`
    /// deserialises to `None` through the per-field `#[serde(default)]`, and every reader below now
    /// assumes there is always a source. A *disarmed* script keeps its own source and is left alone.
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
    ///
    /// ⚠️ **The default does not count, and this is the one place that rule is made.** It is armed
    /// in the file — there is nothing else for the flag to say — but it decides nothing, so every
    /// reader downstream of this would otherwise tell the model and the page that the battles going
    /// past are free when the run is paying a full prefill for each of them. That is the exact
    /// direction the nudge cannot afford to be wrong in, and it is `false` here so that the turn
    /// line, `read_battle_script`, [`live_source`](Self::live_source) and the page's `armed` chip
    /// all inherit it rather than each remembering to ask.
    pub fn armed(&self) -> bool {
        self.saved.armed && self.saved.source.is_some() && !self.is_default()
    }

    /// Whether what is installed is [`DEFAULT`], untouched. Trimmed on both sides, because
    /// [`Self::set`] trims what it stores and a trailing newline is not an edit.
    pub fn is_default(&self) -> bool {
        self.saved.source.as_deref().map(str::trim) == Some(DEFAULT.trim())
    }

    pub fn last_failure(&self) -> Option<&str> {
        self.saved.last_failure.as_deref()
    }

    /// What [`Live`] should hold: the source only while the script is armed, since the policy runs
    /// whatever it is given. `None` for the default, which is [`armed`](Self::armed)'s doing and is
    /// what keeps the emulator thread's battle path exactly as it was.
    pub fn live_source(&self) -> Option<String> {
        self.armed().then(|| self.saved.source.clone()).flatten()
    }

    /// The one line a battle turn says about the script. Three states rather than two, because
    /// "yours is still the one we gave you" and "yours broke" want opposite sentences and the
    /// difference is invisible from the source alone: [`Live::failed`] drops the source the moment
    /// it fails, and the default never reaches [`Live`] at all.
    pub fn state(&self) -> ScriptState {
        match (self.armed(), self.is_default()) {
            (true, _) => ScriptState::Armed,
            // Only reachable through a real failure, which keeps the source it failed on.
            (false, false) => ScriptState::Disarmed,
            (false, true) => ScriptState::Unedited,
        }
    }

    /// `set_battle_script`. Validates before arming, and the answer is the validation table.
    pub fn set(&mut self, source: Option<&str>) -> String {
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
                self.saved = Saved { source: Some(source.to_string()), armed: true, last_failure: None };
                self.persist();
                format!("ok, armed. Validated on {} scenarios:\n{table}", SCENARIOS.len())
            }
            Err(why) => format!("Not armed, and nothing was changed. {why}"),
        }
    }

    /// `read_battle_script`.
    ///
    /// ⚠️ **There is no "there is no script" answer any more, and that is most of why [`DEFAULT`]
    /// exists.** This used to be able to spend a round trip saying only that the model had not
    /// written anything, which it already knew. It now always comes back with a file to edit.
    pub fn read(&self) -> String {
        let Some(source) = self.source() else {
            return "There is no battle script, which should not be possible. \
                    `set_battle_script` will install one."
                .to_string();
        };
        let state = match (self.armed(), self.is_default(), self.last_failure()) {
            (true, _, _) => "Armed. This is deciding your battle turns.".to_string(),
            (false, true, _) => "**This is the default script and it decides nothing** — it hands \
                                 every battle turn back to you, so each one costs a request. \
                                 Replace it with `set_battle_script`."
                .to_string(),
            (false, false, Some(why)) => format!("**Disarmed** after it failed: {why}\n\nFix it and call `set_battle_script` again, or pass `null` to go back to the default."),
            (false, false, None) => "Not armed.".to_string(),
        };
        format!("{state}\n\n```rhai\n{source}\n```")
    }

    /// The policy hit a failure. The script is kept — it is the thing the model has to edit — but
    /// it stops deciding turns until the model arms it again.
    ///
    /// ⚠️ **One strike, and not one battle.** A script that failed once will fail again, and each
    /// failure costs a whole request against the history to say so. Disarming for the rest of the
    /// battle only moves that cost to the next battle; disarming for one turn pays it every turn.
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

// ---------------------------------------------------------------------------------------------
// The cell between the two threads
// ---------------------------------------------------------------------------------------------

/// Whether a script is deciding battle turns, as the battle turn reports it.
///
/// ⚠️ **This is a fact about the run, not about this turn, and it is why the state is carried
/// rather than inferred.** A failure is reported once, by the note [`LlmPolicy`] writes on the turn
/// that caused it; every battle turn after that one said nothing at all, so a run whose script broke
/// on Route 3 spent the rest of its life paying for battle turns with no idea it had stopped being
/// free. The deployed run of 2026-08-27 is the other half of the same hole: **207 battle turns, and
/// `set_battle_script` was never called once** — nothing on a battle turn had ever mentioned that a
/// script was an option, and the argument for one lives in the system prompt, which is the least
/// recent thing in every request.
///
/// [`LlmPolicy`]: crate::pokemon::llm_policy::LlmPolicy
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScriptState {
    /// [`DEFAULT`] is installed, untouched: a script exists but decides nothing, so battle turns
    /// cost what they always did.
    ///
    /// ⚠️ **Named for what is true rather than for what used to be.** This was `Unset`, back when a
    /// run could genuinely have no script; a variant still called that while every run has one is
    /// exactly the drift that costs the next reader an hour.
    #[default]
    Unedited,
    /// Armed and deciding battle turns — so a turn carrying this is one it did not decide.
    Armed,
    /// One was written and has stopped deciding turns. The source is kept, because it is the thing
    /// the model has to edit; `read_battle_script` says why it stopped.
    Disarmed,
}

/// The armed script, shared between the worker thread that writes it and the emulator thread that
/// runs it.
///
/// ⚠️ **Two directions, and both are needed.** The worker arms and disarms deliberately, in
/// response to `set_battle_script`; the policy disarms *because a battle went wrong*, and the file
/// on disk has to learn about that or a restart re-arms a script already known to be broken. So the
/// failure travels back through here and [`crate::llm::worker::Worker::run_one`] drains it into
/// [`BattleScript::disarm`] at the top of the next turn.
///
/// ⚠️ **The worker still owns the file.** This carries no path and never writes: one writer per run
/// directory is the rule `run::transcript` and `llm::history` both keep, and a policy that persisted
/// from the emulator thread would race the worker's own `persist`.
#[derive(Debug, Default)]
pub struct Live {
    inner: std::sync::Mutex<LiveInner>,
}

#[derive(Debug, Default)]
struct LiveInner {
    source: Option<String>,
    state: ScriptState,
    failure: Option<String>,
}

impl Live {
    /// Point the policy at a script, or at none. Called by the worker after a successful
    /// `set_battle_script` and on a restart.
    pub fn arm(&self, source: Option<String>, state: ScriptState) {
        let mut inner = self.locked();
        inner.source = source;
        inner.state = state;
        inner.failure = None;
    }

    /// What the policy should run this turn, if anything.
    pub fn source(&self) -> Option<String> {
        self.locked().source.clone()
    }

    /// What the battle turn should say about it. Survives [`Self::take_failure`], which the worker
    /// calls to persist the reason — the *reason* is spent by being reported once, and the fact that
    /// there is a broken script to go and fix is not.
    pub fn state(&self) -> ScriptState {
        self.locked().state
    }

    /// The policy found the script wanting. It stops deciding turns immediately — the source is
    /// dropped here rather than flagged, so nothing can consult it again before the worker has
    /// caught up — and `why` is left for the worker to persist.
    pub fn failed(&self, why: &str) {
        let mut inner = self.locked();
        inner.source = None;
        inner.state = ScriptState::Disarmed;
        inner.failure = Some(why.to_string());
    }

    /// Taken by the worker at the top of a turn, once.
    pub fn take_failure(&self) -> Option<String> {
        self.locked().failure.take()
    }

    /// A poisoned lock is recovered rather than propagated: everything held across it is a clone of
    /// a `String`, so there is no half-updated state to protect, and panicking here would stop the
    /// run over a script.
    fn locked(&self) -> std::sync::MutexGuard<'_, LiveInner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

// ---------------------------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------------------------

/// The scenarios every script is put through before it is armed.
///
/// ⚠️ **They are the point of `set_battle_script`, not a formality.** A script is written blind, by
/// a model that cannot run it, against an API it has read once. Compiling is not evidence that it
/// terminates, chooses anything, or chooses something legal — and the alternative to finding that
/// out here is finding it out mid-battle, at the cost of a disarm and a turn.
///
/// ⚠️ **The table that comes back is as valuable as the pass/fail.** It shows the model its own
/// policy's behaviour on six turns it never has to describe, which is the only chance it gets to
/// notice that "run below 10%" reads `hp` where it meant `hp_frac` before a real battle does.
///
/// ⚠️ **Every scenario is turn 1, and validation is therefore not a proof.** A script whose
/// behaviour depends on `battle.turn`, on a party member that only exists later in the run, or on a
/// bag it expects to have something in cannot be checked here at all — there is no game to check it
/// against, which is the whole reason these run on the worker thread in the first place. That is
/// what the one-strike disarm is for, and why it is a separate mechanism rather than a fallback
/// nobody expects to need. `a_script_that_fails_disarms_and_hands_the_turn_back` is built on
/// exactly this gap.
const SCENARIOS: &[(&str, fn() -> GameState)] = &[
    ("full-hp wild", scenarios::healthy_wild),
    ("low-hp wild", scenarios::hurt_wild),
    ("low-hp trainer, bench healthy", scenarios::hurt_trainer),
    ("last one standing", scenarios::last_mon),
    ("no damaging pp left", scenarios::out_of_pp),
    ("weakened wild, balls in the bag", scenarios::catchable_wild),
];

/// Compile and run a script through every scenario. `Ok` is the table the model is shown.
fn validate(source: &str) -> Result<String, String> {
    let mut table = String::new();
    for (name, scenario) in SCENARIOS {
        let state = scenario();
        let evaluation = run(source, &state, 1);
        let what = match &evaluation.outcome {
            // ⚠️ The report's verb phrase, not `BattleAction`'s `Display` — see
            // `battle_report::intent`, which says both why and what the alternative put here.
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

/// One of [`SCENARIOS`]' states, for the tests in [`crate::llm::battle_report`] and for
/// `prompt::probe_turn_requests` — a healthy lead in a wild battle. Here rather than duplicated
/// there, so the modules that describe a battle cannot drift about what one looks like.
#[cfg(any(test, feature = "diagnostics"))]
pub fn test_scenario() -> GameState {
    scenarios::healthy_wild()
}

/// The six turns [`SCENARIOS`] puts a script through, hand-built so validation needs no emulator
/// and can run on the worker thread while a turn is in flight.
pub(crate) mod scenarios {
    use crate::pokemon::GameState;
    use crate::pokemon::bag::{Bag, BagItem};
    use crate::pokemon::battle::{BattleState, BattleType};
    use crate::pokemon::item::ItemId;
    use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
    use crate::pokemon::pokemon::Pokemon;
    use crate::pokemon::species::PokemonSpecies;

    /// ⚠️ **The level is set through the experience and a `recalculate`, never by writing the
    /// field.** `Pokemon::maxed` builds a Lv100, and assigning `level` alone leaves a Lv12 Squirtle
    /// with 292 HP — which validates a script's `hp_frac` rules perfectly well and validates
    /// anything written against `battle.me.hp` against a number no real party member ever has.
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

    /// Every move out of PP. `battle_options` offers no `Fight` row at all here, which is the case
    /// a script written around `best_move` will walk into and has to survive.
    pub fn out_of_pp() -> GameState {
        let mut lead = starter(14);
        lead.moves = lead.moves.map(|slot| slot.map(|battle_move| PokemonMove { pp: 0, ..battle_move }));
        state(vec![lead, bench(12)], vec![BagItem::new(ItemId::Potion, 1)], BattleType::Wild, 0, rattata(9))
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
    use crate::run::tests::Scratch;

    fn decide(source: &str, state: &GameState) -> Outcome {
        run(source, state, 1).outcome
    }

    fn wild() -> GameState {
        scenarios::healthy_wild()
    }

    /// The shortest script that reaches an action on **every** scenario, which is what arming
    /// requires. `battle.run()` alone does not: two of the six are trainer battles.
    const ALWAYS_DECIDES: &str = "if battle.can_run { battle.run(); }\nbattle.ask();";

    /// ⚠️ `switch` is a statement keyword in rhai and reserved even in method position, which is
    /// why the action is `switch_to`. That is a fact about rhai's grammar rather than a choice, so
    /// it is pinned: if a future rhai lets `switch` through, this still passes and the name can be
    /// reconsidered deliberately rather than by accident.
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

    /// The whole feature in one assertion: a script chooses a real battle action, and the action is
    /// one the game would have offered.
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

    /// ⚠️ The example in `DOCS.md` is the one piece of this the model copies verbatim, so it is
    /// checked rather than proof-read. It runs against every scenario, since a worked example that
    /// only works on a healthy lead is the example that teaches the bug.
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

    /// The rule the docs state, and the one Alex's original sketch depends on: `run()` followed by
    /// more code must not fall through into it.
    #[test]
    fn an_action_ends_the_script() {
        // If `run` did not terminate, `fight` below would overwrite the choice.
        let outcome = decide("battle.run(); battle.fight(battle.best_move);", &wild());
        assert_eq!(outcome, Outcome::Action(BattleAction::Run));
    }

    /// ⚠️ Rhai has `try`/`catch`, so the abort *can* be swallowed. The choice cell is what makes the
    /// rule hold anyway: the first action wins however the script is written.
    #[test]
    fn the_first_action_wins_even_when_the_abort_is_caught() {
        let outcome = decide(
            "try { battle.run(); } catch(e) { } battle.fight(battle.best_move);",
            &wild(),
        );
        assert_eq!(outcome, Outcome::Action(BattleAction::Run), "the caught abort still committed");
    }

    /// ⚠️ **Two of this API's names were parse errors before they were anything else** —
    /// `mv.type` and `battle.switch(...)`, both reserved words in rhai, both found by a test rather
    /// than by reading the grammar. A name a model cannot type is worse than a missing feature: the
    /// script does not misbehave, it does not compile, and the model spends a round trip finding
    /// out. So every field and every action is *parsed* here, mechanically, rather than trusted.
    #[test]
    fn every_name_the_docs_use_is_one_the_parser_accepts() {
        let fields = ["kind", "turn", "can_run", "trapped", "catch_rate", "me", "foe", "party", "moves", "best_move", "bag"];
        for field in fields {
            let outcome = decide(&format!("let x = battle.{field}; battle.ask();"), &wild());
            assert_eq!(outcome, Outcome::Ask, "`battle.{field}` does not parse or does not exist");
            assert!(DOCS.contains(&format!("battle.{field}")), "`battle.{field}` exists and is undocumented");
        }
        // ⚠️ **Documented *in its own section*, not merely mentioned somewhere.** The fields are
        // tabulated per object now, and `name` appears on all three — so a bare `contains` would
        // pass for a Move field that only the Pokemon table lists, which is precisely the confusion
        // this rewrite was for.
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

    /// A hang is the failure `catch_unwind` cannot catch, so the fuel limit is the guard that
    /// matters most. `loop` has no exit here at all.
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

    /// The other half of the same guard: recursion, which the operation count reaches more slowly
    /// than the call-depth limit does.
    #[test]
    fn unbounded_recursion_is_stopped() {
        let outcome = decide("fn down(n) { down(n + 1) } down(0); battle.run();", &wild());
        assert!(matches!(outcome, Outcome::Failed(_)), "got {outcome:?}");
    }

    /// A script that decides nothing is a failure with a sentence, never a silently skipped turn.
    #[test]
    fn choosing_nothing_is_a_failure_that_says_so() {
        let Outcome::Failed(why) = decide("let x = 1 + 1;", &wild()) else { panic!("expected a failure") };
        assert!(why.contains("without calling an action"), "{why}");
        assert!(why.contains("battle.fight"), "the reason names the way out: {why}");
    }

    /// ⚠️ The script filters `battle_options`; it never invents. A move the Pokémon does not know is
    /// refused, and the refusal says what it *does* know.
    #[test]
    fn an_action_the_game_would_refuse_is_named_rather_than_taken() {
        let Outcome::Failed(why) = decide(r#"battle.fight("Hydro Pump");"#, &wild()) else {
            panic!("a move it does not know must not be accepted")
        };
        assert!(why.contains("Hydro Pump"), "{why}");
        assert!(why.contains("Ember"), "the reason lists what is actually usable: {why}");
    }

    /// Running from a trainer is the case a model will get wrong, and the message has to point at
    /// `can_run` rather than at the tool.
    #[test]
    fn running_from_a_trainer_is_refused_with_the_reason() {
        let Outcome::Failed(why) = decide("battle.run();", &scenarios::hurt_trainer()) else {
            panic!("there is no running from a trainer battle")
        };
        assert!(why.contains("can_run"), "{why}");
    }

    /// ⚠️ The cartridge's spelling is not the one a model types. All three of these are the same
    /// item, and the run most needs the one it has never thrown.
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

    /// `battle.ask()` keeps the script armed and hands one turn back. Without it scripting is
    /// all-or-nothing and a model will choose nothing.
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

    /// ⚠️ The facts the script reads are the ones the turn was built from, and the two must agree.
    /// `hp_frac` is the field every script will branch on, so it is the one pinned.
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

    /// A bench member's moves are scored against the foe that is out, which is the whole basis of a
    /// coverage switch. Squirtle's Water Gun beats Charmander's Scratch against nothing in
    /// particular here — what is asserted is that the numbers are *there* and differ.
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

    /// ⚠️ `best_move` is `()` when nothing can hurt the foe, and a script written around it has to
    /// survive that. The `out_of_pp` scenario exists for this and `SCENARIOS` runs it on every set.
    #[test]
    fn no_usable_damaging_move_leaves_best_move_unset() {
        let outcome = decide(
            "if battle.best_move == () { battle.run(); } battle.fight(battle.best_move);",
            &scenarios::out_of_pp(),
        );
        assert_eq!(outcome, Outcome::Action(BattleAction::Run));
    }

    /// The sandbox has no way out. None of these are registered, and a model reaching for one gets
    /// a compile error rather than a file.
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

    // ---------------------------------------------------------------------------------------
    // Validation and persistence
    // ---------------------------------------------------------------------------------------

    /// The table is the feedback loop: it shows the model what its own rules do on six turns it
    /// never has to describe.
    #[test]
    fn validation_arms_a_good_script_and_shows_what_it_chose() {
        let mut script = BattleScript::open(None);
        let answer = script.set(Some(
            "if battle.me.hp_frac < 0.1 && battle.can_run { battle.run(); }\n\
             if battle.best_move == () { battle.ask(); }\n\
             battle.fight(battle.best_move);",
        ));
        assert!(script.armed(), "a script that passes every scenario is armed: {answer}");
        for (name, _) in SCENARIOS {
            assert!(answer.contains(name), "the table is missing `{name}`:\n{answer}");
        }
        // The table's rows are the report's verb phrases, not `BattleAction`'s menu rows: see
        // `battle_report::intent`, and `probe_battle_script_answers` for what it reads like.
        assert!(answer.contains("tried to run"), "the low-hp wild scenario should have fled:\n{answer}");
    }

    /// ⚠️ Compiling is not evidence. This script is syntactically perfect and dies on the one
    /// scenario where nothing can attack — which is exactly the failure a live battle would find.
    #[test]
    fn a_script_that_only_works_sometimes_is_not_armed() {
        let mut script = BattleScript::open(None);
        let answer = script.set(Some("battle.fight(battle.best_move);"));
        assert!(!script.armed(), "it must not arm: {answer}");
        assert!(answer.contains("no damaging pp left"), "the answer names the scenario:\n{answer}");
        assert!(script.is_default(), "a refused script must not be stored: the default is left where it was");
    }

    #[test]
    fn a_script_that_does_not_compile_says_why() {
        let mut script = BattleScript::open(None);
        let answer = script.set(Some("if battle.me.hp_frac < { battle.run("));
        assert!(!script.armed());
        assert!(answer.starts_with("Not armed"), "{answer}");
        assert!(answer.len() > 40, "the parse error has to reach the model: {answer}");
    }

    /// One strike. A script that failed is kept so it can be edited, and stops deciding turns.
    #[test]
    fn a_failure_disarms_but_keeps_the_script() {
        let mut script = BattleScript::open(None);
        let answer = script.set(Some(ALWAYS_DECIDES));
        assert!(script.armed(), "{answer}");

        script.disarm("it chose a move BULBASAUR does not know");
        assert!(!script.armed(), "one failure is enough");
        assert_eq!(script.source(), Some(ALWAYS_DECIDES), "the source is what the model has to edit");
        assert!(script.read().contains("Disarmed"), "read_battle_script says so: {}", script.read());
        assert!(script.read().contains("does not know"), "and why: {}", script.read());
    }

    /// The run directory round trip, and the one thing that makes it worth persisting at all: a
    /// process that restarts fights the next battle the way the model told it to.
    #[test]
    fn a_script_survives_the_process_that_wrote_it() {
        let scratch = Scratch::new("battle-script");
        let source = "if battle.can_run && battle.me.hp_frac < 0.2 { battle.run(); }\n\
                      if battle.best_move == () { battle.ask(); }\n\
                      battle.fight(battle.best_move);";

        let mut written = BattleScript::open(Some(&scratch.0));
        written.set(Some(source));
        assert!(written.armed());

        let reopened = BattleScript::open(Some(&scratch.0));
        assert_eq!(reopened.source(), Some(source), "byte for byte");
        assert!(reopened.armed(), "and still deciding turns");

        // A disarm is durable too, or a restart re-arms a script that is known to be broken.
        let mut written = BattleScript::open(Some(&scratch.0));
        written.disarm("it ran out of operations");
        assert!(!BattleScript::open(Some(&scratch.0)).armed());
    }

    /// ⚠️ **Unsetting goes back to [`DEFAULT`] rather than to nothing**, which is the whole of what
    /// "a run always has a script" means at the tool boundary. The behaviour is identical either way
    /// — the default hands every turn back — so nothing is lost, and what is gained is that
    /// `read_battle_script` can never again answer a round trip with "there is no battle script".
    #[test]
    fn unsetting_goes_back_to_the_default() {
        let scratch = Scratch::new("battle-script");
        let mut script = BattleScript::open(Some(&scratch.0));
        script.set(Some(ALWAYS_DECIDES));
        let answer = script.set(None);
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
        script.set(Some(ALWAYS_DECIDES));
        let answer = script.set(Some(&"// padding\n".repeat(MAX_SOURCE)));
        assert!(answer.contains(&MAX_SOURCE.to_string()), "{answer}");
        assert_eq!(script.source(), Some(ALWAYS_DECIDES), "the armed script is untouched");
        assert!(script.armed());
    }

    /// An unreadable file is the default script, not a refusal to play — `TodoList::open`'s rule.
    #[test]
    fn a_corrupt_file_starts_on_the_default_rather_than_failing() {
        let scratch = Scratch::new("battle-script");
        std::fs::write(scratch.0.join(files::BATTLE_SCRIPT), b"{ not json").unwrap();
        let script = BattleScript::open(Some(&scratch.0));
        assert!(!script.armed());
        assert!(script.is_default());
    }

    /// ⚠️ **A run written before there was a default is carried across it.** `{"source": null}` is
    /// what every run's file said until this change, and the per-field `#[serde(default)]` reads it
    /// back as `None` — which every reader below `open` now assumes cannot happen.
    #[test]
    fn a_run_from_before_the_default_is_brought_onto_it() {
        let scratch = Scratch::new("battle-script");
        let path = scratch.0.join(files::BATTLE_SCRIPT);
        std::fs::write(&path, br#"{"source":null,"armed":false,"last_failure":null}"#).unwrap();
        let script = BattleScript::open(Some(&scratch.0));
        assert!(script.is_default(), "an empty file is the default now");
        assert_eq!(script.state(), ScriptState::Unedited);

        // ⚠️ A *disarmed* script is left exactly as it was: it has its own source, and replacing it
        // with the default would throw away the thing the model has to edit and the reason it broke.
        std::fs::write(&path, br#"{"source":"battle.run();","armed":false,"last_failure":"it fled"}"#).unwrap();
        let broken = BattleScript::open(Some(&scratch.0));
        assert_eq!(broken.source(), Some("battle.run();"));
        assert_eq!(broken.state(), ScriptState::Disarmed);
        assert_eq!(broken.last_failure(), Some("it fled"));
    }

    /// ⚠️ **The default has to compile and has to ask** — it ships as a `const` and is never run in
    /// anger (`live_source` withholds it), so nothing else would ever find out that it did not.
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

    /// ⚠️ **The default never reaches the emulator thread, and that is what keeps this change free.**
    /// A default that *was* run would evaluate an engine on every battle turn of every run and, far
    /// worse, set `LlmPolicy::note` each time — which suppresses the `TurnContext::Battle` line that
    /// is the entire point of having a default at all.
    #[test]
    fn the_default_is_never_handed_to_the_policy() {
        let script = BattleScript::open(None);
        assert!(script.is_default(), "a fresh run starts on it");
        assert!(script.live_source().is_none(), "but the policy is handed nothing");
        assert!(!script.armed(), "and nothing tells the model or the page that battles are free");
        assert_eq!(script.state(), ScriptState::Unedited);

        // Whereas a script the model actually wrote is handed over, which is the contrast.
        let mut script = BattleScript::open(None);
        script.set(Some(ALWAYS_DECIDES));
        assert!(script.armed());
        assert_eq!(script.live_source().as_deref(), Some(ALWAYS_DECIDES));
        assert_eq!(script.state(), ScriptState::Armed);
    }

    /// ⚠️ **`read_battle_script` can no longer spend a round trip saying "there is nothing"**, which
    /// is the reason the default exists at all: the model is asked to edit a file, not to invent one.
    #[test]
    fn reading_a_fresh_runs_script_answers_with_the_default_source() {
        let answer = BattleScript::open(None).read();
        assert!(answer.contains("default script"), "{answer}");
        assert!(answer.contains("battle.ask()"), "the source itself, to edit: {answer}");
        assert!(answer.contains("set_battle_script"), "and what to do about it: {answer}");
    }

    /// What `set_battle_script` actually answers with, printed rather than asserted.
    ///
    /// ⚠️ **The table is prose a model reads, and nothing but reading it catches prose.** It is the
    /// only place the six scenarios' names, the column widths and the rendering of a `BattleAction`
    /// meet, and every one of those reads perfectly well while saying the wrong thing. Same reason
    /// `prompt::probe_turn_requests` exists, and `#[ignore]`d on top of its feature gate for the
    /// same reason: it asserts nothing.
    #[cfg(feature = "diagnostics")]
    #[test]
    #[ignore]
    fn probe_battle_script_answers() {
        let example = DOCS.rsplit("```rhai").next().and_then(|t| t.split("```").next()).unwrap();
        let mut script = BattleScript::open(None);
        // First, because it is the answer every run gets before it has written anything and the one
        // that used to be a wasted round trip saying "there is no battle script".
        println!("── read_battle_script, on a fresh run ──\n{}\n", script.read());
        println!("── set_battle_script, with the documented example ──\n{}\n", script.set(Some(example)));
        println!("── read_battle_script ──\n{}\n", script.read());
        script.disarm("`battle.fight` was given `Hydro Cannon`, which is not a move that can be used this turn.");
        println!("── read_battle_script, after a failure ──\n{}\n", script.read());
        println!("── set_battle_script, with one that only works sometimes ──\n{}\n",
                 script.set(Some("battle.fight(battle.best_move);")));
    }

    /// ⚠️ **The bundled strategy is checked the way the docs' example is, and for a stronger
    /// reason**: it is the deterministic policy's own logic, so a change here that silently stops it
    /// arming has broken the one script known to finish this game.
    #[test]
    fn the_deterministic_strategy_still_arms_and_still_plays() {
        let mut script = BattleScript::open(None);
        let answer = script.set(Some(DETERMINISTIC));
        assert!(script.armed(), "the bundled strategy no longer arms:\n{answer}");

        // It mirrors the policy's order, so the scenarios it was built from pin its arms.
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

        // ⚠️ It is longer than the docs' example and must still fit, or the one strategy that is
        // known to work is the one the size cap refuses.
        assert!(DETERMINISTIC.len() < MAX_SOURCE, "it is {} bytes against {MAX_SOURCE}", DETERMINISTIC.len());
    }


    /// What the language actually does, printed rather than asserted, so `DOCS.md` can be written
    /// from evidence instead of from memory.
    #[cfg(feature = "diagnostics")]
    #[test]
    #[ignore]
    fn probe_language_features() {
        let state = wild();
        let snippets: &[(&str, &str)] = &[
            ("fn sees battle", "fn f() { battle.turn } let x = f(); battle.ask();"),
            ("fn with param", "fn f(b) { b.turn } let x = f(battle); battle.ask();"),
            ("array .len", "let n = battle.party.len; battle.ask();"),
            ("array .len()", "let n = battle.party.len(); battle.ask();"),
            ("filter closure", "let a = battle.party.filter(|p| !p.fainted); battle.ask();"),
            ("map closure", "let a = battle.moves.map(|m| m.damage); battle.ask();"),
            ("reduce", "let a = battle.moves.reduce(|s, m| s + m.damage, 0); battle.ask();"),
            ("while loop", "let i = 0; while i < 3 { i += 1; } battle.ask();"),
            ("loop+break", "let i = 0; loop { i += 1; if i > 2 { break; } } battle.ask();"),
            ("string concat", r#"let s = "a" + battle.turn; battle.ask();"#),
            ("string methods", r#"let s = "AB".to_lower(); let c = s.contains("a"); battle.ask();"#),
            ("map literal", "let m = #{ a: 1 }; let v = m.a; battle.ask();"),
            ("unit compare", "if battle.best_move != () { battle.ask(); } battle.ask();"),
            ("float math", "let f = battle.me.hp_frac * 100.0; battle.ask();"),
            ("int division", "let d = 7 / 2; battle.ask();"),
            ("switch stmt", "let x = switch battle.turn { 1 => 10, _ => 20 }; battle.ask();"),
            ("in operator", r#"let b = "a" in "abc"; battle.ask();"#),
            ("array push", "let a = []; a.push(1); battle.ask();"),
            ("sort_by", "let a = battle.moves; a.sort(|x, y| y.damage - x.damage); battle.ask();"),
            ("index chain", "let d = battle.party[0].moves[0].damage; battle.ask();"),
            ("early return", "fn f(n) { if n > 0 { return 1; } 0 } let x = f(1); battle.ask();"),
        ];
        for (name, code) in snippets {
            let outcome = run(code, &state, 1).outcome;
            let verdict = match &outcome {
                Outcome::Failed(why) => format!("NO  — {}", why.lines().next().unwrap_or("")),
                _ => "yes".to_string(),
            };
            println!("  {name:<18} {verdict}");
        }
    }

    /// The docs are carried in the context once the model reads them, so they are bounded the way a
    /// guide chapter is.
    #[test]
    fn the_docs_stay_within_what_they_cost_to_carry() {
        // ⚠️ **9.5 KB, and it was 6.** The first version was terse enough to be wrong: it described
        // the language in one sentence ("close to Rust and JavaScript"), never documented `bag` at
        // all, and left it unclear which fields belonged to which object. A model that has to guess
        // pays a round trip per guess, and this is fetched **once** and then carried — so the
        // trade is ~800 tokens against the requests a working script removes, which is the same
        // arithmetic that bought the tools their place in the catalogue.
        assert!(DOCS.len() < 9_500, "the docs are {} bytes", DOCS.len());
        for name in ["battle.fight", "battle.switch", "battle.use_item", "battle.run", "battle.ask"] {
            assert!(DOCS.contains(name), "the docs never mention {name}");
        }
        assert!(DOCS.contains("hp_frac"), "the field every script branches on is undocumented");
        assert!(DOCS.contains("effectiveness"), "the reason no type chart is needed is undocumented");
    }

    /// ⚠️ **The docs make claims about the *language*, and a wrong one costs a round trip.** These
    /// are the constructs they tell the model it may use, each run through the real engine — the
    /// list was built by running them rather than from memory, and `fn` scope was the one that came
    /// back different from what the first draft of the docs said.
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

        // ⚠️ **And the one the docs warn about, which has to keep failing.** A `fn` body cannot see
        // `battle`. If a future rhai changes that, this test is where the warning gets deleted from
        // the docs deliberately rather than quietly becoming a lie in the other direction.
        let Outcome::Failed(why) = decide("fn f() { battle.turn } let x = f(); battle.ask();", &state)
        else { panic!("a `fn` must not see `battle`; the docs devote a numbered point to it") };
        assert!(why.contains("battle"), "and the reason has to name it: {why}");
    }

    /// ⚠️ **Every field the docs tabulate has to exist, and every field that exists has to be
    /// tabulated.** The first version of the docs never mentioned `bag` at all, so the run that most
    /// needed to start throwing Poké Balls had no documented way to find one.
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
        // The bag is documented as a table of Item, and the item names in it have to be the ones
        // the game actually uses — `use_item` normalises, but a script comparing `item.name` does
        // not, and the docs tell it to do exactly that.
        for name in ["Potion", "SuperPotion", "PokeBall"] {
            assert!(DOCS.contains(name), "the docs name no `{name}` for a script to compare against");
        }
        assert!(DOCS.contains("battle.bag"), "the bag is undocumented");
    }
}
