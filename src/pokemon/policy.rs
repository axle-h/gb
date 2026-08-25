use std::collections::{VecDeque, HashSet};
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver};
use rand::prelude::StdRng;
use rand::seq::IteratorRandom;
use rand::SeedableRng;
use crate::pokemon::{GameState, PokemonApi};
use crate::pokemon::actions::OverworldAction;
use crate::geometry::Point8;
use crate::pokemon::badge::Badge;
use crate::pokemon::bag::BagItem;
use crate::pokemon::battle::{BattleAction, BattleType};
use crate::pokemon::damage::{expected_damage, is_damaging_move, pick_best_move};
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::data::PokemonNamePicker;
use crate::pokemon::tile::MetaTile;
pub use crate::pokemon::item::ItemId;
use crate::pokemon::map::{Map, MapSprite};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::world_graph::WorldGraph;

/// Non-blocking policy interface.
///
/// All methods return `Option<_>`. `None` means "not ready yet — ask again next frame".
/// This keeps the game loop running while the policy waits for input.
pub trait Policy {
    /// What this decider is called: `"llm"`, `"random"`, `"console"` or `"scripted"`.
    ///
    /// ⚠️ **Required rather than defaulted**, which is unusual on this trait. It reaches two places
    /// that outlive the process — `StatusSnapshot::policy`, which the page renders, and the row
    /// `run::hall_of_fame` writes for a finished run — and a default would be a plausible-looking
    /// lie in the one place it matters. There are four implementations and the compiler finds them
    /// all.
    ///
    /// ⚠️ **`"llm"` and `"random"` are a wire contract**, not free text: the SPA reads
    /// `StatusSnapshot.policy` and the deployment's status line is built from it.
    ///
    /// The *model* is deliberately not part of this. `run::RunMeta::model` already holds `GB_MODEL`,
    /// and `LlmPolicy` could not answer anyway — `LlmConfig` is moved into the worker thread when
    /// the policy is built.
    fn name(&self) -> &'static str;

    /// Choose the next overworld action.
    ///
    /// `world_graph` is the agent's **incrementally-built** map graph — it only contains
    /// sections the player has already physically visited (accurate, sprite-resolved). Use it
    /// for backtracking to known places (e.g. heal-return to a Pokémon Center); forward travel
    /// into not-yet-visited maps must be scripted with explicit [`PolicyStep::EnterMap`] steps.
    fn pick_overworld_action(&mut self, state: &GameState, world_graph: &WorldGraph) -> Option<OverworldAction>;
    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction>;

    /// What the player is called, asked once when a **new** game starts.
    ///
    /// `None` — the default — keeps whatever `data::START_OF_GAME` was captured with, which is the
    /// right answer for anything replaying a scripted route: a fixture chain that renamed the
    /// trainer would differ from every state it was captured against.
    ///
    /// ⚠️ **Not a poll, unlike every other `pick_*` on this trait.** The other methods return
    /// `Option` to mean "ask me again next frame", because they are answered by a model that may be
    /// mid-completion. This one is answered before the emulator has run a single instruction, so
    /// there is nobody to wait for and `None` gets to mean what it reads as. Anything that needed a
    /// round trip would have to name the player *after* the game had started, which the game's own
    /// screens can no longer do.
    ///
    /// Trimmed to [`MAX_PLAYER_NAME`](crate::pokemon::MAX_PLAYER_NAME) by
    /// `PokemonApiTrait::write_player_name`, which is the game's own limit for this field.
    fn player_name(&self) -> Option<String> {
        None
    }

    /// Called when the nickname-entry screen opens for `species`.
    ///
    /// - `None`          → not ready yet; will be called again next frame.
    /// - `Some(None)`    → decline a nickname; the game keeps the default species name.
    /// - `Some(Some(s))` → give this nickname (up to 10 characters, A-Z / a-z / 0-9 / common punctuation).
    fn pick_nickname(&mut self, _species: PokemonSpecies) -> Option<Option<String>> {
        Some(None) // default: keep the default species name
    }

    /// Called when the mart's Buy/Sell/Quit menu first appears.
    ///
    /// - `None`       → not ready yet; will be called again next frame.
    /// - `Some(None)` → do not buy anything.
    /// - `Some(Some(item))` → buy the item.
    fn pick_mart_purchase(&mut self, _state: &GameState) -> Option<Option<BagItem>> {
        Some(None) // default: open the mart but buy nothing
    }

    /// Called on the level-up "Which move should be forgotten?" prompt, when a Pokémon that already
    /// knows 4 moves would learn `new_move`. `current_moves` are the 4 known moves (slot order).
    ///
    /// - `None`             → not ready yet; asked again next frame.
    /// - `Some(None)`       → decline learning; keep the current four moves.
    /// - `Some(Some(slot))` → forget the move in `slot` (0-3) and learn `new_move`.
    fn pick_move_to_forget(&mut self, _current_moves: &[PokemonMove], _new_move: PokemonMoveName)
        -> Option<Option<usize>>
    {
        Some(None) // default: never drop an existing move
    }

    /// Called each idle overworld tick. Returns a non-walking field action to perform (e.g. teach an
    /// HM), or `None` to fall through to [`pick_overworld_action`].
    fn pick_field_move(&mut self, _state: &GameState) -> Option<FieldMove> {
        None
    }

    /// Called for **every** event the agent emits, as it is emitted and before it is buffered.
    ///
    /// This is the only way a policy learns anything between decisions. The `pick_*` methods see a
    /// [`GameState`], which is a snapshot of RAM and says nothing about what just happened; the
    /// events carry the narrative — the text of a conversation
    /// ([`AgentEvent::TextBox`](crate::pokemon::agent::AgentEvent::TextBox)), a battle starting and
    /// ending, and above all
    /// [`OverworldActionAborted`](crate::pokemon::agent::AgentEvent::OverworldActionAborted), which
    /// is the feedback a decider needs to stop re-picking a route that cannot be walked.
    ///
    /// The default ignores them, so no existing policy is affected: `RandomPolicy` and
    /// `DeterministicPolicy` do not override it.
    ///
    /// ⚠️ Called from inside the agent's tick, so it must not block — the same non-blocking contract
    /// the `pick_*` methods are under, without the `Option` to make it obvious.
    fn on_event(&mut self, _event: &crate::pokemon::agent::AgentEvent) {}

    /// Called at the top of every policy poll, before any `pick_*` for that decision point.
    ///
    /// This is where a policy that answers questions — an LLM working through a batch of read-only
    /// tool calls before it commits to a move — gets to do so, without a round trip through the
    /// agent and without the emulator advancing underneath it. The observation facade in
    /// [`crate::pokemon::observe`] is written against exactly this triple, so servicing a call is a
    /// direct function call rather than a message.
    ///
    /// ⚠️ **The state is observed once per poll and every queued call is answered from it**, so one
    /// turn never sees a torn view: `read_party` and `read_map` in the same assistant message are
    /// guaranteed to agree with each other.
    ///
    /// Default: a no-op. No existing policy is affected.
    fn service_tools(&mut self, _state: &GameState, _api: &mut PokemonApi<'_>, _graph: &WorldGraph) {}

    /// **W9 / §14** — how much **emulated** time may pass with the agent asking this policy nothing
    /// at all before it wants waking anyway. `None` — the default, and what every scripted policy
    /// returns — means never, and compiles the watchdog down to one comparison per tick.
    ///
    /// This is not "how long a decision may take". The `pick_*` methods are polled fifty times a
    /// second while a policy thinks, and every one of those polls resets the clock. What it measures
    /// is the agent reaching **no decision point of any kind** — a jam in `RunningScript`, or an
    /// `OverworldMovement` walking into a sprite forever — which is the one failure mode nothing
    /// else covers, because the policy is not consulted in those states and so cannot notice.
    ///
    /// ⚠️ **Read once, when the agent is built.** A policy that changed its mind later would not be
    /// heard.
    fn stuck_timeout(&self) -> Option<std::time::Duration> {
        None
    }

    /// **W9 / §14** — the agent has gone [`Self::stuck_timeout`] without asking anything, and is
    /// asking now.
    ///
    /// Called on **every** tick for as long as the jam lasts, immediately after
    /// [`Self::service_tools`] — which is the whole point: a tool batch is only ever answered at a
    /// policy poll, so a decision that needs one could not be made at all if this were a one-shot
    /// notification.
    ///
    /// It returns nothing because there is nothing a jammed agent could carry out. The answer
    /// travels the escape hatch instead: [`Self::take_manual_input`] is collected at the top of the
    /// next tick and pre-empts the state machine, which is also what clears the jam — a queued press
    /// resets the agent to `Idle` (see
    /// [`queue_manual_input`](crate::pokemon::agent::PokemonAgent::queue_manual_input)).
    ///
    /// Default: a no-op, and unreachable for any policy that did not ask for it above.
    fn pick_unstick(&mut self, _state: &GameState, _jam: Jam<'_>) {}

    /// The game has been restarted underneath this policy — throw away anything held about the old
    /// one. `run_dir` is the **new** run directory, for a policy that keeps files in it.
    ///
    /// Called from [`PokemonAgent::restart`](crate::pokemon::agent::PokemonAgent::restart), which
    /// `POST /api/new-run` reaches on the emulator thread. Nothing else calls it: a policy that is
    /// never reset simply never sees it.
    ///
    /// Default: a no-op. Correct for every scripted policy — `DeterministicPolicy`'s queue is the
    /// script it was constructed with rather than anything it learned, and `RandomPolicy` holds
    /// nothing at all.
    fn restart(&mut self, _run_dir: Option<&std::path::Path>) {}

    /// **W5** — raw button presses the policy wants delivered, collected by the agent at the top of
    /// its next tick and handed to
    /// [`queue_manual_input`](crate::pokemon::agent::PokemonAgent::queue_manual_input).
    ///
    /// This is the collection half of W0.4's escape hatch. The queue lives on the agent, but the
    /// agent owns the policy rather than the other way round, so the presses have to be *pulled*:
    /// there is no moment at which a `pick_*` could push them, and returning them from one would mean
    /// a return type that says "a walk, or some buttons" at every site.
    ///
    /// The default returns an empty `Vec`, which does not allocate — this runs once per agent tick
    /// for every policy, including the scripted ones that will never use it.
    fn take_manual_input(&mut self) -> Vec<crate::joypad::JoypadButton> {
        Vec::new()
    }

    fn is_exhausted(&self) -> bool {
        false
    }

    /// Returns the number of steps remaining in the policy queue, if known.
    fn steps_remaining(&self) -> Option<usize> {
        None
    }

    /// Returns true if the current step is expected to run for a long time without
    /// advancing the queue (e.g. grinding levels or catching a Pokémon). Used by the
    /// test fixture to exempt these steps from the short stall-detection threshold.
    fn current_step_is_long_running(&self) -> bool {
        false
    }
}

/// **W9 / §14** — what the watchdog knows about the jam it is waking the policy for.
///
/// Both fields exist to be *reported*: §14's rule is that every firing is a bug report, so the
/// state the agent believes it is in and how long it has believed it go to the model, to the UI and
/// to stdout rather than being quietly recovered from.
#[derive(Debug, Clone, Copy)]
pub struct Jam<'a> {
    /// What the agent thinks it is doing — [`PokemonAgent::state_debug`](crate::pokemon::agent::PokemonAgent::state_debug).
    pub agent_state: &'a str,
    /// Emulated time since the agent last asked the policy anything.
    pub stuck_for: std::time::Duration,
}

// ── Random (always-ready) ─────────────────────────────────────────────────────

/// Picks uniformly from whatever the agent offers. `gb serve --policy random` plays the deployment
/// with it, and `integration_tests::soak` uses it as a **fuzzer** for the agent's state machine.
///
/// ⚠️ **It deliberately has no `stuck_timeout`**, so W9's watchdog never runs under it. That is the
/// point of the soak test: a policy that nudged itself out of a jam would hide the jam.
#[derive(Default)]
pub struct RandomPolicy {
    /// `None` — the default, and what `gb serve` uses — draws from the thread RNG, so no two runs
    /// are alike. `Some` pins the sequence.
    ///
    /// ⚠️ **A fuzzer that cannot repeat itself cannot verify its own fix.** Reseeding every run means
    /// a failure is gone the moment you go looking for it, and a pass proves only that *this* draw
    /// was clean. `soak` therefore seeds it and prints the seed.
    rng: Option<StdRng>,
    /// The seed, kept beside the stream it built. Only [`Policy::player_name`] reads it, and it does
    /// so rather than drawing from `rng` because that method takes `&self`: the name is chosen
    /// without disturbing the sequence the run is played from.
    seed: Option<u64>,
}

impl RandomPolicy {
    /// A policy whose choices are fixed by `seed` — the same seed always plays the same game.
    pub fn seeded(seed: u64) -> Self {
        Self { rng: Some(StdRng::seed_from_u64(seed)), seed: Some(seed) }
    }
}

/// What [`RandomPolicy`] might call itself. Seven characters or fewer, because that is the game's
/// own limit for a player name — a list rather than random letters, since the name is on the trainer
/// card, in every "…used STRENGTH!" line and at the top of the page.
const RANDOM_NAMES: &[&str] = &[
    "DICEY", "CHANCE", "FLUKE", "RANDOM", "SHUFFLE", "ROLL", "COINTOS", "HAZARD", "LOTTO", "WHIM",
    "SCATTER", "DRIFT", "ENTROPY", "JITTER", "NOISE", "STRAY",
];

impl Policy for RandomPolicy {
    fn name(&self) -> &'static str { "random" }

    /// ⚠️ **Off `self.rng` when there is one, so a seeded run stays a seeded run.** `soak` and the
    /// stall hunt both build this with an explicit seed precisely so a failure can be gone back to,
    /// and a name drawn from the thread RNG would be one more thing differing between two runs of
    /// the same seed.
    fn player_name(&self) -> Option<String> {
        // `pick_*` take `&mut self`; this does not, so the seeded stream is advanced by neither —
        // the seed picks the name directly and the sequence the run plays from is untouched.
        let index = match &self.rng {
            Some(_) => self.seed.unwrap_or(0) as usize % RANDOM_NAMES.len(),
            None => rand::random::<u64>() as usize % RANDOM_NAMES.len(),
        };
        Some(RANDOM_NAMES[index].to_string())
    }

    fn pick_overworld_action(&mut self, state: &GameState, _world_graph: &WorldGraph) -> Option<OverworldAction> {
        let actions = state.map.actions();
        match &mut self.rng {
            Some(rng) => actions.into_iter().choose(rng),
            None => actions.into_iter().choose(&mut rand::rng()),
        }
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


pub(crate) fn battle_options(state: &GameState) -> Option<Vec<BattleAction>> {
    let battle_state = state.battle.as_ref()?;

    // Safari Zone battles have their own menu (no FIGHT/PKMN/ITEM). Offer all four Safari options so a
    // future LLM-driven policy can actually hunt; the deterministic policy just RUNs (below).
    if battle_state.battle_type == BattleType::Safari {
        return Some(vec![
            BattleAction::SafariBall,
            BattleAction::SafariBait,
            BattleAction::SafariRock,
            BattleAction::Run,
        ]);
    }

    let mut opts = battle_state.player.available_battle_moves();

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

/// Returns `true` total PP remaining across all damaging moves dips below ≤20% of its maximum PP remaining.
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
        // No damaging moves, so we can't say they're all low on PP.
        return false;
    }

    (total_damaging_pp as f32 / total_max_pp as f32) < MIN_PP_PCT
}


/// How a step names a party member.
///
/// `Slot` is positional and is right whenever the position is the *point* — Cut always uses the lead,
/// so `MovePokemonToFront { slot: 1 }` means "the second one, whatever it is". `Species` is resolved
/// against the live [`GameState`] every tick, and is what to use when the step means a particular
/// **mon**: a slot index is a guess about how many members the party happened to have when the run
/// reached this step, and it silently addresses the wrong mon the moment that guess is off. That is
/// what broke `eevee_vaporeon_surf_steps` (slot 1 was a Pidgey, not the gift Eevee) and what made
/// `victory_road_1f_steps` need a `machop_slot` argument its two callers disagreed about.
///
/// Resolution is deliberately *late*: a step may name a mon the party does not hold yet (the Eevee is
/// still a Poké Ball on the floor when `eevee_vaporeon_surf_steps` is composed), so an unresolved
/// `Species` means "keep waiting", not "skip".
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PartyRef {
    /// The member at this party index.
    Slot(u8),
    /// The first member of this species.
    Species(PokemonSpecies),
}

impl PartyRef {
    /// The party index this reference currently points at, or `None` if the party holds no such mon.
    pub fn resolve(&self, state: &GameState) -> Option<u8> {
        match *self {
            Self::Slot(slot) => (usize::from(slot) < state.pokemon.len()).then_some(slot),
            Self::Species(species) => state.pokemon.iter()
                .position(|p| p.species == species)
                .map(|i| i as u8),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PolicyStep {
    Goto { map: Map, strict: bool },
    /// Take exactly one explicit map transition: walk to and use the warp/connection on the
    /// current map that leads to `to_map` (matching the raw landing `to_position` when given, to
    /// disambiguate maps with several warps to the same target — e.g. Mt Moon). This is how the
    /// deterministic policy crosses not-yet-explored mazes; it is a **hard requirement** — if the
    /// transition is not reachable on the current map the agent stalls (proving under-specification)
    /// rather than silently rerouting over an inaccurate pre-resolved graph.
    EnterMap { to_map: Map, to_position: Option<Point8> },
    /// Walk to and interact with a visible sprite by name.
    Interact(MapSprite),
    /// Like `Interact`, but if the sprite can't be reached (walled off — e.g. a Silph trainer behind
    /// the teleport-pad maze) the step gives up and pops instead of waiting forever. For best-effort
    /// gauntlet training where some trainers on a floor are unreachable from where we entered.
    InteractIfReachable(MapSprite),
    /// Walk to the map's PC tile, face it, and press A (e.g. Bill's cell-separator PC). The PC is a
    /// hidden-object tile, not a sprite; `MetaTileMap::pc_locations` supplies its coordinate. Should
    /// be scripted only when using the PC is valid (e.g. after Bill's Pokémon enters the machine).
    UsePc { map: Map },

    // ── Postgame steps (`docs/postgame-coverage-plan.md`) ────────────────────────────────────────
    //
    // Each variant's logic lives in `postgame::<stream>`; the arm here delegates in one line, which
    // is what keeps this file mergeable when several streams are open at once (§4.1 of the plan).
    // These landed as empty seams and were filled in by their owners; the shapes below are the ones
    // the real drivers wanted, not the plan's drafts.
    /// **B** — Fly to `to`. ⚠️ The town map is a bespoke screen, not a `HandleMenuInput` list.
    Fly { to: Map },
    /// **H** — collect the hidden `item` on `map`. Routes itself, like `Fish`. The tile comes from
    /// [`crate::pokemon::tile_map::MetaTileMap::hidden_items`] rather than the caller, because the
    /// ROM's coordinates need the map's connection-strip offset applied first. Driver:
    /// [`crate::pokemon::postgame::aides`]; build with [`Self::hidden_item_steps`].
    SearchHiddenItem { map: Map, item: ItemId },
    /// **H** — use **HM05 Flash** from the party menu with the mon in `slot`, lighting a dark cave.
    /// Completes when `wMapPalOffset` reaches 0, which is what "lit" means to the ROM; pops
    /// immediately on an already-lit map, so it is safe to leave in a step list.
    UseFlash { slot: u8 },
    /// **G** — run a one-NPC script that opens the **party menu**, acting on `slot`: the Route 5
    /// Day Care, or the Lavender Name Rater. An `enter(script.map())` must precede it; the driver
    /// ([`crate::pokemon::postgame::gifts`]) walks the last tiles and owns the conversation, because
    /// these menus open on a *stale* cursor and an A-mash acts on an arbitrary mon.
    PartyScript { script: crate::pokemon::postgame::gifts::PartyScript, slot: u8 },
    /// **I** — use bag `item` from the overworld on `target` (nothing, a party member, or one of its
    /// moves). One variant for the whole of `ItemUsePtrTable`'s overworld half, because the chain is
    /// always START → ITEM → the bag row → USE and only the menus *after* `USE` differ. Driver:
    /// [`crate::pokemon::postgame::items`]; build with [`Self::use_medicine`],
    /// [`Self::use_pp_restore`] or [`Self::use_item`].
    ///
    /// ⚠️ The step **pops with a reason** rather than issuing when the game would decline the use —
    /// a potion at full HP, an Ether on a full-PP move, the Bicycle indoors. Those all print a text
    /// box that reads like success and keep the item, so issuing them is an endless retry.
    UseBagItem { item: ItemId, target: crate::pokemon::postgame::items::UseTarget },
    /// **L** — like [`Self::EnterMap`], but it **gives up instead of stalling**: if the transition to
    /// `to_map` is not reachable from where the agent is standing, the step pops with a printed
    /// reason after a bounded wait. The whole point of workstream L is to find the rooms that cannot
    /// be entered, so a tour has to be able to survive finding one — `EnterMap`'s deliberate hard
    /// stall (which is right for scripted forward travel) would end the tour at the first locked door
    /// and report nothing about the ninety rooms after it.
    EnterMapIfReachable { to_map: Map },
    /// **I3/I4** — pace `on_map`'s grass into a wild battle and use each of `items` once, in order.
    /// The stat items and the Poké Doll are `wIsInBattle`-gated, so being *in* a battle is the whole
    /// cost and one step spends the lot. ⚠️ A Poké Doll ends the battle, so it must be last.
    UseItemsInBattle { on_map: Map, items: &'static [ItemId] },
    // ─────────────────────────────────────────────────────────────────────────────────────────────

    /// Move `qty` of `item` between the bag and PC item storage, at the PC on `map` (Phase 0 tasks
    /// 0.5/0.6). Routing to `map` happens here; everything from the walk to the PC tile onward is
    /// [`crate::pokemon::postgame::item_storage`], which needs to own the A press that opens the PC.
    /// Build with [`Self::deposit_item`] / [`Self::withdraw_item`].
    UseItemPc { op: crate::pokemon::postgame::item_storage::PcItemOp, item: ItemId, qty: u8, map: Map },
    /// **A** — deposit / withdraw / release / change box at the PC on `map`, via `BILL's PC`. Routed
    /// and handed over exactly like [`Self::UseItemPc`]; the driver is
    /// [`crate::pokemon::postgame::pc_box`]. Build with [`Self::deposit_pokemon`],
    /// [`Self::withdraw_pokemon`], [`Self::change_box`], [`Self::release_pokemon`].
    UsePcBox { op: crate::pokemon::postgame::pc_box::PcBoxOp, map: Map },
    /// **C** — a fishing session on `map` with `rod`, running until `goal` is met. Routing to `map`
    /// happens here; each individual cast is [`crate::pokemon::postgame::fishing`]. Build with
    /// [`Self::fish`].
    Fish {
        rod: crate::pokemon::postgame::fishing::Rod,
        map: Map,
        goal: crate::pokemon::postgame::fishing::FishGoal,
    },
    /// **E** — hunt `targets` in the Safari Zone on `map`, for at most `max_trips` paid entries.
    ///
    /// One step spans the whole hunt, including being ejected at 0 steps and walking back in for
    /// another ¥500 — the trip is the game's unit, not the policy's. Both halves live in
    /// [`crate::pokemon::postgame::safari`]: [`safari::pick`](crate::pokemon::postgame::safari::pick)
    /// paces the grass and re-enters, and
    /// [`safari::pick_battle_action`](crate::pokemon::postgame::safari::pick_battle_action) replaces
    /// the blanket RUN for the encounters it starts. Build with [`Self::safari_hunt_steps`].
    SafariHunt { targets: &'static [PokemonSpecies], map: Map, max_trips: u32 },
    /// **E** — walk out of the Safari Zone to the gate mat, from whichever area a hunt ended in.
    ///
    /// Its own step rather than an `enter(SafariZoneGate)` because a hunt can end in two very
    /// different places — deep in the west, or standing on the gate after an ejection — and only the
    /// zone's own chain topology gets out of both. [`crate::pokemon::postgame::safari::exit`].
    SafariExit,
    /// **F** — buy Game Corner coins at the counter until at least `target` are held, ¥1000 → 50 a
    /// time. Routes to `Map::GameCorner` and then talks to the coin clerk once per purchase; the
    /// clerk's YES/NO opens on YES so the generic A-mash answers it and no driver is needed. Pops
    /// early (with a reason) when the money or the 9999-coin case runs out.
    /// [`crate::pokemon::postgame::game_corner`].
    BuyGameCoins { target: u16 },
    /// **F** — sell `item` to the mart clerk on `map`. Routed and handed over exactly like
    /// [`Self::UseItemPc`]: the driver walks the last tiles itself because it must own the whole
    /// conversation, including the Buy/**Sell**/Quit menu that `assert_pokemart_state` would otherwise
    /// take for a purchase. Driver: [`crate::pokemon::postgame::game_corner`].
    SellToMart { map: Map, item: BagItem },
    /// **F** — buy `prize` from a Game Corner prize vendor. Routes to `Map::GameCornerPrizeRoom`, then
    /// the driver faces the vendor's bg-event and drives the prize menu.
    /// [`crate::pokemon::postgame::game_corner`].
    RedeemPrize { prize: crate::pokemon::postgame::game_corner::Prize },
    /// Walk to and pick up an item sprite (a Poké Ball on the ground), staying on this step until
    /// the sprite is gone. Unlike [`Interact`], this does **not** pop after issuing a single walk:
    /// picking up an item can be interrupted (e.g. the Mt Moon fossil area triggers the Super Nerd
    /// battle at the only approach tile), so the step persists and re-issues the walk after each
    /// interruption until the item sprite disappears from the map. Also used to clear item-sprite
    /// blockers that plug a corridor (collecting one Mt Moon fossil opens the exit passage).
    CollectItem(MapSprite),
    DefeatGymLeader { leader: MapSprite, badge: Badge },
    /// Battle a fixed trainer (e.g. an Elite Four member) by walking into its line of sight, then advance
    /// once it's beaten. Unlike `DefeatGymLeader` there's no badge to gate on — completion is detected the
    /// same way as beaten gym trainers (standing in the trainer's LOS with no battle starting). The trainer
    /// faces DOWN, so route to the tile directly below it (`route_to_face_dir(.., Up)`).
    BattleTrainer { trainer: MapSprite },
    /// Walk in grass and throw Pokéballs until a Pokémon is caught. `ball` pins *which* ball to throw:
    /// `Bag::best_pokeball` ranks by effectiveness, so with a Master Ball in the bag every catch spends
    /// it — fine for a legendary, ruinous for an incidental HM-slave. `None` keeps the old "best in the
    /// bag" behaviour; an explicit ball falls back to that once it runs out.
    CatchPokemon { species: PokemonSpecies, on_map: Map, ball: Option<ItemId> },
    /// **H5** — walk `on_map`'s grass throwing balls at anything the dex does not have, until every
    /// species whose encounter share is at least `min_share` percent is owned. The species list is not
    /// passed in: it comes from the ROM's own wild table via [`crate::pokemon::wild`], because the
    /// point of the step is that a route's contents are a fact, not a guess. Driver:
    /// [`crate::pokemon::postgame::aides`]; build with [`Self::dex_sweep_steps`].
    ///
    /// ⚠️ **Needs a box with room in it.** A catch with a full party goes to the open PC box, and a
    /// full box refuses it — which looks like a ball that keeps failing.
    SweepDex { on_map: Map, min_share: u8, ball: Option<ItemId> },
    /// Enable/disable "train this slot" mode: while `Some(slot)`, the battle policy switches that party
    /// member in at the start of each battle so it earns the XP (for levelling a bench mon on the
    /// trainer gauntlet). `None` turns it off (e.g. before a hard fight where the lead must stay in).
    SetTrainSlot(Option<u8>),
    /// Reorder the party so the member in `slot` becomes the lead (slot 0), written straight to RAM
    /// (no menu navigation). Makes a trained bench mon the battle lead so it fights — and earns XP —
    /// from the start of every battle, with no in-battle switch-in needed.
    MovePokemonToFront { target: PartyRef },
    /// Walk in grass until the party member in `slot` reaches at least `target_level`. During grind
    /// battles the policy switches that slot in so it earns the XP (usually `slot: 0`, the lead; use a
    /// higher slot to train a bench mon, e.g. a freshly-evolved Vaporeon).
    GrindUntilLevel { target_level: u8, on_map: Map, slot: u8 },
    /// Buy item from the currently open Pokémart (must follow an Interact with the clerk).
    BuyFromMart { map: Map, item: BagItem },
    /// Teach an HM/TM `item` (e.g. HM01 Cut) to the party member `target` names, from the overworld.
    /// Drives the START → ITEM → use → choose-Pokémon menus; the move-replace menu (if the mon already
    /// knows 4 moves) is handled by the global forget-move handler. Persists until the target knows the
    /// move — so a [`PartyRef::Slot`] aimed at a mon that *cannot* learn it never completes. Prefer
    /// [`PartyRef::Species`] unless the position is genuinely what is meant.
    TeachMove { item: ItemId, target: PartyRef },
    /// Use an evolution `stone` (e.g. Water Stone) from the bag on the party member `target` names, to
    /// evolve it (e.g. Eevee → Vaporeon). Persists until that mon's species changes.
    EvolveWithStone { stone: ItemId, target: PartyRef },
    /// Use a Rare Candy from the bag on the party member in `slot` (levels it up and, crucially, frees
    /// a bag slot). Drives the same START → ITEM → USE → choose-Pokémon menus as teaching a move;
    /// persists until the Rare Candy is consumed (no longer in the bag).
    UseRareCandy { slot: u8 },
    /// Toss `item` from the bag to free a slot. Gen 1's bag holds **20** items and a mart purchase of a
    /// *new* item silently fails once it is full — the clerk says "You can't carry any more items", the
    /// `BuyFromMart` step retries and gives up, and the leg carries on without what it bought. Note that
    /// `state.bag` under-reports: `Bag`'s reader drops every id `ItemId` cannot name (all the TMs), so a
    /// bag printing 13 entries can be at 19. Pops immediately if the item isn't held, so it is safe to
    /// leave in a step list. Key items and HMs cannot be tossed (pokered `IsKeyItem`/`IsItemHM`).
    TossItem { item: ItemId },
    /// Use the **DIG** field move (TM28) from the party menu with the mon in `slot` — Gen 1's reusable
    /// Escape Rope. In any `EscapeRopeTilesets` map (Cavern included) it warps the player to
    /// `wLastBlackoutMap`, the town of the last Pokémon Center used, so healing before a dungeon also
    /// chooses where Dig lands. Completes on the map change. This is how the Seafoam leg gets home:
    /// every walkable route back east is script-sealed until the boulder chain no one needs is done.
    Dig { slot: u8 },
    /// Cut down a tree blocking the way on `map` (requires Cut + the Cascade Badge). Routes to face a
    /// `MetaTile::CutTree`, then uses the Cut field move. Persists until no reachable tree remains.
    CutTree { map: Map },
    /// Activate Strength using the party mon `target` names (an HM-slave that knows it). Completes once
    /// `BIT_STRENGTH_ACTIVE` is set. Strength resets on every map change, so re-issue it per floor
    /// before pushing boulders. Only meaningful once a party member knows Strength.
    UseStrength { target: PartyRef },
    /// Push a boulder onto the Strength switch at `switch` (a cave floor coordinate), solving the
    /// current floor's boulder puzzle. Requires `BIT_STRENGTH_ACTIVE` already armed (issue `UseStrength`
    /// first). The agent runs `MetaTileMap::solve_boulder_push(switch)` to plan the pushes and drives
    /// them. Completes once a boulder sits on `switch` (the map script then sets the switch event and
    /// opens the barrier). Re-solvable from any partial state, so it resumes after a wild battle.
    SolveBoulders { switch: crate::geometry::Point8 },
    /// Push a boulder onto a floor `hole` (Victory Road 3F) so it falls to the floor below — revealing a
    /// hidden boulder there (VR2F's second-switch boulder). Reuses the boulder solver/executor aimed at
    /// the hole tile; completes once one boulder has fallen (the visible count drops). Requires Strength.
    DropBoulderInHole { hole: crate::geometry::Point8 },
    /// Solve the Vermilion Gym trash-can switch puzzle: check the first switch can, then the second,
    /// unlocking the door to Lt. Surge. The correct cans are read from RAM (`GameState::trash_cans`)
    /// so the agent goes straight to them and never triggers a reset. Persists until the 2nd lock is
    /// open. Only meaningful on `Map::VermilionGym`.
    SolveTrashCans,
    /// Walk to face the hidden switch/poster BG-event tile at `at` on `map` and press A, until doing so
    /// reveals a passage — a reachable warp/connection to `reveals` appears (e.g. the Celadon Game
    /// Corner poster flips a switch that opens the staircase down to the Rocket Hideout). The tile is a
    /// `bg_event`, not a sprite, so `Interact` can't target it. Idempotent: re-pressing after the reveal
    /// is harmless; the step pops once `reveals` is reachable.
    FlipSwitch { map: Map, at: Point8, reveals: Map },
    /// Inside an elevator room, use the floor panel to travel to the floor at menu index `floor`.
    /// Faces the panel bg-event, opens the floor list-menu, navigates the cursor to `floor`, confirms,
    /// then steps back onto the elevator warp (whose destination the menu redirected at runtime) — the
    /// step completes when the resulting warp changes the map. Used for the Rocket Hideout elevator
    /// (needs the Lift Key) to reach Giovanni's split-off B4F room.
    UseElevator { panel: Point8, floor: u8 },
    /// Face the sprite `target`, then **use** the bag item `item` on it from the field (START → ITEM →
    /// select → USE). Used for the Poké Flute on a road-blocking Snorlax: the item's effect starts a
    /// battle, which the normal battle handler wins; the step completes once `target` is gone.
    UseFieldItem { item: ItemId, target: MapSprite },
    /// Face the vending-machine bg-event at `at` and press A to buy `drink` (the machine's menu opens
    /// with the cheapest drink at the cursor, so A-mashing selects it). Persists until `drink` is in the
    /// bag. Used for the Celadon Mart roof drink needed to pass the Saffron gate guards.
    UseVendingMachine { at: Point8, drink: ItemId },
}

/// A non-walking overworld action the agent performs directly (opening menus / using field moves),
/// requested by the policy when the corresponding queue step is at the front.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FieldMove {
    /// Reorder the party so `slot` becomes the lead (RAM write, no menus). Single-shot.
    ReorderParty { slot: u8 },
    /// Teach `item` (an HM/TM) to the party member in `target_slot` via the bag.
    TeachMove { item: ItemId, target_slot: u8 },
    /// Use evolution `stone` on the party member in `target_slot` (bag → stone → USE → pick the mon),
    /// evolving it. `evolve_from` is the species before evolving, so completion = the slot's species
    /// changed. Shares the `TeachingMove` menu-driver (same START→ITEM→bag→USE→party chain).
    EvolveWithStone { stone: ItemId, target_slot: u8, evolve_from: PokemonSpecies },
    /// Use the Cut field move on the tree the player is currently facing.
    CutTree,
    /// Walk to the tile at `target` and press A. `facing`, if set, forces the approach so the player
    /// ends up facing that direction (needed for Pokémon Mansion statue switches, which only trigger
    /// when faced from directly below, i.e. facing Up).
    ///
    /// Named for the Vermilion Gym cans it was written for, but it is the generic **face a tile and
    /// interact** move, and three unrelated steps ride it: `SolveTrashCans`, `FlipSwitch` (Mansion
    /// statues, the Rocket Hideout poster) and `SearchHiddenItem`. They share it because the ROM does:
    /// all three are `hidden_event`s, dispatched by `CheckForHiddenEvent` when A is pressed and the
    /// tile in front of the player matches.
    CheckTrashCan { target: crate::geometry::Point8, facing: Option<crate::pokemon::map_metadata::PlayerFacingDirection> },
    /// Drive the elevator floor menu (panel at `panel`) to select menu index `floor`, then ride the
    /// redirected warp out. Done when the map changes (we've left the elevator room).
    UseElevator { panel: crate::geometry::Point8, floor: u8 },
    /// Face the sprite at `target`, then use bag `item` on it (START → ITEM → select → USE). The
    /// item's field effect (e.g. the Poké Flute waking a Snorlax) does the rest.
    UseFieldItem { item: ItemId, target: crate::geometry::Point8 },
    /// **G** — a party-menu script: walk to `npc` (a *(tile to face, direction)* pair, resolved from
    /// `actions()` because `route_to_face_dir` cannot reach a sprite behind a counter), then drive
    /// the party menu to `slot`. Driver: [`crate::pokemon::postgame::gifts::tick`].
    UsePartyScript {
        script: crate::pokemon::postgame::gifts::PartyScript,
        slot: u8,
        npc: (crate::geometry::Point8, crate::pokemon::map_metadata::PlayerFacingDirection),
    },
    /// Use a field move from the party menu: START → POKéMON → the mon at `slot` → the field-move entry
    /// at `move_index`. That menu lists only the mon's *field* moves (`FieldMoveDisplayData`) and keeps
    /// them in its move-slot order, so the index depends on what else the mon knows — the policy
    /// computes it from the live move list (`field_move_index`). Strength arms `BIT_STRENGTH_ACTIVE`;
    /// Dig warps the player out of the cave.
    UseFieldMove { slot: u8, move_index: u8 },
    /// Toss `item` from the bag (START → ITEM → the item → TOSS → quantity → YES) to free a slot.
    TossItem { item: ItemId },
    /// Walk to the PC at `pc`, open it, and move `qty` of `item` between the bag and PC item storage.
    /// Driven by [`crate::pokemon::postgame::item_storage`] (Phase 0 tasks 0.5/0.6).
    UseItemPc { op: crate::pokemon::postgame::item_storage::PcItemOp, item: ItemId, qty: u8, pc: crate::geometry::Point8 },
    /// **Workstream A** — walk to the PC at `pc` and drive Bill's PC box menus. Driven by
    /// [`crate::pokemon::postgame::pc_box`].
    UsePcBox { op: crate::pokemon::postgame::pc_box::PcBoxOp, pc: crate::geometry::Point8 },

    // ── Reserved postgame seams (task 0.8) ──────────────────────────────────────────────────────
    // The entry points for the reserved `AgentState`s. `agent.rs` already turns each of these into
    // its state in one line, so a workstream only has to return one from its own `pick_field_move`.
    /// **B** — Fly to `to`.
    Fly { to: Map },
    /// **C** — cast `rod` **once** at the water tile `at`. Driven by
    /// [`crate::pokemon::postgame::fishing`].
    Fish { rod: crate::pokemon::postgame::fishing::Rod, at: crate::geometry::Point8 },
    /// **F** — walk to the mart clerk at `clerk` and sell `item` to them. Driven by
    /// [`crate::pokemon::postgame::game_corner`].
    SellToMart { item: BagItem, clerk: (crate::geometry::Point8, crate::pokemon::map_metadata::PlayerFacingDirection) },
    /// **F** — walk to the prize vendor's bg-event tile and buy `prize` with coins. Driven by
    /// [`crate::pokemon::postgame::game_corner`].
    RedeemPrize { prize: crate::pokemon::postgame::game_corner::Prize },
    /// **I** — use bag `item` on `target` from the overworld. Driven by
    /// [`crate::pokemon::postgame::items`].
    UseBagItem { item: ItemId, target: crate::pokemon::postgame::items::UseTarget },
    // (**H** reserved a `SearchHiddenItem` field move here. It is gone: a hidden item is collected by
    // facing its tile and pressing A, which is exactly `CheckTrashCan` — see
    // [`crate::pokemon::postgame::aides`].)
    // ────────────────────────────────────────────────────────────────────────────────────────────
    /// Primitive Strength push: shove the boulder at `boulder` one tile in `dir` (Strength must be armed).
    /// The agent routes behind the boulder and double-presses; it completes as soon as the boulder leaves
    /// its tile. A policy plans *which* boulder/direction with the `MetaTileMap::solve_boulder_push` helper
    /// (or, for an LLM, by reasoning over `map.sprites` + `map.strength_switches`), then issues these one
    /// at a time.
    PushBoulder { boulder: crate::geometry::Point8, dir: crate::joypad::JoypadButton },
}

/// True for the four Pokémon Mansion floors, whose statue switches only trigger when faced from below.
fn is_mansion_floor(map: Map) -> bool {
    matches!(map, Map::PokemonMansion1F | Map::PokemonMansion2F | Map::PokemonMansion3F | Map::PokemonMansionB1F)
}

/// The move an HM item teaches (HM01 Cut … HM05 Flash), used to check whether a mon already knows it.
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

/// The moves that get their own entry in the party menu's field-move list, from pokered
/// `FieldMoveDisplayData`. (Its ordering column is display-only — the menu itself lists a mon's field
/// moves in move-slot order, which is why the index has to be computed per mon.)
fn is_field_move(name: PokemonMoveName) -> bool {
    matches!(name, PokemonMoveName::Cut | PokemonMoveName::Fly | PokemonMoveName::Surf
        | PokemonMoveName::Strength | PokemonMoveName::Flash | PokemonMoveName::Dig
        | PokemonMoveName::Teleport | PokemonMoveName::Softboiled)
}

/// Where `want` sits in the field-move menu for the party member in `slot`: the count of field moves
/// it knows in earlier move slots. Defaults to 0 if the mon or the move is missing, which is what a
/// lone-field-move HM slave would use anyway.
pub(crate) fn field_move_index(state: &GameState, slot: u8, want: PokemonMoveName) -> u8 {
    state.pokemon.get(slot as usize).map_or(0, |mon| field_move_index_of(mon, want))
}

/// `want`'s row in `mon`'s field-move box, for callers that hold the mon but not a whole `GameState`.
pub(crate) fn field_move_index_of(mon: &crate::pokemon::pokemon::Pokemon, want: PokemonMoveName) -> u8 {
    mon.moves.iter().flatten().map(|m| m.name).filter(|&n| is_field_move(n))
        .position(|n| n == want).unwrap_or(0) as u8
}

impl PolicyStep {
    /// The Victory Road boulder slave, named by species because it is caught *inside*
    /// [`Self::victory_road_1f_steps`] — the slot it lands in depends on how many mons the run arrived
    /// with, which is exactly what the old `machop_slot` argument was guessing at (and `complete_game_steps`
    /// and the leg test guessed differently: 4 versus 2).
    const MACHOP: PartyRef = PartyRef::Species(PokemonSpecies::Machop);

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

    /// Bank `qty` of `item` in PC item storage, freeing bag slots. `map` must have a PC —
    /// any Pokémon Center will do (see `MetaTileMap::pc_locations`).
    pub const fn deposit_item(item: ItemId, qty: u8, map: Map) -> Self {
        Self::UseItemPc { op: crate::pokemon::postgame::item_storage::PcItemOp::Deposit, item, qty, map }
    }

    /// Take `qty` of `item` back out of PC item storage.
    pub const fn withdraw_item(item: ItemId, qty: u8, map: Map) -> Self {
        Self::UseItemPc { op: crate::pokemon::postgame::item_storage::PcItemOp::Withdraw, item, qty, map }
    }

    /// Explicit forward transition to `map`, disambiguated by the raw landing `to_position`.
    pub const fn enter_at(map: Map, x: u8, y: u8) -> Self {
        Self::EnterMap { to_map: map, to_position: Some(Point8 { x, y }) }
    }

    /// The explicit Mt Moon crossing (1F west entrance → Route 4 east exit), including the fossil
    /// chokepoint. Requires standing in Mt Moon 1F. See `mt_moon_traversal` doc in the tests.
    pub fn mt_moon_traversal() -> Vec<Self> { vec![
        Self::enter_at(Map::MtMoonB1F, 5, 5),
        Self::enter_at(Map::MtMoonB2F, 21, 17),
        Self::CollectItem(MapSprite::MTMOONB2F_HELIX_FOSSIL),
        Self::enter_at(Map::MtMoonB1F, 23, 3),
        Self::enter(Map::Route4),
        Self::enter(Map::CeruleanCity),
    ] }

    /// The Bill's-House SS-Ticket sub-sequence (pokered `scripts/BillsHouse.asm`), assuming the
    /// agent is already inside `BillsHouse`: talk to Bill's Pokémon (A-mash picks the default YES →
    /// it walks into the cell separator) → use the PC (runs the Cell Separation System, Bill exits
    /// the machine) → talk to Bill for the SS Ticket. Bill's exit is a ~1-2s scripted walk, so an
    /// `Interact` issued mid-script aborts (reason `Script`); retry a few times so one lands after he
    /// settles (extra talks after the ticket is received are harmless — same text, no re-give).
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

    /// Board the S.S. Anne (from Vermilion City, SS Ticket in the bag), defeat every trainer in the
    /// ship's cabins to level the party, beat the rival guarding the captain's door, and receive
    /// **HM01 Cut** from the captain.
    ///
    /// Cabins are disconnected rooms within the `*Rooms` maps, each reached by a distinct warp
    /// landing (`enter_at` disambiguates); we visit them one by one and `Interact` each trainer
    /// (walking up + A starts a trainer battle). There is **no Pokémon Center on the ship**, so each
    /// floor is a self-contained heal → board → sweep → disembark cycle that returns to Vermilion —
    /// the lone starter would otherwise be worn down by attrition. Coordinates are decoded from
    /// pokered `data/maps/objects/SSAnne*Rooms.asm`. Floors are ordered to level the party as high as
    /// possible before the rival (a single 6-Pokémon battle with no mid-battle healing).
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
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity)]); // disembark

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
        s.extend([Self::enter(Map::SSAnne1F), Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity)]); // disembark

        // ── 2F cabins (4 trainers) + Bow (2 trainers, via 3F) ──
        s.extend(Self::heal_at_vermilion());
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::SSAnne1F), Self::enter(Map::SSAnne2F)]);
        s.extend([
            Self::enter_at(Map::SSAnne2FRooms, 12, 5), Self::Interact(MapSprite::SSANNE2FROOMS_GENTLEMAN1),
                                                       Self::Interact(MapSprite::SSANNE2FROOMS_FISHER), Self::enter(Map::SSAnne2F),
            Self::enter_at(Map::SSAnne2FRooms, 2, 15), Self::Interact(MapSprite::SSANNE2FROOMS_GENTLEMAN2),
                                                       Self::Interact(MapSprite::SSANNE2FROOMS_COOLTRAINER_F), Self::enter(Map::SSAnne2F),
        ]);
        // Bow: SSAnne2F → SSAnne3F → SSAnneBow (one open room, two sailors). Party is strong by now.
        s.extend([
            Self::enter(Map::SSAnne3F), Self::enter(Map::SSAnneBow),
            Self::Interact(MapSprite::SSANNEBOW_SAILOR2), Self::Interact(MapSprite::SSANNEBOW_SAILOR3),
            Self::enter(Map::SSAnne3F), Self::enter(Map::SSAnne2F),
        ]);
        s.extend([Self::enter(Map::SSAnne1F), Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity)]); // disembark

        // ── Rival + Captain (HM01) ── (heal first — the rival is 6 Pokémon in one battle)
        s.extend(Self::heal_at_vermilion());
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::SSAnne1F), Self::enter(Map::SSAnne2F)]);
        s.push(Self::enter(Map::SSAnneCaptainsRoom)); // rival battle triggers on approach to the (36,4) warp
        s.extend(std::iter::repeat(Self::Interact(MapSprite::SSANNECAPTAINSROOM_CAPTAIN)).take(4));
        // ── Disembark back to Vermilion (after HM01 the ship departs on the way out of the dock) ──
        s.extend([
            Self::enter(Map::SSAnne2F), Self::enter(Map::SSAnne1F),
            Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity),
        ]);
        s
    }

    /// From Cerulean City (post-Cascade): fetch the **SS Ticket** from Bill, then cross to Vermilion
    /// City via the **trashed-house terrace bridge** + Underground Path (Route 5 → 6). The trashed
    /// house is the only way between Cerulean's split terraces: its back door lands in the Route-5
    /// terrace (`enter_at(CeruleanCity, 27, 9)` — front door ~27,11 does not reach it). See
    /// `can_reach_vermilion`. Bill's guard on Route 25 clears once you meet him, opening the bridge.
    pub fn cerulean_to_vermilion_steps() -> Vec<Self> {
        let mut steps = vec![
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::Route24),
            Self::enter(Map::Route25),
            Self::enter(Map::BillsHouse),
        ];
        steps.extend(Self::bill_ss_ticket_steps());
        steps.extend([
            Self::enter(Map::Route25),
            Self::enter(Map::Route24),
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

    /// Thunder Badge (from Vermilion City after the S.S. Anne, with **HM01 Cut** in the bag): teach Cut
    /// to the starter, cut the tree sealing the gym enclosure, solve the two-switch **trash-can
    /// puzzle** (which unlocks the door), then beat Lt. Surge. All via the real UI — see
    /// `can_get_thunder_badge` (integrated) and `can_teach_cut` / `can_cut_gym_tree` /
    /// `can_beat_lt_surge` (focused). `SolveTrashCans` must precede `DefeatGymLeader`: the door is
    /// shut (Surge unreachable) until both switches are hit.
    pub fn thunder_badge_steps() -> Vec<Self> {
        let mut s = Self::heal_at_vermilion();
        s.extend([
            Self::TeachMove { item: ItemId::Hm01Cut, target: PartyRef::Slot(0) }, // the lead: `CuttingTree` only ever asks slot 0
            Self::CutTree { map: Map::VermilionCity },
            Self::enter(Map::VermilionGym),
            Self::SolveTrashCans,
            Self::DefeatGymLeader { leader: MapSprite::VERMILIONGYM_LT_SURGE, badge: Badge::ThunderBadge },
        ]);
        s
    }

    /// Head back from Vermilion (just after the Thunder Badge, standing inside the gym) to Cerulean
    /// City, reusing the Underground Path in reverse. Saffron's south gate (Route 6) is guard-blocked,
    /// so the Underground Path (Route 5 ↔ Route 6) is the only legal way north. Exiting the gym drops
    /// the player into the Cut-tree enclosure (the tree regrows on re-entering the map), so cut it
    /// again before reaching the rest of the city. Heal at both ends of the trek.
    pub fn back_to_cerulean_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::VermilionCity), // exit the gym into the Cut-tree enclosure
            Self::CutTree { map: Map::VermilionCity },
        ];
        s.extend(Self::heal_at_vermilion());
        s.extend([
            // **Stock healing items before leaving Vermilion.** Everything from here to Celadon —
            // Rock Tunnel's encounter-dense maze, then seven floors of Pokémon Tower — is
            // Pokémon-Center-less and fought by a *lone* starter, and the run used to carry no healing
            // items at all: `pick_battle_action`'s "HP critical — using healing item" branch works, it
            // simply had nothing to reach for, so attrition ended the run rather than costing it a
            // detour. Vermilion is the **first mart on the route that stocks Super Potions**
            // (`data/items/marts.asm:17`); Cerulean, the next one, sells only the +20 Potion.
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

    /// The Rock Tunnel warp-maze crossing (Route 10 north entrance → Route 10 south exit), discovered
    /// offline by `discover_rock_tunnel_path` (ExplorerPolicy). Assumes the agent stands on Route 10
    /// having just come from Route 9. No Flash needed — the agent routes from RAM tile collision, not
    /// the darkened screen.
    pub fn rock_tunnel_traversal() -> Vec<Self> { vec![
        // North entrance → a 4-hop 1F↔B1F chain → south exit. Warp pairs (from ROM + real-engine
        // probing): each `enter_at` lands in a region whose only forward (unvisited) warp is the next.
        Self::enter_at(Map::RockTunnel1F, 15, 3),   // Route 10 north entrance
        Self::enter_at(Map::RockTunnelB1F, 33, 25),
        Self::enter_at(Map::RockTunnel1F, 5, 3),
        Self::enter_at(Map::RockTunnelB1F, 23, 11),
        Self::enter_at(Map::RockTunnel1F, 37, 17),
        Self::enter_at(Map::Route10, 8, 53),        // south exit (→ Lavender)
    ] }

    /// Cerulean City (main terrace, post-Thunder) → Lavender Town. Route 9 (east) is on a separate
    /// Cerulean terrace reached via the **trashed-house back door** (27,9) — the same bridge used to
    /// reach Route 5/Vermilion. Route 9's west-entry pocket is sealed by a **Cut tree at (5,8)**; cut
    /// it to cross east. Then Route 10 → **Rock Tunnel** (warp maze) → Route 10 south → Lavender.
    pub fn cerulean_to_lavender_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::CeruleanTrashedHouse),   // main terrace front door
            Self::enter_at(Map::CeruleanCity, 27, 9), // back door → Route-9 terrace
            Self::enter(Map::Route9),
            Self::CutTree { map: Map::Route9 },        // cut the (5,8) tree boxing the west pocket
            Self::enter(Map::Route10),
            // Heal at the Rock Tunnel Pokémon Center (Route 10, at the tunnel mouth) before diving in:
            // the encounter-dense maze must be crossed in one uninterrupted push (a mid-tunnel
            // flee-to-heal or blackout can't resume the scripted warp chain), so enter at full HP/PP.
            // This also makes it the nearest heal-return target if PP still runs low mid-crossing.
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

    /// Lavender Town → Celadon City via the **Route 7–8 Underground Path** (all four Saffron gates
    /// demand a drink only sold in Celadon — a chicken/egg — so Saffron is bypassed). Linear tunnel,
    /// same building-tunnel-building shape as the Route 5–6 path already used: Lavender → Route 8 →
    /// `UndergroundPathRoute8` → `UndergroundPathWestEast` → `UndergroundPathRoute7` → Route 7 →
    /// Celadon City, then heal at the Celadon Center.
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

    /// Rainbow Badge (from Celadon City): the gym entrance is sealed by a row of trees, so cut them,
    /// enter, and beat Erika. `DefeatGymLeader` persists until the badge is won (self-heals on a
    /// blackout and re-routes through the grass-maze junior trainers). Erika's team is all Grass/Poison
    /// (Victreebel/Tangela/Vileplume ~lv24–29); Grass moves are resisted, so the starter leans on its
    /// Normal move (Cut/Body Slam) + level lead — the party is ~lv35+ Venusaur by now.
    pub fn celadon_rainbow_steps() -> Vec<Self> {
        vec![
            Self::CutTree { map: Map::CeladonCity },   // cut the trees sealing the gym entrance
            Self::enter(Map::CeladonGym),
            // The gym is a garden maze whose paths are blocked by real cuttable trees (GYM tileset
            // tile $50 — pokered `cut.asm`). Cut them to weave up to Erika (junior trainers engage by
            // LOS en route). `CutTree` persists until no reachable tree remains, so it clears each
            // chokepoint as the previous cut opens access to the next.
            Self::CutTree { map: Map::CeladonGym },
            Self::DefeatGymLeader { leader: MapSprite::CELADONGYM_ERIKA, badge: Badge::RainbowBadge },
        ]
    }

    /// From Celadon City (post-Erika, inside the gym) to inside the **Rocket Hideout** (B1F). Exit the
    /// gym — its entrance trees regrew on re-entry, so re-cut them — heal, walk to the **Game Corner**,
    /// beat the Rocket guarding the poster (he vanishes on defeat), flip the poster switch to open the
    /// hidden staircase, and descend. Getting the **Silph Scope** (needed for the Poké Flute) means
    /// crossing the hideout's spinner-tile floors + elevator to Giovanni — handled separately.
    pub fn rocket_hideout_entrance_steps() -> Vec<Self> {
        let mut s = vec![
            // Beating Erika reloaded the map, so the gym's internal garden trees regrew and now wall
            // the player in — re-cut them to reach the gym exit warp before leaving (the junior
            // trainers are already beaten, so re-crossing the garden starts no new battles).
            Self::CutTree { map: Map::CeladonGym },
            Self::enter(Map::CeladonCity),          // exit the gym into the (regrown) tree enclosure
            Self::CutTree { map: Map::CeladonCity }, // re-cut to reach the rest of the city
            Self::enter(Map::CeladonPokecenter),
            Self::Interact(MapSprite::CELADONPOKECENTER_NURSE),
            Self::enter(Map::CeladonCity),
            Self::enter(Map::GameCorner),
        ];
        // The Rocket stands on (9,5) blocking the poster at (9,4) — beat him (he vanishes on defeat,
        // freeing (9,5)), then flip the poster switch to open the hidden staircase and descend. A
        // single `Interact` (not retried): it pops the instant it issues the walk, so it never hangs
        // after the Rocket vanishes; the ensuing `FlipSwitch` waits out the battle and then flips.
        s.extend([
            Self::Interact(MapSprite::GAMECORNER_ROCKET),
            Self::FlipSwitch { map: Map::GameCorner, at: Point8 { x: 9, y: 4 }, reveals: Map::RocketHideoutB1F },
            Self::enter(Map::RocketHideoutB1F),
        ]);
        s
    }

    /// From inside the Rocket Hideout (B1F), descend the spinner floors B2F/B3F to B4F and get the
    /// **Lift Key**. B2F/B3F are **spinner-tile floors** (arrow tiles force a fixed slide, modelled in
    /// the BFS via `MetaTileMap::spinners`). B4F is split — the stairs land in a left room; beating
    /// Rocket 3 isn't enough, his **after-battle text** (a second talk) sets EVENT_ROCKET_DROPPED_LIFT_KEY
    /// and `ShowObject`s the Lift Key ball at (10,2). He stays put after defeat, so Interact him a few
    /// times (battle, then the reveal talk), then grab the key (`CollectItem` waits for the ball to
    /// appear — see `collect_item_seen`).
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

    /// From inside the Rocket Hideout (B1F), get the **Silph Scope** (needed to see the Pokémon Tower
    /// ghosts → Poké Flute). First get the Lift Key (`lift_key_steps`), then take the **elevator** to
    /// Giovanni's split-off B4F room.
    ///
    /// Two runtime door blocks gate this (modelled via `MetaTileMap` door overlays so BFS avoids them
    /// until open): (1) the **B1F elevator door** stays shut until Rocket 5 is beaten — so we enter the
    /// elevator from **B2F** instead (its own elevator warp is ungated), and the BFS reroutes there
    /// automatically. (2) On B4F the elevator lands in the lower room, walled off from Giovanni by a
    /// **door that opens only after both Rockets (trainers 0 & 1) are beaten** — so fight them first,
    /// which drops the wall on the post-battle map reload. Then beat Giovanni (Grass starter is 4×
    /// on his Ground/Rock team; he vanishes on defeat and `ShowObject`s the Scope ball at (25,2)).
    pub fn silph_scope_steps() -> Vec<Self> {
        let mut s = Self::lift_key_steps();
        s.extend([
            // Back up to B2F (spinner nav works both ways) and into the elevator (B2F's warp is not
            // gated by the Rocket-5 door, unlike B1F's).
            Self::enter(Map::RocketHideoutB3F),
            Self::enter(Map::RocketHideoutB2F),
            Self::enter(Map::RocketHideoutElevator),
            // Panel bg-event at (1,1); floors are B1F(0)/B2F(1)/B4F(2) — pick B4F. The menu redirects
            // the exit warp to B4F (25,15) in Giovanni's lower room.
            Self::UseElevator { panel: Point8 { x: 1, y: 1 }, floor: 2 },
            // Beat both Rockets to open the door up to Giovanni (single Interact each — trainers stay
            // put after defeat, so a lone talk suffices and the step pops once it issues the walk).
            Self::Interact(MapSprite::ROCKETHIDEOUTB4F_ROCKET1),
            Self::Interact(MapSprite::ROCKETHIDEOUTB4F_ROCKET2),
            // Beat Giovanni (single Interact — he vanishes on defeat, revealing the Scope), then collect.
            Self::Interact(MapSprite::ROCKETHIDEOUTB4F_GIOVANNI),
            Self::CollectItem(MapSprite::ROCKETHIDEOUTB4F_SILPH_SCOPE),
        ]);
        s
    }

    /// From inside the Rocket Hideout (post-Giovanni, holding the Silph Scope), get the **Poké Flute**:
    /// leave the hideout, travel to Lavender Town, climb **Pokémon Tower** to 7F, and rescue Mr. Fuji.
    ///
    /// Exit is via the elevator to **B2F** (Giovanni's B4F room is walled off; the B1F elevator warp
    /// lands behind the still-shut Rocket-5 door, so ride to B2F and take the stairs up to B1F instead),
    /// then out to the Game Corner and Celadon. Heal, then cross the **Route 7–8 Underground Path** to
    /// Lavender (reverse of `lavender_to_celadon_steps`). In the tower the Channelers engage by sight as
    /// the agent climbs; on 6F stepping toward the 7F stairs triggers the **ghost Marowak** (a scripted
    /// lv30 battle now visible thanks to the Scope); on 7F the three Rockets fall and then Mr. Fuji warps
    /// the player to his house, where talking to him hands over the Poké Flute.
    pub fn poke_flute_steps() -> Vec<Self> {
        let mut s = vec![
            // Leave the hideout: elevator (from Giovanni's isolated B4F room) down to B2F, up to B1F,
            // out to the Game Corner, into Celadon; then heal.
            Self::enter(Map::RocketHideoutElevator),
            Self::UseElevator { panel: Point8 { x: 1, y: 1 }, floor: 1 }, // B2F = menu index 1
            // **B1F is two disconnected halves**, split by the full-width wall at row 16, and B2F has a
            // staircase into each: (21,22) → B1F (21,24) in the south half, (27,8) → B1F (23,2) in the
            // north. Only the north half holds the Game Corner staircase (21,2). A bare
            // `enter(RocketHideoutB1F)` takes the *nearest* warp, which off the elevator is the southern
            // one at 10 steps against the northern one's 33 — and the south half's only other exit is
            // the elevator, behind the still-shut Rocket-5 door at column 23. So name the landing.
            // (`fuchsia::probe_hideout_b1f_halves` dumps both halves and the two landings.)
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
            // Heal at Lavender before diving into the tower: it's a long, trainer-heavy climb (7 floors
            // of Channelers + the ghost Marowak + three 7F Rockets) with NO Pokémon Center inside, so a
            // worn-down lone starter can black out mid-climb — and a tower black-out can't resume the
            // scripted deep-interior Mr. Fuji rescue (the Interact steps pop "no path" from the far-away
            // respawn), skipping it. A fresh full-HP/PP party clears the tower in one push. This also
            // makes Lavender the nearest respawn if it does black out. (Same rule as Rock Tunnel.)
            Self::enter(Map::LavenderPokecenter),
            Self::Interact(MapSprite::LAVENDERPOKECENTER_NURSE),
            // Top the Super Potions back up at the Lavender mart before climbing — full HP at the door
            // is not enough on its own. The tower is the single longest unbroken fight in the run for a
            // lone starter (7 floors of Channelers, the ghost Marowak, then three Rockets, no Center
            // inside), and a Rocket's **Drowzee sleeps the lead and kills it through the sleep**: with
            // an empty bag there is no answer to that, because a sleeping mon cannot attack but the
            // *player* can still use an item. That black-out is what stalled `full_playthrough` at
            // Mr Fuji's House — see the note on the rescue steps below.
            Self::enter(Map::LavenderMart),
            Self::BuyFromMart { item: BagItem::new(ItemId::SuperPotion, 10), map: Map::LavenderMart },
            Self::enter(Map::LavenderTown),
        ]);
        // Climb the tower. Each up-warp is at the same corner on consecutive floors; Channelers engage
        // by line of sight as the agent routes to each warp. On 6F the walk to the 7F stairs crosses the
        // ghost-Marowak trigger tile.
        s.extend([
            Self::enter(Map::PokemonTower1F),
            Self::enter(Map::PokemonTower2F),
            Self::enter(Map::PokemonTower3F),
            Self::enter(Map::PokemonTower4F),
            Self::enter(Map::PokemonTower5F),
            Self::enter(Map::PokemonTower6F),
            // The Rare Candy ball at (6,8) blocks the *only* chokepoint into the 6F sub-region that
            // holds the ghost-Marowak trigger and the 7F stairs — collect it to open the path.
            Self::CollectItem(MapSprite::POKEMONTOWER6F_RARE_CANDY),
            Self::enter(Map::PokemonTower7F),
        ]);
        // 7F: beat the three Rockets (they leave on defeat), then talk to Mr. Fuji — his script warps
        // the player to Mr. Fuji's house. There, talk to him again to receive the Poké Flute.
        s.extend([
            Self::Interact(MapSprite::POKEMONTOWER7F_ROCKET1),
            Self::Interact(MapSprite::POKEMONTOWER7F_ROCKET2),
            Self::Interact(MapSprite::POKEMONTOWER7F_ROCKET3),
            // ⚠️ **Re-assert the floor before the rescue, so a black-out is survivable.** `Interact`
            // pops the moment it issues its walk, and an `OverworldActionAborted { reason: WrongMap }`
            // pops it too — so if the party faints up here, the respawn in Lavender silently *skips*
            // the rescue and the run wedges on the next step instead, in Mr Fuji's house, staring at a
            // Mr Fuji who is still `hidden` because he is still in the tower. That failure is 200 steps
            // and ten minutes of game time away from its cause, which is what made it expensive to
            // diagnose. `goto` re-climbs (the Channelers stay beaten, so it is quick) and is a no-op
            // when we are already standing here.
            Self::goto(Map::PokemonTower7F),
            Self::Interact(MapSprite::POKEMONTOWER7F_MR_FUJI),
        ]);
        // ⚠️ **Talk to Mr Fuji at home more than once.** His tower script warps the player into his
        // house *standing on the door tile*, and a plain `Interact` pops the moment it issues its walk
        // — so the first one pops while the warp is still resolving and the Flute is never collected.
        // The leg test `fuchsia::can_get_poke_flute` cannot see this: it uses `run_leg`, which keeps
        // stepping after the queue empties until the Flute appears, and the agent gets there on its own.
        // `full_playthrough` has no such tolerance — its queue moves straight on to `enter(LavenderTown)`
        // with the player still parked on the door, and the run wedged there for the whole budget.
        // Repeating is the same idiom `silph_giovanni_steps` uses for Giovanni; each extra one is a
        // no-op once the conversation has happened.
        for _ in 0..4 { s.push(Self::Interact(MapSprite::MRFUJISHOUSE_MR_FUJI)); }
        s
    }

    /// With the Poké Flute, wake the **Snorlax** blocking **Route 12** (south of Lavender), opening the
    /// road toward Fuchsia. From Mr. Fuji's house: out to Lavender, south onto Route 12, then use the
    /// Poké Flute while facing the Snorlax — that starts a lv30 wild battle the party fights normally;
    /// the sprite is gone once it faints, which pops the `UseFieldItem` step.
    pub fn snorlax_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::LavenderTown), // leave Mr. Fuji's house
            // Heal at Lavender: the party has fought all through the tower with no rest, and the long
            // Route 12–15 trainer gauntlet ahead will black it out otherwise. Also makes Lavender the
            // fallback center for any low-PP heal-flee on the way south.
            Self::enter(Map::LavenderPokecenter),
            Self::Interact(MapSprite::LAVENDERPOKECENTER_NURSE),
            Self::enter(Map::LavenderTown),
            Self::enter(Map::Route12),      // south connection off Lavender (lands at the north tip)
            // The Route-12 Gate building blocks the road; pass through it (north warp → gate → south
            // warp). Disambiguate the two gate→Route12 warps by the south exit's raw landing (10,21),
            // else EnterMap would take the north warp we just came in on and loop.
            Self::enter(Map::Route12Gate1F),
            Self::EnterMap { to_map: Map::Route12, to_position: Some(Point8 { x: 10, y: 21 }) },
            Self::UseFieldItem { item: ItemId::PokeFlute, target: MapSprite::ROUTE12_SNORLAX },
        ]
    }

    /// Soul Badge (Koga, Fuchsia). With the Snorlax cleared, continue **Route 12 south → 13 → 14 → 15 →
    /// Fuchsia City** (all map connections; the Cool-Trainers/Bikers/Beauties on 13–15 engage by line of
    /// sight and are fought normally). Heal at the Fuchsia Center, then enter Koga's gym and beat him —
    /// his team is Poison (Koffing/Muk/Weezing ~lv37–43); a Grass starter resists Poison and leans on
    /// its Normal move + level lead. `DefeatGymLeader` persists through the invisible-wall maze + the six
    /// rocker junior trainers until the badge is won.
    pub fn soul_badge_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::Route13),
            // Cross into Route 14 at the OPEN row-8 landing (19,8): the nearest crossing lands at (19,6),
            // a dead-end pocket sealed by a south-facing Bird Keeper. Route 13 can reach the (0,9) west
            // edge which lands here.
            Self::EnterMap { to_map: Map::Route14, to_position: Some(Point8 { x: 19, y: 8 }) },
            Self::enter(Map::Route15),
            // Route 15 also has a gate building walling off the Fuchsia (west) connection. Enter its
            // east door (nearest), cross, and take the west exit (lands Route 15 (7,8), west of the
            // wall) before the Fuchsia connection is reachable.
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

    /// Safari Zone run for **HM03 Surf** + the **Gold Teeth** (→ HM04 Strength from the Warden). From
    /// Fuchsia: pay at the gate (the "would you like to join?" prompt auto-confirms on A-mash → 500 +
    /// 30 Safari Balls + a 500-step budget), cross Center → West, grab the Gold Teeth, and get Surf from
    /// the Secret House fishing guru. The deterministic policy RUNs from every Safari encounter (never
    /// costs a ball; the BALL/BAIT/ROCK options exist for a future hunting policy).
    pub fn safari_zone_surf_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::FuchsiaCity),       // out of Koga's gym
            Self::enter(Map::SafariZoneGate),
            Self::enter(Map::SafariZoneCenter),  // pays 500 via the join prompt, auto-walks in
            // The Center's West warp is across the central water; the item-bearing West area is reached
            // the long way round: Center → East → North → West (the only land route).
            Self::enter(Map::SafariZoneEast),
            Self::enter(Map::SafariZoneNorth),
            // North→West has two warp pairs: the eastern one (lands (27,0)) drops onto a lower shelf that
            // one-way ledges wall off from the Gold Teeth / Secret House plateau. Take the WESTERN pair
            // (North (3,35) → West (21,0)) which lands on the plateau with both reachable.
            Self::enter_at(Map::SafariZoneWest, 21, 0),
            Self::CollectItem(MapSprite::SAFARIZONEWEST_GOLD_TEETH),
            Self::enter(Map::SafariZoneSecretHouse),
            Self::Interact(MapSprite::SAFARIZONESECRETHOUSE_FISHING_GURU), // hands over HM03 Surf
        ]
    }

    /// After the Surf run (holding the Gold Teeth): leave the Safari Zone and give the Gold Teeth to the
    /// **Warden** (Warden's House, Fuchsia) for **HM04 Strength**. Exiting navigates back to the gate;
    /// if the 500-step timer runs out first the game warps the player to the gate anyway, so either way
    /// the `enter(SafariZoneGate)` step resolves.
    pub fn safari_zone_strength_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::SafariZoneWest),    // out of the secret house
            // Center is split by water: the North entrance lands in a top pocket, walled off from the
            // gate. So retrace the full way in (West → North → East → Center) — East→Center lands at the
            // bottom region where the gate is.
            Self::enter(Map::SafariZoneNorth),
            Self::enter(Map::SafariZoneEast),
            Self::enter(Map::SafariZoneCenter),
            Self::enter(Map::SafariZoneGate),
            Self::enter(Map::FuchsiaCity),
            Self::enter(Map::WardensHouse),
            Self::Interact(MapSprite::WARDENSHOUSE_WARDEN), // give Gold Teeth → HM04 Strength
        ]
    }

    /// Enter Saffron (for Silph Co / the Marsh Badge): trek Fuchsia → Celadon, buy a Fresh Water from
    /// the Celadon Mart roof vending machine, then pass the Route-7 gate guard (who takes the drink and
    /// opens all four Saffron gates). Reverse of the soul-badge trek back to Lavender, then the Route
    /// 7–8 underground path to Celadon.
    pub fn saffron_entry_steps() -> Vec<Self> {
        let mut s = vec![
            Self::enter(Map::FuchsiaCity), // out of the Warden's house
            // Fuchsia → Lavender (reverse of the soul-badge routes; Snorlax already cleared).
            Self::enter(Map::Route15),      // from Fuchsia: lands on the west side of the Route-15 gate
            // Reverse the Route-15 gate: west door → east exit (lands Route 15 (14,8), east of the wall).
            Self::enter(Map::Route15Gate1F),
            Self::EnterMap { to_map: Map::Route15, to_position: Some(Point8 { x: 14, y: 8 }) },
            Self::enter(Map::Route14),
            Self::enter(Map::Route13),
            Self::enter(Map::Route12),      // from Route 13: lands south of the Route-12 gate
            // Reverse the Route-12 gate: south door → north exit (lands Route 12 (10,15), north of it).
            Self::enter(Map::Route12Gate1F),
            Self::EnterMap { to_map: Map::Route12, to_position: Some(Point8 { x: 10, y: 15 }) },
            Self::enter(Map::LavenderTown),
            // The nearest Lavender→Route8 crossing (0,11) jams; take the (0,9) one (lands Route8 (59,8)).
            Self::EnterMap { to_map: Map::Route8, to_position: Some(Point8 { x: 59, y: 8 }) },
        ];
        // Lavender → Celadon via the Route 7–8 underground path (existing helper: heals at Celadon too).
        s.extend(Self::lavender_to_celadon_steps());
        // Into the Mart, up to the roof, buy a Fresh Water from the vending machine.
        s.extend([
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonMart2F),
            Self::enter(Map::CeladonMart3F),
            Self::enter(Map::CeladonMart4F),
            Self::enter(Map::CeladonMart5F),
            Self::enter(Map::CeladonMartRoof),
            Self::UseVendingMachine { at: Point8 { x: 10, y: 1 }, drink: ItemId::FreshWater },
            // Back down and out to Celadon, then east through the Route-7 gate into Saffron.
            Self::enter(Map::CeladonMart5F),
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonCity),
            Self::enter(Map::Route7),
            Self::enter(Map::Route7Gate),        // west door
            // Walk east through the gate to the east door (Route 7 (18,10), Saffron side). Crossing the
            // guard-trigger tile (3,4) hands over the Fresh Water (we have it → no push-back).
            Self::EnterMap { to_map: Map::Route7, to_position: Some(Point8 { x: 18, y: 10 }) },
            Self::enter(Map::SaffronCity),
        ]);
        s
    }

    /// Silph Co, part 1: from Saffron, enter Silph Co and ride the elevator to **5F** for the **Card
    /// Key** (which opens the locked doors throughout the building). The elevator works like the Rocket
    /// Hideout's (panel bg-event at (3,0), 11-floor menu: 1F=0 … 5F=4 … 11F=10, redirected exit warp).
    pub fn silph_co_card_key_steps() -> Vec<Self> {
        vec![
            // Stock up on HYPER Potions at the Saffron Mart before entering Silph. The Silph rival /
            // Giovanni / Sabrina and Blaine fights need a heal big enough to lift the healer back above
            // the enemy's per-turn damage — a Super Potion's +50 just cancels Alakazam's Psychic (an
            // unwinnable stalemate), whereas a Hyper Potion (+200) restores to near-full so the mon
            // survives above the heal threshold and actually fights back. The heal prefers the biggest
            // potion in the bag. (Done here, not in saffron_entry, so that leg's exit position — which
            // the Eevee leg's Route-7 crossing depends on — is unchanged.)
            Self::enter(Map::SaffronMart),
            Self::BuyFromMart { item: BagItem::new(ItemId::HyperPotion, 15), map: Map::SaffronMart },
            Self::enter(Map::SaffronCity),
            Self::enter(Map::SilphCo1F),
            Self::enter(Map::SilphCoElevator),                          // step onto the (20,0) $58 warp tile
            Self::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 4 }, // 5F = menu index 4
            // The Card Key sits in a walled 5F pocket (row 16) reachable only by *arriving* on the
            // 5F (9,15) teleport pad and stepping down. (9,15)↔9F(17,15) are a pair: walk to the
            // reachable (9,15) pad → 9F(17,15); step back onto (17,15) → arrive standing on 5F(9,15),
            // now adjacent to the pocket. Then collect.
            Self::enter(Map::SilphCo9F),                                // via 5F (9,15) → 9F (17,15)
            Self::enter(Map::SilphCo5F),                                // via 9F (17,15) → arrive at 5F (9,15)
            Self::CollectItem(MapSprite::SILPHCO5F_CARD_KEY),
        ]
    }

    /// From the 5F Card-Key pocket: thread the teleport-pad maze up to **Giovanni** on 11F, beat him
    /// (his after-battle script liberates Saffron), talk to the freed **Silph President** for the Master
    /// Ball, then thread the pads back down and out to Saffron and heal. Pad chain:
    ///   5F(9,15)→9F(17,15); 9F→elevator→3F; 3F(11,11)→7F(5,3) rival pocket; 7F(5,7)→11F(3,2).
    /// Giovanni's scripted battle only fires by STANDING on his trigger (walking to his front passes it),
    /// so we clear the blocking Rocket then queue many single-shot Interacts until one lands in position.
    pub fn silph_giovanni_steps() -> Vec<Self> {
        use crate::pokemon::map::MapSprite as MS;
        let mut s = vec![
            // Lead with the bulky Venusaur (already slot 0): with Hyper Potions it out-heals the rival's
            // Alakazam (Psychic) and Pidgeot and mows the Ground/Rock/Grass-weak mons, while the fresh
            // Vaporeon stays in RESERVE — it comes in when Venusaur finally falls to the Fire ace
            // (Charizard) and one-shots it with Surf (4×). Leading the frail Vaporeon instead gets it
            // KO'd early and wastes its one job.
            Self::enter(Map::SilphCo9F),                                 // 5F(9,15) pad → 9F(17,15)
            Self::enter(Map::SilphCoElevator),
            Self::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 2 }, // 3F = menu index 2
            Self::EnterMap { to_map: Map::SilphCo7F, to_position: Some(Point8 { x: 5, y: 3 }) },   // 3F(11,11) pad
            // Fight the 7F rival EXPLICITLY (walk into his front, battle, end standing there) — routing
            // straight for the 11F pad instead trips his line-of-sight mid-walk at a stray tile and the
            // subsequent 11F warp resolves off a desynced position. This mirrors the proven route.
            Self::Interact(MS::SILPHCO7F_RIVAL),
            Self::EnterMap { to_map: Map::SilphCo11F, to_position: Some(Point8 { x: 3, y: 2 }) },  // 7F(5,7) pad
            Self::InteractIfReachable(MS::SILPHCO11F_ROCKET1),
        ];
        // Use InteractIfReachable (not Interact): reachable → walk in (crossing his (6,13) trigger fires
        // the scripted battle whose after-script liberates Saffron); once he's beaten and unreachable it
        // pops after a bounded wait instead of hanging forever (plain Interact never gives up).
        for _ in 0..14 { s.push(Self::InteractIfReachable(MS::SILPHCO11F_GIOVANNI)); }
        s.extend([
            // ⚠️ **Free a bag slot before the President reaches into his pocket.** A gift is not a
            // purchase: there is no "you gave up" to detect, the ROM just prints "You have no room for
            // this." inside his thank-you speech and the `Interact` completes looking exactly like a
            // success. The Master Ball then never arrives, and the failure surfaces 100 steps later in
            // the Seafoam Islands, where `CatchPokemon { ball: MasterBall }` silently falls back to the
            // best ball in the bag, throws a Great Ball at a lv50 Articuno and loses the party.
            // TM34 Bide is the deadest weight the run carries — picked up in Cerulean, never taught.
            Self::TossItem { item: ItemId::Tm34Bide },
            Self::Interact(MS::SILPHCO11F_SILPH_PRESIDENT),             // Master Ball + Rockets leave Saffron
            Self::EnterMap { to_map: Map::SilphCo7F, to_position: Some(Point8 { x: 5, y: 7 }) },   // 11F(3,2) pad
            Self::EnterMap { to_map: Map::SilphCo3F, to_position: Some(Point8 { x: 11, y: 11 }) }, // 7F(5,3) pad
            Self::enter(Map::SilphCoElevator),
            Self::UseElevator { panel: Point8 { x: 3, y: 0 }, floor: 0 }, // 1F
            Self::enter(Map::SaffronCity),
            Self::enter(Map::SaffronPokecenter),
            Self::Interact(MS::SAFFRONPOKECENTER_NURSE),
            Self::enter(Map::SaffronCity),
        ]);
        s
    }

    /// Marsh Badge: Saffron Gym is a teleport-pad maze; `DefeatGymLeader` routes through the intra-map
    /// teleporters and beats each gym trainer via line of sight to reach **Sabrina**. Requires Saffron to
    /// have been liberated (see [`silph_giovanni_steps`]) so the gym door isn't Rocket-blocked.
    pub fn marsh_badge_steps() -> Vec<Self> {
        use crate::pokemon::map::MapSprite as MS;
        vec![
            Self::enter(Map::SaffronGym),
            Self::DefeatGymLeader { leader: MS::SAFFRONGYM_SABRINA, badge: Badge::MarshBadge },
        ]
    }

    /// From Saffron: fetch the free **Eevee** (Celadon Mansion roof house), buy a **Water Stone** (Celadon
    /// Dept Store 4F), evolve Eevee → **Vaporeon** and teach it **Surf** — the lone Grass starter can't
    /// learn Surf, and Surf is needed to reach Cinnabar. Ends back in Celadon.
    pub fn eevee_vaporeon_surf_steps() -> Vec<Self> {
        use crate::pokemon::map::MapSprite as MS;
        vec![
            // Saffron → Celadon via the Route-7 gate (east crossing at (19,10); the plain connection lands
            // in a ledge-sealed pocket).
            Self::EnterMap { to_map: Map::Route7, to_position: Some(Point8 { x: 19, y: 10 }) },
            Self::enter(Map::Route7Gate),
            Self::EnterMap { to_map: Map::Route7, to_position: Some(Point8 { x: 11, y: 10 }) },
            Self::enter(Map::CeladonCity),
            // Free Eevee from the Celadon Mansion roof house (BACK entrance (24,3)→1F(4,0); the front door
            // is the dead-end condos). Climb the stairwell to the roof.
            Self::EnterMap { to_map: Map::CeladonMansion1F, to_position: Some(Point8 { x: 4, y: 0 }) },
            Self::enter(Map::CeladonMansion2F),
            Self::enter(Map::CeladonMansion3F),
            Self::enter(Map::CeladonMansionRoof),
            Self::enter(Map::CeladonMansionRoofHouse),
            Self::CollectItem(MS::CELADONMANSION_ROOF_HOUSE_EEVEE_POKEBALL),
            Self::enter(Map::CeladonMansionRoof),
            Self::enter(Map::CeladonMansion3F),
            Self::enter(Map::CeladonMansion2F),
            Self::EnterMap { to_map: Map::CeladonMansion1F, to_position: Some(Point8 { x: 4, y: 0 }) },
            Self::enter(Map::CeladonCity),
            // Dept Store 4F: buy a Water Stone.
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonMart2F),
            Self::enter(Map::CeladonMart3F),
            Self::enter(Map::CeladonMart4F),
            Self::BuyFromMart { item: BagItem::new(ItemId::WaterStone, 1), map: Map::CeladonMart4F },
            Self::enter(Map::CeladonMart1F),
            Self::enter(Map::CeladonCity),
            // Evolve the Eevee → Vaporeon and teach it Surf. Vaporeon is the answer to the Silph
            // rival's Alakazam (Surf ignores its huge Special wall better than Venusaur's resisted Razor
            // Leaf) and to Blaine's Fire team, plus it carries Surf for the Cinnabar crossing.
            // Both name the mon by **species**: where the gift Eevee lands depends on how many members
            // the party already has, and the fixture chain's parties are not the mainline's.
            Self::EvolveWithStone { stone: ItemId::WaterStone, target: PartyRef::Species(PokemonSpecies::Eevee) },
            Self::TeachMove { item: ItemId::Hm03Surf, target: PartyRef::Species(PokemonSpecies::Vaporeon) },
            // Back to Saffron for Silph Co (Celadon → Route 7 → Saffron).
            Self::EnterMap { to_map: Map::Route7, to_position: Some(Point8 { x: 11, y: 10 }) },
            Self::enter(Map::Route7Gate),
            Self::EnterMap { to_map: Map::Route7, to_position: Some(Point8 { x: 19, y: 10 }) },
            Self::enter(Map::SaffronCity),
        ]
    }

    /// Saffron → Cinnabar Island (needs Surf on Vaporeon + Cut on Venusaur). Route 6 → Vermilion →
    /// Diglett's Cave → Route 2 (Cut two trees) → Viridian → Route 1 → Pallet → **Surf across Route 21**
    /// to Cinnabar.
    pub fn saffron_to_cinnabar_steps() -> Vec<Self> {
        vec![
            // Venusaur leads here (slot 0), which is what the Cut field-move executor needs — it always
            // uses the lead and only Venusaur knows Cut. Surf still works from any lead (its executor
            // picks the surfer's slot dynamically), so Vaporeon can still ferry us across Route 21.
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

    /// Pokémon Mansion → Secret Key (unlocks the Cinnabar Gym). The mansion is a switch-gate maze: one
    /// global switch (`EVENT_MANSION_SWITCH_ON`) toggles every floor's sliding doors, and the only way
    /// to the B1F Secret Key is to *fall through a 3F hole* to 1F's right side (the hole warp-down is
    /// modelled in `apply_mansion_holes`). Route (all switch flips are single toggles):
    ///   1F → 2F → 3F(via the 6,1 north stairs) → flip 3F switch (opens the hole region) → fall to
    ///   1F(16,14) → B1F → flip (18,25) (opens the top) → flip (20,3) (opens the left column) → Key.
    /// Frees a bag slot with the Rare Candy first (the bag arrives full at 20/20, blocking the pickup).
    pub fn mansion_secret_key_steps() -> Vec<Self> {
        vec![
            // Heal to full HP/PP first — the mansion is a long battle-heavy crossing with no Pokémon
            // Center inside, so the party must enter with full move PP (else it Struggles itself out).
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
            // NB: TM14 **Blizzard** sits at (19,25), right beside this switch, and taking it here is
            // tempting — it is the Elite Four's Lance answer once it is on Articuno. It is deliberately
            // NOT collected: adding it shifted the RNG line onto the losing side of the Route-22 rival
            // fight, which is a coin flip this run cannot afford (see `victory_road_1f_steps` — its
            // Hyper Potion restock is a no-op because the Viridian Mart does not stock them, so that
            // fight is fought on leftovers and stalemates on PP). `probe_get_blizzard` takes the TM for
            // the Elite Four fixture chain instead, where `seafoam_articuno_steps` teaches it.
            Self::FlipSwitch { map: Map::PokemonMansionB1F, at: Point8 { x: 20, y: 3 }, reveals: Map::PokemonMansion1F },
            Self::CollectItem(MapSprite::POKEMONMANSIONB1F_SECRET_KEY),
        ]
    }

    /// Volcano Badge: from the B1F Secret-Key pocket, exit the mansion, heal, and clear the Cinnabar
    /// Gym. Exiting reverses the two B1F switch flips (reopening the (23,22) staircase up), then out to
    /// Cinnabar. The gym is a quiz-gate snake maze: `DefeatGymLeader` beats each fire trainer via its
    /// line of sight (all face down → engage from below), which unlocks the gate ahead, then Blaine.
    /// Blaine's Fire team folds to Vaporeon's Surf / Venusaur's bulk.
    pub fn volcano_badge_steps() -> Vec<Self> {
        vec![
            Self::FlipSwitch { map: Map::PokemonMansionB1F, at: Point8 { x: 20, y: 3 }, reveals: Map::PokemonMansion1F },
            Self::FlipSwitch { map: Map::PokemonMansionB1F, at: Point8 { x: 18, y: 25 }, reveals: Map::PokemonMansion1F },
            Self::enter(Map::PokemonMansion1F),   // (23,22) staircase up → 1F right side
            Self::enter(Map::CinnabarIsland),
            Self::enter(Map::CinnabarPokecenter), // heal before the gym gauntlet
            Self::Interact(MapSprite::CINNABARPOKECENTER_NURSE),
            Self::enter(Map::CinnabarIsland),
            // Lead with Vaporeon for Blaine — his all-Fire team takes 2× from Surf, and Venusaur (Grass)
            // is 2× weak to Fire. Party is [Venusaur, Vaporeon] here, so slot 1 → front puts Vaporeon up.
            Self::MovePokemonToFront { target: PartyRef::Species(PokemonSpecies::Venusaur) },
            Self::enter(Map::CinnabarGym),
            Self::DefeatGymLeader { leader: MapSprite::CINNABARGYM_BLAINE, badge: Badge::VolcanoBadge },
        ]
    }

    /// **Articuno** (Seafoam Islands B4F) — the Ice sweeper the Elite Four's Lance needs. A
    /// there-and-back detour off Cinnabar Island: Surf east onto Route 20, dive into the Seafoam
    /// **east** entrance, solve both boulder puzzles on the way down, and throw the **Master Ball**
    /// (guaranteed catch) at the static lv50 bird on B4F.
    ///
    /// ## The one boulder pair that matters
    ///
    /// Every gate in these caves reads `CheckBothEventsSet`, whose `and mask` / `cp mask` sets Z when
    /// **both** flags are set — so each guard's `jr z` / `ret z` means *the obstacle is gone once the
    /// pair is down*, not before. (Reading it the other way round is what once made a boulder-free
    /// route look possible; the emulator disproved it, `mode Script` and all.)
    ///
    /// **SEAFOAM4** (`EVENT_SEAFOAM4_BOULDER{1,2}_DOWN_HOLE`, set by pushing two of B3F's own boulders
    /// into its floor holes at (3,16)/(6,16)) is what lets the player onto B4F's west lake, the only
    /// approach to Articuno at (6,1). Both ways onto that lake are gated on it:
    ///  * the lake's single shore tile (7,11), where `IsSurfingAllowed`
    ///    (`engine/overworld/field_move_messages.asm`) refuses to mount Surf with "The current is much
    ///    too fast!"; and
    ///  * falling through a B3F floor hole, which lands the player at B4F (4,14)/(5,14) already surfing
    ///    (`DungeonWarpData` + `ForcedBikeOrSurfMaps`) — but `SeafoamIslandsB4FMoveObjectScript` then
    ///    force-walks them UP/RIGHT/UP straight back off the lake onto the central land at (7,10).
    ///
    /// B3F is the only floor below 1F whose boulders can be pushed on arrival. The game intends a chain
    /// — 1F's pair into 1F's holes reveals B1F's pair, which reveals B2F's, which reveals B3F's last two
    /// — and `data/maps/toggleable_objects.asm` enforces it: every boulder on B1F and B2F starts
    /// HIDDEN. B3F is the exception. Its (5,14) and (9,14) are not toggleable objects at all
    /// (permanently visible) and its (3,15)/(8,14) start SHOWn, so the floor offers four pushable
    /// boulders with no prerequisites — which is exactly the pair of drops SEAFOAM4 needs, and why
    /// the chain above can be skipped.
    ///
    /// ## Getting out again — an Escape Rope, because there is no way back east on foot
    ///
    /// **SEAFOAM3** (B2F's boulders, hidden behind that same 1F→B1F→B2F chain) is what would reopen the
    /// eastward return, and without it every candidate is sealed:
    ///  * B4F's (20,17)/(21,17) staircases: `SeafoamIslandsB4FDefaultScript` force-walks the player
    ///    north off them, `res BIT_FORCED_WARP` cancelling the warp, until SEAFOAM3 is set;
    ///  * B2F's floor hole at (22,6) (reached from B4F via the (25,4)/(25,3) pockets, and free of any
    ///    boulder requirement) drops the player into B3F's east region at (19,7) — but landing there
    ///    runs `SeafoamIslandsB3FMoveObjectScript`, whose x=18/19 strong-current RLE sweeps them right
    ///    back down to B4F. Verified the hard way: the agent looped B2F → B3F → B4F → B2F;
    ///  * B3F's (15,8) current tile is one-way west→east into the same sweep;
    ///  * and the west exit (up B3F (5,12) → B2F → B1F → 1F, out of Route 20's **west** entrance) lands
    ///    on the far side of Route 20's x=63 wall — Fuchsia, not Cinnabar, with no way back short of
    ///    walking half of Kanto.
    ///
    /// So the leg leaves the way any Gen-1 player would: an **Escape Rope**. Seafoam is Cavern, which is
    /// in `EscapeRopeTilesets`, and the rope warps to `wLastBlackoutMap` — set by the last Pokémon
    /// Center used, which is the Cinnabar heal at the top of this list. One item, one step, and the six
    /// boulder pushes across three floors that SEAFOAM3 would have cost are never needed.
    ///
    /// ## Getting to the boulders
    ///
    /// No floor is one connected space: the Cavern tileset has **elevation tile-pair collisions** —
    /// $05↔$20/$21/$2A/$41 on land, $14↔$05 on water (so inside Seafoam you can only get on or off the
    /// water at a shore tile) — which cut columns that look wide open in the block map. Every floor is a
    /// set of walled pockets joined only by warps, so each hop is an explicit `enter_at`:
    ///
    /// | leg | route |
    /// |---|---|
    /// | in | Route 20 → 1F (26,17) → B1F (23,15) → B2F (25,11) → B3F (25,14) |
    /// | | → B4F (20,17) → B3F (8,6): the only crossing to B3F's **west** half |
    /// | SEAFOAM4 | drop two of B3F's boulders into (3,16) and (6,16) — kills the B4F current |
    /// | catch | hole (6,16) → B4F (5,14), afloat on the west lake; Master Ball on the bird |
    /// | out | Escape Rope → Cinnabar Island |
    ///
    /// The B4F round trip on the way in is not a detour, it is the only way across: B3F's east and west
    /// halves are joined solely by the (15,8) strong-current tile (modelled impassable — see
    /// `apply_seafoam_currents`), so the west half, and hence its holes, can only be entered from B4F.
    /// Two other approaches are dead ends: Route 20's west Seafoam entrance (48,5) sits behind one-way
    /// ledges (Jump West/South) so it can only be walked *out of*, never surfed into; and on B2F the
    /// east column stops at row 10 on an elevation boundary, putting (25,3) out of reach from (25,11).
    ///
    /// ## The Strength slave
    ///
    /// Neither Venusaur nor Vaporeon learns HM04, so the pushes need a slave. **Slowpoke** does learn
    /// it and is one of the two commonest land encounters on Seafoam 1F (which has the highest
    /// encounter rate of the five floors, and whose entry pocket is all land), so it is caught on the
    /// way in — with **Great Balls**, explicitly, because `Bag::best_pokeball` would otherwise spend the
    /// Master Ball earmarked for Articuno on it. It lands at party slot 2 and Strength is re-armed on
    /// every floor, since `BIT_STRENGTH_ACTIVE` resets on each map change.
    ///
    /// The floors are decoded offline from the ROM block maps by `probe_seafoam_actions_offline`,
    /// `probe_seafoam_connectivity_offline` (which floods the whole dungeon through its warps) and
    /// `probe_seafoam_boulder_and_exit_offline` (which runs the Sokoban planner on both pairs) — all
    /// ROM-only and instant, so the whole chain below is checked before paying for an emulator run.
    pub fn seafoam_articuno_steps() -> Vec<Self> {
        // Strength is armed per floor: `BIT_STRENGTH_ACTIVE` is cleared on every map change, and the
        // route leaves and re-enters each boulder floor.
        // The HM slave is named by species, not by slot: it is caught *inside* this step list, so any
        // slot index would be an assumption about how big the party was on arrival.
        const SLAVE: PartyRef = PartyRef::Species(PokemonSpecies::Slowpoke);
        vec![
            // Heal and stock balls first: Route 20's swimmer gauntlet is fought on the way over, and
            // there is no Pokémon Center inside Seafoam (its wilds are fled — `in_center_less_dungeon`).
            Self::enter(Map::CinnabarIsland),
            Self::enter(Map::CinnabarPokecenter),
            Self::Interact(MapSprite::CINNABARPOKECENTER_NURSE),
            Self::enter(Map::CinnabarIsland),
            // The bag is at Gen 1's 20-item cap by now, and a full bag makes the purchase below fail
            // silently — the clerk refuses and `BuyFromMart` just gives up, which then spends the
            // Master Ball on the HM-slave and leaves nothing for Articuno. The Nugget is pure
            // sell-fodder this run never sells, and it is not a key item, so it is the slot to free.
            Self::TossItem { item: ItemId::Nugget },
            Self::BuyFromMart { item: BagItem::new(ItemId::GreatBall, 10), map: Map::CinnabarMart },
            // Top the Hyper Potions back up while at the last mart on the route that sells them. The
            // Route-22 rival, three legs from here, is the fight this run is thinnest for, and it used
            // to be fought on whatever was left over from Saffron. No bag slot needed: the run is
            // already carrying a Hyper Potion stack, so this only deepens it.
            Self::BuyFromMart { item: BagItem::new(ItemId::HyperPotion, 20), map: Map::CinnabarMart },
            Self::enter(Map::CinnabarIsland),
            // Surf east across Route 20 to the east Seafoam entrance.
            Self::enter(Map::Route20),
            Self::enter_at(Map::SeafoamIslands1F, 26, 17),
            // The HM slave, before anything needs pushing.
            Self::CatchPokemon { species: PokemonSpecies::Slowpoke, on_map: Map::SeafoamIslands1F,
                                 ball: Some(ItemId::GreatBall) },
            Self::TeachMove { item: ItemId::Hm04Strength, target: SLAVE },
            // TM28 is dead weight in the bag and the way home in the slave's move list.
            Self::TeachMove { item: ItemId::Tm28Dig, target: SLAVE },
            // Down the east side, one walled pocket at a time, then across to B3F's west half.
            Self::enter_at(Map::SeafoamIslandsB1F, 23, 15),
            Self::enter_at(Map::SeafoamIslandsB2F, 25, 11),
            Self::enter_at(Map::SeafoamIslandsB3F, 25, 14),
            Self::enter_at(Map::SeafoamIslandsB4F, 20, 17),
            Self::enter_at(Map::SeafoamIslandsB3F, 8, 6),
            // ── SEAFOAM4: two of B3F's four boulders into its two holes. The planner moves (5,14) out
            // of the corridor first — it is the only tile from which (3,15) can be reached at all.
            Self::UseStrength { target: SLAVE },
            Self::DropBoulderInHole { hole: Point8 { x: 3, y: 16 } },
            Self::DropBoulderInHole { hole: Point8 { x: 6, y: 16 } },
            // Fall through the (6,16) hole into the west lake, already surfing, and Master-Ball the bird.
            Self::enter_at(Map::SeafoamIslandsB4F, 5, 14),
            Self::CatchPokemon { species: PokemonSpecies::Articuno, on_map: Map::SeafoamIslandsB4F,
                                 ball: Some(ItemId::MasterBall) },
            // The bird arrives with Peck and Ice Beam — 10 PP of Ice for five Elite Four rooms. TM14
            // (taken in the Mansion) adds Blizzard: same STAB, 120 power, and five more Ice attacks.
            // Skipped harmlessly if this run never collected the TM.
            Self::TeachMove { item: ItemId::Tm14Blizzard, target: PartyRef::Species(PokemonSpecies::Articuno) },
            // Out with DIG: there is no walkable way back east (see the doc above), and it lands on
            // Cinnabar Island because that is where the Pokémon Center at the top of this list set
            // `wLastBlackoutMap`.
            Self::Dig { slot: 2 }, // the Slowpoke: caught after Vaporeon + Venusaur, so slot 2 (`Dig` is still slot-addressed)
            Self::enter(Map::CinnabarIsland),
        ]
    }

    /// Earth Badge (8th): Giovanni's **Viridian Gym**, which reopens once Team Rocket is beaten at Silph
    /// Co (done). From Cinnabar, Surf back across Route 21 to Pallet and up to Viridian, heal at the
    /// Center, then clear the gym's **spinner-tile maze** (see the `ViridianGym` arrow table in
    /// `tile_map.rs`) to Giovanni. His Ground/Rock team is 2–4× weak to Vaporeon's Surf, which already
    /// leads coming out of Blaine, so the healed party clears it without a Hyper-Potion restock.
    pub fn earth_badge_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::CinnabarIsland),   // out of Blaine's gym
            // Cinnabar → Viridian: Surf across Route 21 to Pallet, then up Route 1.
            Self::enter(Map::Route21),
            Self::enter(Map::PalletTown),
            Self::enter(Map::Route1),
            Self::enter(Map::ViridianCity),
            // Heal before the toughest gym.
            Self::enter(Map::ViridianPokecenter),
            Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
            Self::enter(Map::ViridianCity),
            Self::enter(Map::ViridianGym),
            Self::DefeatGymLeader { leader: MapSprite::VIRIDIANGYM_GIOVANNI, badge: Badge::EarthBadge },
        ]
    }

    /// After all 8 badges: reach Victory Road 1F, catch a Machop HM-slave + teach it Strength, then
    /// solve the 1F boulder puzzle (push a boulder onto the (17,13) switch) and climb the now-open (1,1)
    /// ladder to VR2F. Reliable from a fresh run; folded into `complete_game_steps`. The deeper VR2F/VR3F
    /// puzzle is `victory_road_2f_3f_steps` (PP-marginal from a fresh run — see its note).
    pub fn victory_road_1f_steps() -> Vec<Self> {
        vec![
            Self::enter(Map::ViridianCity),          // out of the gym
            // The Route-22 rival is a Silph-rival redux (Alakazam + Charizard). Beat it like Silph: heal
            // (full HP/PP), restock Hyper Potions (a Super Potion's +50 only cancels Alakazam's Psychic →
            // unwinnable stalemate), and lead the bulky Venusaur. Cave wilds during the catch are FLED, not
            // fought (see the wild-flee block in `pick_battle_action`), so PP holds through the boulder solves.
            Self::enter(Map::ViridianPokecenter),
            Self::Interact(MapSprite::VIRIDIANPOKECENTER_NURSE),
            Self::enter(Map::ViridianCity),
            // (No mart stop here: the **Viridian Mart does not sell Hyper Potions** — Poké Ball,
            // Antidote, Parlyz Heal, Burn Heal only, per `data/items/marts.asm` — so the restock that
            // used to sit here walked in, failed four times and walked out. The stack is bought on
            // Cinnabar instead, in `seafoam_articuno_steps`, at the last mart on the route that
            // stocks them.)
            // ⚠️ **By species.** This used to be `slot: 1`, written when the party was Venusaur +
            // Vaporeon and slot 1 *was* Venusaur. By Victory Road the run also carries the Seafoam
            // Slowpoke and Articuno, and their arrival order is not fixed — so the "lead the bulky
            // Venusaur" comment could put the lv30 HM-slave in front instead, which is how a party
            // that should beat VR1F's nine trainers ended up blacked out on the way to the ladder.
            Self::MovePokemonToFront { target: PartyRef::Species(PokemonSpecies::Venusaur) },
            Self::enter(Map::Route22),
            Self::enter(Map::Route22Gate),           // walk west → rival ambush → gate to Route 23
            Self::Interact(MapSprite::ROUTE22GATE_GUARD), // walk to (5,2): badge check + flips the dynamic warp
            Self::enter(Map::Route23),
            Self::goto(Map::VictoryRoad1F),
            // Catch a wild Machop (learns Strength) as the boulder HM-slave — Master Ball, thrown at once.
            Self::CatchPokemon { species: PokemonSpecies::Machop, on_map: Map::VictoryRoad1F, ball: None },
            Self::TeachMove { item: ItemId::Hm04Strength, target: Self::MACHOP }, // caught just above
            // VR1F: push a boulder onto (17,13), climb to VR2F.
            Self::UseStrength { target: Self::MACHOP },
            Self::SolveBoulders { switch: Point8 { x: 17, y: 13 } },
            // ⚠️ **`goto`, not `enter`, because a black-out on this last walk is otherwise terminal.**
            // VR1F has ~9 mandatory trainers and no Pokémon Center, and the party can lose the last of
            // them on the way to the ladder. When it does, the respawn is in Viridian — and worse, the
            // *agent reads it as success*: a warp is "done" when the map changes, and a black-out
            // changes the map, so the run logs `OverworldActionCompleted { Warp → VictoryRoad2F }`
            // while standing in Viridian City. A single-hop `enter` then re-issues forever, because it
            // cannot cross four maps. Strict `Goto` re-runs `route_toward` every tick until the player
            // is *actually* on VR2F, so it walks back (Route 22 → 23 → VR1F → ladder) instead — and the
            // second climb is cheap, because beaten trainers stay beaten and the boulder stays on the
            // switch. Putting the recovery *in front of* the fragile step does not work; it has to
            // **be** the fragile step.
            Self::goto(Map::VictoryRoad2F),
        ]
    }

    /// The VR2F/VR3F half of Victory Road (from standing on VR2F to the Indigo Plateau lobby): the
    /// interconnected hole-drop puzzle. Validated end-to-end by `can_solve_victory_road_2f_3f` (from a
    /// VR3F fixture). NB: chaining this onto a *fresh* run is PP-marginal — VR's ~9 mandatory trainers
    /// plus the Route-22 rival drain Venusaur past its ~50 damaging PP in some RNG lines (there is no
    /// Pokémon Center inside VR), so it is NOT yet in `complete_game_steps`; that needs a stronger team.
    pub fn victory_road_2f_3f_steps() -> Vec<Self> {
        vec![
            // VR2F: switch1 (1,16) → up the (23,7) stairs to VR3F.
            Self::UseStrength { target: Self::MACHOP },
            Self::SolveBoulders { switch: Point8 { x: 1, y: 16 } },
            Self::enter(Map::VictoryRoad3F),
            // VR3F: switch (3,5) opens the hole barrier; drop a boulder into the hole (23,15) to reveal 2F's
            // hidden boulder, then fall through the hole to VR2F's east side.
            Self::UseStrength { target: Self::MACHOP },
            Self::SolveBoulders { switch: Point8 { x: 3, y: 5 } },
            Self::DropBoulderInHole { hole: Point8 { x: 23, y: 15 } },
            Self::enter_at(Map::VictoryRoad2F, 22, 16),
            // VR2F east: push the revealed boulder onto switch2 (9,16); this leaves the player in the west.
            Self::UseStrength { target: Self::MACHOP },
            Self::SolveBoulders { switch: Point8 { x: 9, y: 16 } },
            // Return trip: climb back to VR3F and fall through the hole again → back east with switch2 open.
            Self::enter(Map::VictoryRoad3F),
            Self::enter_at(Map::VictoryRoad2F, 22, 16),
            // Out the (29,7/8) exit → Route 23 → Indigo Plateau → the Elite Four lobby.
            Self::enter(Map::Route23),
            Self::enter(Map::IndigoPlateau),
            Self::enter(Map::IndigoPlateauLobby),
        ]
    }

    /// The Elite Four gauntlet, from the Indigo Plateau lobby to the Champion: stock up, heal, then
    /// Lorelei → Bruno → Agatha → Lance → the rival. Validated by `can_beat_elite_four`.
    ///
    /// The two leads are passed in as slots rather than hardcoded because `MovePokemonToFront` rotates
    /// the party — where Articuno sits after Venusaur is pulled to the front depends on where both
    /// started, and a grinded fixture arrives in a different order from an ungrinded one. Callers should
    /// look both up **by species** (see `can_beat_elite_four`), not assume a layout.
    ///
    /// Not folded into `complete_game_steps`: that run stops on Victory Road 2F, because chaining
    /// `victory_road_2f_3f_steps` onto a fresh party is PP-marginal (see its note), so the plateau is
    /// out of reach from there.
    ///
    /// `ice_lead` takes over before Lance. The battle policy only switches when the active mon has *no*
    /// damaging move left, so without the swap Vaporeon stays in and chips away with Bite once
    /// Blizzard's 5 PP is gone — which is exactly how the first Articuno attempt ran Lance's room out of
    /// the clock. Ice Beam is 4× on Dragonair/Dragonite, 2× on Gyarados and Aerodactyl, and 2× again on
    /// the Champion's Pidgeot/Exeggutor/Rhydon/Gyarados.
    pub fn elite_four_steps(lead: u8, ice_lead: u8) -> Vec<Self> {
        vec![
            // ¥3000 and ¥1500 each — 12 + 4 is ¥42,000, inside what the Articuno team arrives with
            // (~¥50k). `BuyFromMart` gives up rather than buying fewer, so asking for more than the
            // wallet holds silently leaves the gauntlet with nothing.
            Self::BuyFromMart { item: BagItem::new(ItemId::FullRestore, 12), map: Map::IndigoPlateauLobby },
            Self::BuyFromMart { item: BagItem::new(ItemId::Revive, 4), map: Map::IndigoPlateauLobby },
            Self::Interact(MapSprite::INDIGOPLATEAULOBBY_NURSE),   // revive + restore all PP
            // Venusaur leads: Razor Leaf is 2× on all of Lorelei's Water types and 4× on Bruno's Onix.
            Self::MovePokemonToFront { target: PartyRef::Slot(lead) },
            Self::enter(Map::LoreleisRoom),
            Self::BattleTrainer { trainer: MapSprite::LORELEISROOM_LORELEI },
            Self::enter(Map::BrunosRoom),
            Self::BattleTrainer { trainer: MapSprite::BRUNOSROOM_BRUNO },
            Self::enter(Map::AgathasRoom),
            Self::BattleTrainer { trainer: MapSprite::AGATHASROOM_AGATHA },
            Self::MovePokemonToFront { target: PartyRef::Slot(ice_lead) },
            Self::enter(Map::LancesRoom),
            Self::BattleTrainer { trainer: MapSprite::LANCESROOM_LANCE },
            Self::enter(Map::ChampionsRoom),
            Self::BattleTrainer { trainer: MapSprite::CHAMPIONSROOM_RIVAL },
        ]
    }


    /// The full deterministic playthrough. Every forward map transition is an explicit `EnterMap`;
    /// on-map tasks (`Interact`/`Buy`/`Grind`/`Catch`) self-route over the incrementally-observed
    /// graph. Starter is **Bulbasaur** — its Grass typing is super-effective against both Brock
    /// (Rock/Ground) and Misty (Water), the two badges this run proves.
    pub fn complete_game_steps() -> Vec<Self> {
        let mut steps = vec![
            // ── Pallet Town: fetch a starter ──
            Self::enter(Map::RedsHouse1F),
            Self::enter(Map::PalletTown),
            Self::soft_goto(Map::Route1),                        // Oak stops you → OaksLab
            Self::Interact(MapSprite::OAKSLAB_BULBASAUR_POKE_BALL), // pick Bulbasaur (+ rival battle)

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

            // ── LONE STARTER (no caught second mon). A weak, low-level catch (the old Route 1 Pidgey)
            // is worse than nothing in the attrition dungeons: when the starter faints there, the game
            // forces the weakling in, and — being alive but unable to win *or* flee a trainer battle —
            // it blocks the black-out that would otherwise heal the whole party, hard-deadlocking the
            // crossing (observed in Mt Moon). With just the starter, a faint triggers a black-out →
            // heal → re-enter and re-fight already a little stronger, which clears Mt Moon and the
            // Nugget Bridge by convergent recovery. (So we also skip buying Poké Balls above.)

            // ── Grind the starter on Route 1 ──
            Self::enter(Map::Route1),
            Self::GrindUntilLevel { target_level: 13, on_map: Map::Route1, slot: 0 },
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
            // Exit the gym to the city first (a single warp): every forward `enter` must be one
            // direct transition. Jumping straight to the Pokécenter from inside the gym is a 2-hop
            // path that would rely on routing through a never-before-observed gym-exit landing.
            Self::enter(Map::PewterCity),
            Self::enter(Map::PewterPokecenter),
            Self::Interact(MapSprite::PEWTERPOKECENTER_NURSE),
            Self::enter(Map::PewterCity),

            // ── Route 3 grind → heal at the Mt Moon Pokécenter ──
            Self::enter(Map::Route3),
            Self::GrindUntilLevel { target_level: 18, on_map: Map::Route3, slot: 0 },
            Self::enter(Map::Route4),
            Self::enter(Map::MtMoonPokecenter),
            Self::Interact(MapSprite::MTMOONPOKECENTER_NURSE),
            Self::enter(Map::Route4),
            Self::enter(Map::MtMoon1F),
        ];

        // ── Cross Mt Moon → Cerulean City ──
        steps.extend(Self::mt_moon_traversal());

        steps.extend([
            // ── Heal in Cerulean, then beat Misty (Cascade Badge) ──
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
            Self::enter(Map::CeruleanCity),
            Self::DefeatGymLeader { leader: MapSprite::CERULEANGYM_MISTY, badge: Badge::CascadeBadge },
            // Exit the gym to the city (single warp) before entering the Pokécenter — see the
            // Pewter gym note above.
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
        ]);

        // ── Bill (SS Ticket) → trashed-house bridge → Vermilion City ──
        steps.extend(Self::cerulean_to_vermilion_steps());
        // ── S.S. Anne: clear every trainer, beat the rival, get HM01 Cut from the captain ──
        steps.extend(Self::ss_anne_steps());
        // ── Thunder Badge: teach Cut → cut the gym tree → trash-can puzzle → Lt. Surge ──
        steps.extend(Self::thunder_badge_steps());
        // ── Back to Cerulean (Underground Path in reverse) → Rock Tunnel → Lavender ──
        steps.extend(Self::back_to_cerulean_steps());
        steps.extend(Self::cerulean_to_lavender_steps());
        // ── Lavender → Celadon (Route 7–8 Underground Path) → Rainbow Badge (Erika) ──
        steps.extend(Self::lavender_to_celadon_steps());
        steps.extend(Self::celadon_rainbow_steps());
        // ── Celadon Game Corner → Rocket Hideout → Silph Scope (Giovanni) ──
        steps.extend(Self::rocket_hideout_entrance_steps());
        steps.extend(Self::silph_scope_steps());
        // ── Pokémon Tower (Silph Scope) → Poké Flute from Mr. Fuji ──
        steps.extend(Self::poke_flute_steps());
        // ── Route 12: wake the Snorlax blocking the road south with the Poké Flute ──
        steps.extend(Self::snorlax_steps());
        // ── Route 12–15 → Fuchsia → Soul Badge (Koga) ──
        steps.extend(Self::soul_badge_steps());
        // ── Safari Zone: HM03 Surf + Gold Teeth → HM04 Strength (Warden) ──
        steps.extend(Self::safari_zone_surf_steps());
        steps.extend(Self::safari_zone_strength_steps());
        // ── Fuchsia → Celadon (buy Super Potions) → Saffron ──
        steps.extend(Self::saffron_entry_steps());
        // ── Celadon: free Eevee → Vaporeon, teach Surf, grind it — needed to beat the Silph rival's
        //    Alakazam (and for Surf to Cinnabar). Done BEFORE Silph. ──
        steps.extend(Self::eevee_vaporeon_surf_steps());
        // ── Silph Co: Card Key → Giovanni (liberates Saffron) → out + heal ──
        steps.extend(Self::silph_co_card_key_steps());
        steps.extend(Self::silph_giovanni_steps());
        // ── Saffron Gym → Marsh Badge (Sabrina) ──
        steps.extend(Self::marsh_badge_steps());
        // ── Surf across Route 21 to Cinnabar Island ──
        steps.extend(Self::saffron_to_cinnabar_steps());
        // ── Pokémon Mansion → Secret Key → Cinnabar Gym → Volcano Badge (Blaine) ──
        steps.extend(Self::mansion_secret_key_steps());
        steps.extend(Self::volcano_badge_steps());
        // ── Seafoam Islands (off Cinnabar) → Master-Ball ARTICUNO, the Elite-Four Ice sweeper ──
        // Starts and ends on Cinnabar Island, so it drops straight in ahead of the Earth Badge. It adds
        // two party members: a Slowpoke HM-slave (Strength + Dig) at slot 2 and Articuno at slot 3.
        steps.extend(Self::seafoam_articuno_steps());
        // ── Cinnabar → Viridian Gym → Earth Badge (Giovanni), the 8th and final gym badge ──
        steps.extend(Self::earth_badge_steps());
        // ── Victory Road 1F: catch a Strength HM-slave, solve the boulder puzzle, climb to VR2F ──
        // (The full VR2F/VR3F puzzle works — `can_solve_victory_road_2f_3f` — but chaining it here is
        // PP-marginal for this team; see `victory_road_2f_3f_steps`.)
        steps.extend(Self::victory_road_1f_steps());

        steps
    }
}

pub struct DeterministicPolicy {
    rng: StdRng,
    queue: VecDeque<PolicyStep>,
    name_picker: PokemonNamePicker,
    /// The last Pokémon Center where the player was healed.
    pub last_pokemon_center: Option<Map>,
    /// Set to `Some(pokecenter)` when the active Pokémon's damaging moves are all at ≤10% PP
    /// and the policy decided to flee the current wild battle. The policy will navigate to that
    /// Pokémon Center and heal before resuming the main queue.
    heal_return: Option<Map>,
    /// Number of times the current `BuyFromMart` step has re-opened the shop without the purchase
    /// registering in the bag. The clerk-entry path occasionally drops the YES-confirm (no clean
    /// joypad rising edge), so the step verifies the bag and retries a few times before giving up
    /// (e.g. for an item the mart doesn't actually sell).
    mart_attempts: u32,
    /// Consecutive ticks the current `DefeatGymLeader` step has failed to find a route to its gym.
    /// A lost gym battle blacks the player out, and for a few ticks after that warp the map/actions are
    /// still unsettled, so routing legitimately fails; popping the step on the first failure (as this
    /// used to) strands the run with an unwinnable queue. Bounded so a genuinely unreachable gym still
    /// gives up instead of deadlocking.
    gym_route_stuck: u32,
    /// The map a `Dig` step was issued from, so the step can pop when Dig has actually warped the
    /// player somewhere else (rather than on the first tick, before the menus have even opened).
    dig_from_map: Option<Map>,
    /// True once the current `CollectItem` step's item sprite has been observed present (not hidden).
    /// The step then pops only when the item *disappears* (collected). Distinguishes "collected" from
    /// "not yet revealed" — some item balls stay hidden until their guard is beaten (e.g. the Rocket
    /// Hideout Lift Key / Silph Scope), and popping on the initial hidden state would skip them.
    collect_item_seen: bool,
    /// Consecutive ticks a `CatchPokemon` step has found no encounter source (no grass/cave-object/water).
    /// On map entry the tile grid is momentarily unsettled (sprites can read out of bounds), so we WAIT a
    /// bounded number of ticks for it to settle rather than popping the catch immediately.
    catch_wander_stuck: u32,
    /// When `Some(slot)`, switch that party slot in at the start of every battle (wild *and* trainer)
    /// so it — not the lead — earns the XP. Used to train a bench mon (e.g. Vaporeon) on the trainer
    /// gauntlet. A safety cap skips the switch when the enemy out-levels the trainee by a wide margin,
    /// so it won't suicide into a much stronger foe (e.g. the rival's ace). Toggle with `SetTrainSlot`.
    train_slot: Option<u8>,
    /// During a `GrindUntilLevel` grind: set once the trainee has been switched into / handed off from the
    /// CURRENT battle (reset each overworld tick). Stops train_slot re-switching a just-handed-off trainee
    /// straight back in (which would oscillate with the low-HP hand-off and let it faint anyway).
    trainee_participated: bool,
    /// Consecutive policy ticks the current `InteractIfReachable` step has waited without the sprite
    /// becoming reachable. Past a threshold the step gives up and pops (the trainer is walled off by
    /// the teleport-pad maze) instead of stalling forever like the plain `Interact`.
    interact_skip_waits: u32,
    /// The value of `mansion_switch_on` captured when the current Pokémon Mansion `FlipSwitch` step
    /// began. The step completes once the flag differs — i.e. the single global switch has toggled
    /// exactly once — so each `FlipSwitch` is one deterministic flip (not an oscillating retry loop).
    mansion_flip_baseline: Option<bool>,
    /// How many of the wanted item the bag held when the current `SearchHiddenItem` step began, so
    /// the pick-up is detected as an *increase*. Same shape, and the same reason, as
    /// [`Self::mansion_flip_baseline`].
    hidden_item_baseline: Option<u8>,
    /// Visible-boulder count captured when the current `DropBoulderInHole` step began. The step completes
    /// once the count drops (a boulder was pushed onto the hole and fell to the floor below), so exactly
    /// one boulder is dropped rather than every boulder that can reach the hole.
    boulder_drop_baseline: Option<usize>,
    /// The `EvolveWithStone` target the baseline below belongs to, so a later step aimed at a
    /// different mon starts its own baseline rather than inheriting this one.
    evolve_baseline: Option<(PartyRef, PokemonSpecies)>,
    /// Positions of gym trainers already beaten during the current `DefeatGymLeader` step (Cinnabar's
    /// quiz-gate maze): a defeated trainer stays on the map as a sprite, so once we detect we're
    /// standing in its line of sight with no battle starting, we record it here to avoid re-targeting.
    gym_beaten: HashSet<Point8>,
    /// Casts the current `Fish` step has issued (workstream C). A cast's outcome is invisible from the
    /// overworld — the bite is a battle that is over by the time the policy is polled again, and this
    /// save has already *seen* every fishable species — so the cast count is what both `FishGoal`s are
    /// measured against. Reset whenever the step pops.
    fish_casts: u32,
    /// Trip bookkeeping for the current `SafariHunt` step (workstream E): how many ¥500 entries have
    /// been paid, and whether we were inside the zone last tick — an ejection at 0 steps is an
    /// *edge*, and `EVENT_IN_SAFARI_ZONE` is only a level. Reset whenever the step pops.
    safari: crate::pokemon::postgame::safari::HuntProgress,
    /// `(trainer position, player position, consecutive stuck ticks)` for the gym-trainer engagement.
    /// If we keep targeting the same beaten trainer from the same frozen spot (its after-battle text
    /// aborts the approach, no battle starts), the counter climbs until we mark it beaten and move on.
    gym_engage: Option<(Point8, Point8, u32)>,
    /// Ticks an `EnterMapIfReachable` step has waited for a route (workstream L). The wait exists at
    /// all because a map's action list is briefly empty on arrival while the sprites settle, so
    /// "cannot reach" has to mean "still cannot reach a few seconds later".
    enter_stuck: u32,
    /// Bag quantity of the current `UseBagItem` step's item when the step began (workstream I), so
    /// completion is "one of them left the bag" and a stack of four Revives spends exactly one.
    item_use_baseline: Option<u8>,
    /// How many times the current `UseBagItem` step has been handed to the driver. Two jobs: it is
    /// the completion test for an item with **no observable at all** (the Itemfinder prints a text
    /// box and changes no RAM), and it bounds a use the game silently declines for a reason
    /// [`crate::pokemon::postgame::items::blocked`] does not model.
    item_use_attempts: u32,
    /// Bag quantities of a `UseItemsInBattle` step's items when it began, parallel to its list. The
    /// step spends exactly one of each, and "spent" has to be measured against a baseline because
    /// several of them (X Attack, Dire Hit) leave no trace once the battle ends.
    battle_item_baseline: Option<Vec<u8>>,
}


/// Whether a map sprite is a static encounter of `species`.
///
/// Not equality, because **a map with more than one of a species numbers them**: the Power Plant's
/// disguised Poké Balls are `Voltorb 1..6` and `Electrode 1`/`Electrode 2`, so an exact match finds
/// neither and `CatchPokemon` falls through to pacing for a wild encounter on a map that has none.
/// The legendaries are unnumbered and match either way.
fn sprite_is_species(name: &str, species: PokemonSpecies) -> bool {
    name.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ') == species.to_string()
}

impl DeterministicPolicy {
    /// Whether the step at the queue front wants `enemy` caught, and with which ball.
    ///
    /// `Some(None)` means "catch it, best ball in the bag" — distinct from `None`, which means this
    /// battle is not a catch at all. Both the flee test and the throw read it, so the two can never
    /// disagree: fleeing from a species the throw block would have caught is how a sweep silently
    /// never finishes.
    fn catch_target(&self, state: &GameState, enemy: PokemonSpecies) -> Option<Option<ItemId>> {
        match self.queue.front() {
            Some(&PolicyStep::CatchPokemon { species, ball, .. }) if species == enemy => Some(ball),
            Some(&PolicyStep::SweepDex { ball, .. })
                if crate::pokemon::postgame::aides::sweep_wants(state, enemy) => Some(ball),
            _ => None,
        }
    }

    /// How many times to re-open the shop for one `BuyFromMart` step before giving up.
    /// Ticks a `DefeatGymLeader` step waits for a route to its gym before concluding there isn't one
    /// (20 ms each, so ~8 s of game time — long enough to cover a black-out warp and its dialogue).
    const MAX_GYM_ROUTE_WAIT: u32 = 400;
    const MAX_MART_ATTEMPTS: u32 = 4;
    /// How many times to hand one `UseBagItem` step to the driver before giving up (workstream I).
    /// A use the game declines consumes nothing, so without a bound the step retries for the whole
    /// leg — the same shape as the full-bag trap, and just as quiet.
    const MAX_ITEM_USE_ATTEMPTS: u32 = 4;
    /// **Policy polls**, not ticks, that one `EnterMapIfReachable` spends before giving up.
    ///
    /// Polls rather than ticks because that is what the policy can count, and it is the better
    /// measure anyway: a legitimate long walk issues *one* action and then spends hundreds of ticks
    /// walking it, so 60 polls is 60 *attempts*, not 60 ticks — several times what any single
    /// transition needs, while a door that bounces the player back burns a poll every few ticks.
    ///
    /// ⚠️ **It was 600, then 200, and both were too generous** — for opposite reasons at each end of
    /// the map. Indoors, a sealed door (the Vermilion dock) burns polls fast, and at 600 the *second*
    /// consecutive give-up outlasted the harness's ten-minute stall window. Outdoors it is the other
    /// way round: one poll on Route 12 is a walk of several minutes, so 200 attempts is hours of game
    /// time and the tour ran out of cycle budget in Lavender instead. Attempts are cheap to be wrong
    /// about in only one direction; keep this small.
    const MAX_ENTER_WAIT: u32 = 60;

    pub fn new(seed: u64, steps: impl IntoIterator<Item = PolicyStep>) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
            queue: steps.into_iter().collect(),
            name_picker: PokemonNamePicker::seed_from_u64(seed),
            last_pokemon_center: None,
            heal_return: None,
            mart_attempts: 0,
            gym_route_stuck: 0,
            dig_from_map: None,
            collect_item_seen: false,
            catch_wander_stuck: 0,
            mansion_flip_baseline: None,
            hidden_item_baseline: None,
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
        }
    }

    /// Route one hop toward `target` over the **incremental** world graph.
    ///
    /// The graph only contains sections the agent has already visited (accurate, sprite-resolved),
    /// so this succeeds for backtracking / already-explored territory (heal-return, reaching a map
    /// the explicit `EnterMap` steps have already led through) and returns `None` for a not-yet-
    /// visited target — the signal that the deterministic policy is under-specified.
    pub(crate) fn route_toward(world_graph: &WorldGraph, actions: &[OverworldAction], target: Map) -> Option<OverworldAction> {
        world_graph.pick_shortest_path_action(actions, target)
    }

    /// Plan the Sokoban to land a boulder on `target` (a Strength switch or a floor hole) and return the
    /// FIRST one-tile push as a `FieldMove::PushBoulder`, or `None` if no boulder can reach it right now
    /// (the caller waits and re-plans next tick). The planner `MetaTileMap::solve_boulder_push` is a shared
    /// helper any policy can call; the deterministic policy just drives its pushes one at a time.
    fn next_boulder_push(state: &GameState, target: Point8) -> Option<FieldMove> {
        let (boulder, dir) = state.map.solve_boulder_push(target)?.into_iter().next()?;
        Some(FieldMove::PushBoulder { boulder, dir })
    }

    /// The action that takes the warp/connection to `to_map` (matching raw `to_position` when
    /// given) from the current map, or `None` if no such transition is reachable here.
    fn enter_map_action(actions: &[OverworldAction], to_map: Map, to_position: Option<Point8>) -> Option<OverworldAction> {
        // Prefer the *nearest* matching warp/connection. A door can span two vertically-adjacent warp
        // tiles (e.g. a gate's 2-tall door) where only the bottom tile — the one the player actually
        // stands on — fires; picking the closest reliably lands on it, whereas taking the first in
        // `actions` order could target the non-firing top tile and jam.
        actions.iter().filter(|a| match a.tile {
            MetaTile::Warp { to_map: m, to_position: p }
            | MetaTile::Connection { to_map: m, to_position: p } => {
                m == to_map && to_position.map_or(true, |want| want == p)
            }
            // Water connection (a surfable map edge): matched by destination map only — it carries no
            // landing `to_position`. The agent Surfs across to reach it.
            MetaTile::ConnectionWater(m) => m == to_map,
            _ => false,
        }).min_by_key(|a| a.route.len()).cloned()
    }

    pub fn complete_game(seed: u64) -> Self {
        Self::new(seed, PolicyStep::complete_game_steps())
    }
}

impl Policy for DeterministicPolicy {
    fn name(&self) -> &'static str { "scripted" }


    fn pick_overworld_action(&mut self, state: &GameState, world_graph: &WorldGraph) -> Option<OverworldAction> {
        // Back in the overworld = the previous battle is over; clear the per-battle grind participation flag.
        self.trainee_participated = false;
        if state.map.map.is_pokemon_center() {
            self.last_pokemon_center = Some(state.map.map);
        }

        let actions = state.map.actions();

        // ── Heal-return detour ────────────────────────────────────────────────
        // When the active Pokémon ran low on PP in a wild battle we fled and
        // stored the target Pokémon Center in `heal_return`.  Route there over the
        // incrementally-built graph (the pokecenter and the way back are already known,
        // since we walked here) and talk to the Nurse before resuming the main queue.
        if let Some(pokecenter) = self.heal_return {
            return if state.map.map == pokecenter {
                // Arrived — find and interact with the Nurse.
                if let Some(action) = actions.iter().find(|a| a.tile == MetaTile::Sprite("Nurse")) {
                    self.heal_return = None;
                    Some(action.clone())
                } else {
                    // Pokecenter map but Nurse tile not visible yet — wait.
                    None
                }
            } else {
                // Still travelling — pick next step toward the pokecenter.
                Self::route_toward(world_graph, &actions, pokecenter)
            };
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
                    // Explicit single map transition: take exactly this warp/connection.
                    if let Some(action) = Self::enter_map_action(&actions, to_map, to_position) {
                        return Some(action);
                    }
                    // A specific connection landing that isn't the nearest crossing (which is all
                    // `actions()` emits) — build it directly (e.g. Route 13→14 open row, not the pocket).
                    if let Some(pos) = to_position {
                        if let Some(action) = state.map.connection_action(to_map, pos) {
                            return Some(action);
                        }
                        // No *land* crossing lands there. Then the caller is asking for the far side of
                        // a water edge, whose `ConnectionWater` tile carries no landing position and
                        // which `actions()` only ever emits when it is the nearest route to that map —
                        // so wherever a footbridge sits beside a river seam, the seam is otherwise
                        // unaskable. Route 24 → Cerulean is exactly that, and the seam is the only way
                        // into the half of Cerulean that holds Cerulean Cave.
                        if let Some(action) = state.map.water_connection_action(to_map) {
                            return Some(action);
                        }
                    }
                    // Recovery: the direct transition isn't on the current map. This happens when a
                    // teleport back into already-explored territory desyncs the linear EnterMap
                    // script — a blackout (fainting) sends the player home, and the heal-flee detour
                    // moves them to a Pokémon Center. If the target map has already been observed,
                    // route back toward it over the incremental world graph (visited territory only).
                    // If it has NOT been observed this returns None and the agent stalls — the
                    // intended hard-fail for genuinely under-specified forward travel.
                    Self::route_toward(world_graph, &actions, to_map)
                },
                PolicyStep::EnterMapIfReachable { to_map } => {
                    // **Workstream L.** `EnterMap`'s body, with a give-up instead of a stall.
                    if state.map.map == to_map {
                        self.enter_stuck = 0;
                        self.queue.pop_front();
                        continue;
                    }
                    // ⚠️ The counter runs on **every** poll, not only when there is no action to
                    // take — because the failure this step exists to survive is not always "nowhere
                    // to go". Vermilion's tour found the other shape: the agent stood at (18,30)
                    // being handed a perfectly good walk to the `VermilionDock` door, over and over,
                    // and the map never changed. An action-less counter resets on each of those and
                    // never fires. Only "this transition has not happened in a minute of game time"
                    // catches both.
                    self.enter_stuck += 1;
                    if self.enter_stuck >= Self::MAX_ENTER_WAIT {
                        println!("[policy] TOUR: gave up entering {to_map} from {} after {} ticks",
                            state.map.map, self.enter_stuck);
                        self.enter_stuck = 0;
                        self.queue.pop_front();
                        continue;
                    }
                    Self::enter_map_action(&actions, to_map, None)
                        .or_else(|| Self::route_toward(world_graph, &actions, to_map))
                },
                PolicyStep::Goto { map: target, strict } => {
                    if state.map.map == target {
                        self.queue.pop_front();
                        continue;
                    }
                    let action = Self::route_toward(world_graph, &actions, target);
                    if !strict && action.is_some() {
                        // a non-strict goto action can be interrupted
                        self.queue.pop_front();
                    }
                    action
                },
                PolicyStep::CatchPokemon { species, on_map, .. } => {
                    if state.map.map != on_map {
                        let action = Self::route_toward(world_graph, &actions, on_map);
                        if action.is_none() {
                            println!("[policy] want to catch pokemon {} in {}, but no path there!", species, on_map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if state.pokedex_owned.contains(&species) {
                        // caught the pokemon (note this only works once for each species)
                        self.queue.pop_front();
                        continue;
                    } else if state.bag.best_pokeball().is_none() {
                        println!("[policy] want to catch a {}, but no Pokéballs left!", species);
                        self.queue.pop_front();
                        continue;
                    } else if on_map.sprites().iter().any(|s| sprite_is_species(s.name, species)) {
                        // STATIC encounter: the legendaries (Articuno on Seafoam B4F, …) are not wild
                        // spawns at all — they are ordinary map sprites named after the species, and
                        // the battle starts by walking into one and pressing A. Route straight to it
                        // instead of pacing for a random encounter. This must be its own branch: the
                        // wander fallback below would otherwise stroll off across the map, which on
                        // B4F means leaving the west lake by its one-way (7,11) shore and stranding
                        // the catch (the game refuses to let you Surf back on).
                        match actions.iter().find(|a| matches!(a.tile, MetaTile::Sprite(n) if sprite_is_species(n, species))) {
                            Some(action) => {
                                println!("[policy] static encounter: routing to {species} at {} ({} steps)",
                                    action.destination, action.route.len());
                                self.catch_wander_stuck = 0;
                                Some(action.clone())
                            }
                            None => {
                                // Not actionable yet. Right after a warp the sprite list is briefly
                                // unsettled, so wait for it rather than wandering; past the bound the
                                // target really is walled off (or already beaten) and we move on.
                                //
                                // A *hidden* sprite is a different thing and gets a much shorter fuse:
                                // a static encounter's sprite vanishes the moment its battle starts and
                                // does not come back until the map is reloaded, so if we are still here
                                // and it is hidden, the encounter happened and was not won (we fled, or
                                // blacked out). Nothing on this map will bring it back — pop, and let
                                // the caller's next step warp out and back in to respawn it.
                                self.catch_wander_stuck += 1;
                                let spent = state.map.sprites.iter()
                                    .any(|s| s.hidden && sprite_is_species(s.name, species));
                                if self.catch_wander_stuck < if spent { 50 } else { 400 } {
                                    None
                                } else {
                                    println!("[policy] {species} is on {on_map} but unreachable (gave up)");
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
                        // No grass (a cave): walk to the farthest reachable object (e.g. a boulder). The
                        // long transit ping-pongs across the cave and cave encounters fire per step.
                        self.catch_wander_stuck = 0;
                        Some(action.clone())
                    } else if let Some(action) = state.map.wander_action() {
                        // No grass and no reachable cave object (a pocket, or a water map like Seafoam):
                        // pace to the farthest reachable walkable tile — walking/Surfing fires per-step
                        // encounters just the same.
                        self.catch_wander_stuck = 0;
                        Some(action)
                    } else {
                        // No encounter source THIS tick. On map entry the tile grid is briefly unsettled
                        // (sprites can read out of bounds → no reachable cave object), so wait a bounded
                        // number of ticks for it to settle before giving up, rather than popping instantly.
                        self.catch_wander_stuck += 1;
                        if self.catch_wander_stuck < 400 {
                            None // wait
                        } else {
                            println!("[policy] want to catch a {species}, but nowhere to trigger an encounter (gave up)!");
                            self.catch_wander_stuck = 0;
                            self.queue.pop_front();
                            continue;
                        }
                    }
                },
                PolicyStep::SweepDex { on_map, min_share, .. } => {
                    // **Workstream H (H5).** The same wander as `CatchPokemon` above, minus its
                    // static-encounter branch (a sweep is wild-only) — what differs is the *stop*
                    // condition, which is a set rather than one species.
                    use crate::pokemon::postgame::aides;
                    if state.map.map != on_map {
                        let action = Self::route_toward(world_graph, &actions, on_map);
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
                        // A cave: pace between the farthest objects, which fires per-step encounters.
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
                PolicyStep::GrindUntilLevel { target_level, on_map, slot } => {
                    if let Some(pokemon) = state.pokemon.get(slot as usize) {
                        if pokemon.level >= target_level {
                            self.queue.pop_front();
                            continue;
                        }
                        // The grind mon fainted. When training a bench slot the lead (Venusaur) keeps
                        // the party alive, so there's no black-out to auto-heal it — detour to the last
                        // Pokémon Center, then the grind resumes with a revived mon.
                        if pokemon.current_hp == 0 {
                            if let Some(center) = self.last_pokemon_center {
                                println!("[policy] grind mon (slot {slot}) fainted — routing to {center} to heal");
                                self.heal_return = Some(center);
                                return Self::route_toward(world_graph, &actions, center);
                            }
                        }
                    } else {
                        println!("[policy] no Pokemon in slot {slot} to level up");
                        self.queue.pop_front();
                        continue;
                    }
                    if state.map.map != on_map {
                        let action = Self::route_toward(world_graph, &actions, on_map);
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
                        // Walk in the NEAREST grass to trigger encounters — ping-pong locally rather than
                        // marching across the map. On a long route (Route 23) wandering far can cross a
                        // one-way ledge into a pocket from which the Pokémon Center becomes unreachable,
                        // stranding the heal-return trek; staying near the entry keeps the center routable.
                        Some(action.clone())
                    } else if let Some(action) = actions.iter()
                        .filter(|a| match a.tile {
                            // Pace to the farthest reachable object/warp so wild encounters (which fire on
                            // EVERY step in a cave/building) keep coming as the trainee ping-pongs across the
                            // map. EXCLUDE item-ball sprites (`PictureId::PokeBall`): walking onto one triggers
                            // a pickup that aborts on a full bag and loops forever (Pokémon Mansion). Warps
                            // (stairs) are fine targets — taking one just changes floor, and GrindUntilLevel
                            // routes back; the transit still triggers encounters.
                            MetaTile::Sprite(name) => !state.map.sprites.iter()
                                .any(|s| s.name == name && s.picture_id == crate::pokemon::sprite::PictureId::PokeBall),
                            MetaTile::Warp { .. } => true,
                            _ => false,
                        })
                        .max_by_key(|a| a.route.len()) {
                        Some(action.clone())
                    } else {
                        println!("[policy] cannot level up a Pokemon, no grass or cave objects nearby!");
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
                        // Losing to the leader blacks the player out to the last Pokémon Center — which
                        // is the whole reason this step never pops itself on a defeat. Getting back in
                        // is not just a walk, though: the black-out reloads the map, so any tree the
                        // leg cut on its way in has **regrown**, and Celadon's gym entrance is sealed by
                        // exactly such trees. So when there is no route, re-cut whatever is reachable
                        // first — `CutTree` pops itself once no reachable tree remains, handing the gym
                        // back to this step with the way open.
                        match Self::route_toward(world_graph, &actions, leader.map()) {
                            Some(action) => { self.gym_route_stuck = 0; Some(action) }
                            None if actions.iter().any(|a| a.tile == MetaTile::CutTree) => {
                                println!("[policy] no route to {} — cutting the regrown trees on {}",
                                    leader.map(), state.map.map);
                                self.gym_route_stuck = 0;
                                self.queue.push_front(PolicyStep::CutTree { map: state.map.map });
                                continue;
                            }
                            None => {
                                // Right after the black-out warp the map and its actions are briefly
                                // unsettled, so wait rather than giving up on the first miss; past the
                                // bound the gym really is unreachable (wrong order, a gate still shut)
                                // and the run moves on.
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
                        // If the player loses and blacks out, the step remains and the agent
                        // navigates back to try again.
                        Some(a.clone())
                    } else if actions.iter().any(|a| a.tile == MetaTile::CutTree) {
                        // The leader is walled off behind cuttable trees. Celadon's gym is a garden maze
                        // of them, and they all regrow when the map reloads — which is exactly what a
                        // black-out on a failed attempt does, leaving the run inside a gym it can no
                        // longer cross. Re-cut and come back to this step (see the sibling recovery in
                        // the off-map branch above).
                        println!("[policy] {} is walled off — cutting the regrown trees in {}",
                            leader, state.map.map);
                        self.queue.push_front(PolicyStep::CutTree { map: state.map.map });
                        continue;
                    } else {
                        // The leader isn't reachable yet — in a gated gym (Cinnabar's quiz-gate snake
                        // maze) the path opens only by beating the junior trainers, each of whom unlocks
                        // the gate ahead. Every gym trainer faces DOWN, so engage the nearest one via its
                        // line of sight: route to the tile directly below it (`route_to_face_dir(.., Up)`
                        // lands on that LOS tile) — a plain adjacent-approach fails when the maze arrives
                        // from behind the trainer. A beaten trainer stays on the map as a sprite; when we
                        // find ourselves already in its LOS with no battle starting, it's beaten, so
                        // record it and skip. Re-evaluated every tick until the leader opens up.
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
                                // Standing in this trainer's LOS but not in battle → it's already beaten.
                                self.gym_beaten.insert(pos);
                                continue;
                            }
                            // Stuck detection: re-targeting the same trainer from the same spot for many
                            // ticks (its after-battle text keeps aborting the approach) → it's beaten.
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
                        // Not in the trainer's room yet (a preceding `enter` normally places us here).
                        let action = Self::route_toward(world_graph, &actions, trainer.map());
                        if action.is_none() { self.queue.pop_front(); continue; }
                        action
                    } else if let Some(sprite) = state.map.sprites.iter().find(|s| !s.hidden && s.name == trainer.name) {
                        let pos = sprite.position;
                        let cur = state.map.player_position;
                        match state.map.route_to_face_dir(pos, Some(PlayerFacingDirection::Up)) {
                            // Standing in the trainer's LOS with no battle → it's beaten. Advance.
                            Some(route) if route.is_empty() => {
                                self.gym_engage = None;
                                self.queue.pop_front();
                                continue;
                            }
                            Some(route) => {
                                // Stuck detection: re-targeting from the same frozen spot (its after-battle
                                // text keeps aborting the approach) → beaten.
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
                    // Prefer the sprite visible on the CURRENT map. Sprite identity is by name only,
                    // so sprites that recur across maps (Nurse, Clerk) cannot be disambiguated by
                    // `sprite.map()` — it returns the first map in enum order, misrouting every
                    // pokecenter heal but the first. The scripted `enter(map)` step preceding each
                    // `Interact` already places the agent on the intended map, so matching the
                    // visible sprite here by name is both correct and robust.
                    if let Some(action) = actions.iter().find(|a| a.tile == MetaTile::Sprite(sprite.name)) {
                        self.queue.pop_front();
                        return Some(action.clone());
                    }
                    let map = sprite.map();
                    if state.map.map == map {
                        // On the sprite's map but it isn't actionable yet (e.g. still walking on, or
                        // the sprite is briefly hidden by a script) — wait for it. (Do NOT pop when the
                        // sprite is hidden: some sprites hide transiently mid-script, e.g. Bill right
                        // after the PC, and popping would abort the interaction. Sprites that vanish
                        // permanently on defeat, like the Game Corner Rocket, are handled by a single
                        // non-retried `Interact` that pops the instant it issues the walk.)
                        None
                    } else {
                        let action = Self::route_toward(world_graph, &actions, map);
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
                        // On the sprite's map but it isn't in the reachable set — either still loading,
                        // or walled off by the maze. Wait a bounded number of ticks, then give up.
                        self.interact_skip_waits += 1;
                        if self.interact_skip_waits > 250 {
                            println!("[policy] {} unreachable after waiting — skipping", sprite);
                            self.interact_skip_waits = 0;
                            self.queue.pop_front();
                            continue;
                        }
                        None
                    } else {
                        let action = Self::route_toward(world_graph, &actions, map);
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
                        let action = Self::route_toward(world_graph, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to use the PC on {}, but no path there!", map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if let Some(action) = actions.iter().find(|a| a.tile == MetaTile::Pc) {
                        // On the PC's map and the PC is reachable — face it and press A, then advance.
                        self.queue.pop_front();
                        return Some(action.clone());
                    } else {
                        // On the map but the PC isn't reachable yet (e.g. a script is still running) —
                        // wait for it to become actionable.
                        None
                    }
                }
                // Reserved seams (task 0.8) — inert until their workstream implements them. To take
                // one: move your variant out of this list into its own one-line arm delegating to
                // your own module. That is a one-line edit to this file, which is the whole point.
                PolicyStep::UseFlash { .. } => None, // on the map — `pick_field_move` drives the menu
                PolicyStep::SearchHiddenItem { map, .. } => {
                    // **Workstream H.** Routing only, like `Fish`; `pick_field_move` owns the walk to
                    // the tile and the A press once we are standing on `map`.
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &actions, map);
                        if action.is_none() {
                            println!("[policy] want the hidden item on {map}, but no path there!");
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        None
                    }
                }
                PolicyStep::Fish { map, .. } => {
                    // Routing only, like `UseItemPc` below: once we are standing on `map`,
                    // `pick_field_move` picks the water tile and hands each cast to the driver.
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &actions, map);
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
                    // **Workstream E.** Paying, pacing the grass, being ejected at 0 steps and walking
                    // back in are all one step — see `postgame::safari::pick`.
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
                    let action = Self::route_toward(world_graph, &actions, Map::GameCornerPrizeRoom);
                    if action.is_none() {
                        println!("[policy] want a prize, but no path to the prize room!");
                        self.queue.pop_front();
                        continue;
                    }
                    action
                }
                PolicyStep::RedeemPrize { .. } => None, // on the map — handed to `pick_field_move`
                PolicyStep::PartyScript { .. } => None, // already routed by a preceding `enter` step
                PolicyStep::UseBagItem { .. } => None,  // **I** — `pick_field_move` owns the menus
                PolicyStep::UseItemsInBattle { on_map, items } => {
                    // **I3/I4.** The same wander as `SweepDex`, with a different stop condition: every
                    // item spent. Getting *into* a wild battle is the only reason to walk at all.
                    if state.map.map != on_map {
                        let action = Self::route_toward(world_graph, &actions, on_map);
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
                    // Routing only. Once we are standing on `map`, `pick_field_move` hands the step to
                    // the storage driver, which walks the last tiles itself so that it — and not the
                    // generic overworld executor — owns the A press that opens the PC.
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &actions, map);
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
                        let action = Self::route_toward(world_graph, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to collect {} on {}, but no path there!", sprite, map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else {
                        let present = state.map.sprites.iter().any(|s| !s.hidden && s.name == sprite.name);
                        if present { self.collect_item_seen = true; }
                        if !present && self.collect_item_seen {
                            // The item was here and is now gone — picked up (or removed by a script). Done.
                            self.collect_item_seen = false;
                            self.queue.pop_front();
                            continue;
                        }
                        if !present {
                            // Not yet revealed (an item ball hidden until its guard is beaten) — wait.
                            None
                        } else {
                            // Keep walking to and pressing A on the item until it disappears; do NOT
                            // pop on issue, so a battle/script interruption (Mt Moon Super Nerd) mid-walk
                            // doesn't abandon the pickup.
                            actions.iter()
                                .find(|a| a.tile == MetaTile::Sprite(sprite.name))
                                .cloned()
                        }
                    }
                }
                PolicyStep::BuyFromMart { item, map } => {
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &actions, map);
                        if action.is_none() {
                            println!("[policy] want to buy {} from {} but no path there!", item, map);
                            self.queue.pop_front();
                            continue;
                        }
                        action
                    } else if state.bag.iter().any(|i| i.id == item.id && i.quantity >= item.quantity) {
                        // Purchase registered (bag now holds ≥ the target quantity) — done.
                        self.mart_attempts = 0;
                        self.queue.pop_front();
                        continue;
                    } else if self.mart_attempts >= Self::MAX_MART_ATTEMPTS {
                        // The shop re-opened this many times without the item appearing. Either the mart
                        // doesn't sell it (e.g. Potion in Viridian), or the bag is at its 20-slot cap and
                        // the game answered "You can't carry any more items." — note that `state.bag` can
                        // look far shorter than 20, because `Bag`'s reader drops item ids `ItemId` has no
                        // name for (every TM, mostly). Give up either way.
                        println!("[policy] gave up buying {} from {} after {} attempts", item, map, self.mart_attempts);
                        self.mart_attempts = 0;
                        self.queue.pop_front();
                        continue;
                    } else {
                        // If triggered in the overworld, talk to the "Clerk" sprite to (re)open the
                        // pokemart menu. `pick_mart_purchase` will drive the actual buy; we re-verify
                        // the bag on the next overworld tick and retry if the confirm was dropped.
                        // Single-counter marts name the seller "Clerk"; the Celadon Dept Store 2F has
                        // "Clerk 1" (items — Poké Balls, Super Potions, …) and "Clerk 2" (TMs), so target
                        // the items clerk by name and never the TM one.
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
                    // Handled by `pick_field_move` (the agent calls it first). If we reach here the
                    // teach isn't ready yet — wait without advancing the queue.
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
                PolicyStep::SetTrainSlot(slot) => {
                    self.train_slot = slot;
                    println!("[policy] train_slot = {slot:?}");
                    self.queue.pop_front();
                    continue;
                }
                PolicyStep::MovePokemonToFront { .. } | PolicyStep::Fly { .. } => {
                    // Handled by `pick_field_move` (a direct RAM reorder; the Fly menu chain and town
                    // map, workstream B); wait without advancing.
                    None
                }
                PolicyStep::UseStrength { .. } => {
                    // Handled by `pick_field_move` (party-menu field-move chain); wait without advancing.
                    None
                }
                PolicyStep::SolveBoulders { .. } | PolicyStep::DropBoulderInHole { .. } => {
                    // Handled by `pick_field_move` (plans + drives the boulder pushes); wait without advancing.
                    None
                }
                PolicyStep::CutTree { map } => {
                    if state.map.map != map {
                        let action = Self::route_toward(world_graph, &actions, map);
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
                        // Route to face a reachable tree; once none remain, the trees are cut — done.
                        match actions.iter().find(|a| a.tile == MetaTile::CutTree).cloned() {
                            Some(action) => Some(action),
                            None => { self.queue.pop_front(); continue; }
                        }
                    }
                }
                PolicyStep::SolveTrashCans => {
                    if state.map.map != Map::VermilionGym {
                        let action = Self::route_toward(world_graph, &actions, Map::VermilionGym);
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
                        let action = Self::route_toward(world_graph, &actions, map);
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
                    // Handled by `pick_field_move` once on the elevator map (an `enter(...Elevator)` step
                    // precedes this one). If we're not on an elevator, the elevator can't be used — pop.
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
                    // Facing the target and driving the bag menus is handled by `pick_field_move` /
                    // `UsingFieldItem` once the target sprite is observed on the current map (a preceding
                    // EnterMap places the agent on its map).
                    None
                }
                PolicyStep::UseVendingMachine { .. } => None, // driven by `pick_field_move`
            }
        }
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        let battle_state = state.battle.as_ref()?;
        let actions = battle_options(state)?;

        // Inside Victory Road the low-PP / low-HP "flee to a Pokémon Center" detours must be SUPPRESSED —
        // fleeing out of the multi-floor puzzle to walk all the way back to Viridian abandons the solve and
        // stalls. And every cave wild here is a pure obstacle: fighting them drains PP and interrupts the
        // long boulder solves / floor-to-floor walks. So on a VR map we suppress the heal detours and flee
        // wilds outright (except the Machop catch target). Trainers here are mandatory and fought normally.
        // Only VR2F/VR3F — the long interconnected half where cumulative PP drain matters. VR1F is short
        // and the freshly-healed team fights its wilds fine (fleeing there just shifts the RNG into a
        // cooltrainer PP stalemate); the Machop catch on VR1F is handled by the CatchPokemon arm below.
        // The same reasoning applies inside the **Seafoam Islands**: it is a five-floor, Pokémon-Center-
        // less cave whose only job is to reach Articuno, its wilds are pure obstacles, and a heal-flee
        // detour from four floors down would abandon the warp chain the leg is scripted on.
        let in_center_less_dungeon = matches!(state.map.map,
            Map::VictoryRoad2F | Map::VictoryRoad3F
            | Map::SeafoamIslands1F | Map::SeafoamIslandsB1F | Map::SeafoamIslandsB2F
            | Map::SeafoamIslandsB3F | Map::SeafoamIslandsB4F);

        // Safari Zone. **Workstream E.** During a `SafariHunt` the encounter is the point, so the hunt
        // owns the choice (throw at anything still wanted, run from the rest — see
        // `postgame::safari::pick_battle_action`). Outside one, keep the old behaviour: always RUN, so
        // the legs that merely *cross* the zone for HM03 and the Gold Teeth spend no steps or balls.
        if battle_state.battle_type == BattleType::Safari {
            if let Some(&PolicyStep::SafariHunt { targets, .. }) = self.queue.front() {
                if let Some(action) = crate::pokemon::postgame::safari::pick_battle_action(state, targets, &actions) {
                    return Some(action);
                }
            }
            return Some(BattleAction::Run);
        }

        // **Workstream C.** A wild battle during a `Fish` step is the bite that step went looking for:
        // throw at the target species, flee everything else. Placed before the heal/PP detours below,
        // which would otherwise abandon the session to walk to a Pokémon Center mid-cast.
        if let Some(&PolicyStep::Fish { goal, .. }) = self.queue.front() {
            if let Some(action) = crate::pokemon::postgame::fishing::pick_battle_action(state, goal, &actions) {
                return Some(action);
            }
        }

        // **Workstream I3/I4.** A wild battle during a `UseItemsInBattle` step is the whole point of
        // the step: spend the next unspent item in it. Placed here, before the flee/heal/catch
        // detours, because every one of them would abandon the battle the step went looking for —
        // and X items are `wIsInBattle`-gated, so there is no second chance out in the overworld.
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
                        // Everything spent and the battle is still running (no Poké Doll in the list,
                        // or it was declined) — leave rather than fight a battle nobody wanted.
                        if let Some(run) = actions.iter().find(|a| matches!(a, BattleAction::Run)) {
                            return Some(*run);
                        }
                    }
                }
            }
        }

        if self.heal_return.is_some() && battle_state.battle_type == BattleType::Wild {
            // returning to the pokemon center, run from battles.
            if let Some(center) = self.last_pokemon_center {
                println!("[policy] PP critically low — fleeing and routing to {center} to heal");
            }
            return Some(BattleAction::Run);
        }

        // ── Low-PP flee ──────────────────────────────────────────────────────
        // If every damaging move the active Pokémon has is at ≤10% of its max PP,
        // run from wild battles and queue a detour to the last visited Pokémon Center.
        if battle_state.battle_type == BattleType::Wild
            && self.heal_return.is_none()
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

        // If the active Pokémon is fainted (forced switch screen), send the
        // healthiest available party member.
        if battle_state.player.current_hp == 0 {
            return actions.iter()
                .filter(|a| matches!(a, BattleAction::SwitchPokemon { .. }))
                .max_by_key(|a| match a {
                    BattleAction::SwitchPokemon { pokemon, .. } => pokemon.current_hp,
                    _ => 0,
                })
                .cloned();
        }

        // ── Flee obstacle wilds during Victory Road boulder tasks / the HM-slave catch ───────
        // Cave wilds here are pure obstacles: fighting each one drains the lead's damaging-move PP over the
        // long multi-floor traversal (the 27-push 3F solve alone triggers dozens) until it Struggles itself
        // — and its team — into a black-out that boots the run to Viridian. Flee them instead: `EndOfBattle`
        // still grants the post-battle no-encounter grace on a run, so the boulder pushes complete between
        // encounters, and PP is preserved for the *mandatory* VR trainers. During a `CatchPokemon` step,
        // still engage the target species (it falls through to the catch-throw block below).
        if battle_state.battle_type == BattleType::Wild
            && actions.iter().any(|a| matches!(a, BattleAction::Run))
        {
            let flee = match self.queue.front() {
                Some(PolicyStep::CatchPokemon { .. }) | Some(PolicyStep::SweepDex { .. }) =>
                    self.catch_target(state, battle_state.enemy.species).is_none(),
                _ => in_center_less_dungeon,
            };
            if flee {
                return Some(BattleAction::Run);
            }
        }

        // Train a bench mon by switching it in so it — not the lead — earns the XP. Two sources:
        //   • a `GrindUntilLevel` step at the queue front → grind that slot (wild battles only), or
        //   • `train_slot` mode → that slot in *every* battle (wild + trainer gauntlet).
        // Only switch to a slot that's alive and not already active; if it has fainted, fight on with
        // whoever's out (the GrindUntilLevel arm / blackout recovery heals it). A level-safety cap
        // skips the switch when the enemy out-levels the trainee by >6, so training mode won't suicide
        // the trainee into a much stronger foe (e.g. the rival's ace) — the lead handles those.
        let is_grinding = matches!(self.queue.front(), Some(&PolicyStep::GrindUntilLevel { .. }));
        let train_slot = match self.queue.front() {
            Some(&PolicyStep::GrindUntilLevel { slot, .. }) if battle_state.battle_type == BattleType::Wild => Some(slot),
            _ => self.train_slot,
        };
        if let Some(slot) = train_slot {
            // ── Grind hand-off (prevents the trainee EVER fainting) ────────────────────────────
            // A weak, underlevelled trainee on high-XP wilds (a lv34 Vaporeon vs Route 23's lv40+ wilds)
            // gets out-sped and one-shot before a "heal/switch when low" reaction can fire — and a faint
            // deep in ledge-strewn Route 23 strands the faint-recovery trek and stalls the run. So on TURN 1
            // hand off immediately to the strongest healthy tank (Venusaur): the trainee led, so it is
            // flagged as a battle participant and earns the shared Gen-1 XP, and switching resolves before
            // the enemy attacks (Gen 1) — the trainee takes ZERO damage and never faints, while the tank
            // lands the KO. `trainee_participated` (reset each overworld tick) then stops train_slot
            // re-switching it in. Scoped to grinding so `train_slot` mode (mid-`full_playthrough`) is unaffected.
            // Only hand off when the wild could actually threaten the trainee (its level is within 6 of the
            // trainee's, or higher). Once the trainee out-levels the local wilds (a lv50 Vaporeon vs the
            // Mansion's lv30-39 wilds) it solos them safely for FULL XP — much faster than the halved
            // participation XP a hand-off gives.
            let enemy_threatens = battle_state.enemy.level + 6 >= battle_state.player.level;
            if is_grinding && battle_state.active_party_slot == slot && !self.trainee_participated
                && enemy_threatens
            {
                if let Some(sw) = actions.iter()
                    // Tank must out-level the trainee and be above the 25% heal threshold, so it stays a
                    // valid hand-off target between battles (it tops itself up via heal-at-25% + the free
                    // Viridian heal on the low-PP flee) rather than dropping out and re-exposing the trainee.
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

        // When catching, throw a Pokéball immediately if one is available. Two steps get here —
        // `CatchPokemon`, which wants one named species, and H5's `SweepDex`, which wants anything the
        // dex is missing — so the target test is `catch_target` rather than an equality.
        if let Some(ball) = self.catch_target(state, battle_state.enemy.species) {
            let species = &battle_state.enemy.species;
            // ⚠️ **Wild only.** A trainer's Pokémon can be an unowned species too, and the game answers
            // a ball with "the TRAINER blocked the BALL!" and no turn consumed — an infinite retry, not
            // a wasted turn. This guard is what stops a sweep dying to the first trainer whose line of
            // sight crosses the grass it is pacing in.
            if battle_state.battle_type == BattleType::Wild {
                // A step may pin its ball so an incidental catch doesn't spend the Master Ball; fall
                // back to the best in the bag if that ball has run out.
                let chosen = ball
                    .and_then(|id| state.bag.iter().find(|i| i.id == id && i.quantity > 0))
                    .or_else(|| state.bag.best_pokeball());
                if let Some(best_pokeball) = chosen {
                    if let Some(use_pokeball_action) = actions.iter()
                        .find(|a| matches!(a, BattleAction::UseItem { item, .. } if item.id == best_pokeball.id )) {

                        // Catch-rate-3 targets (the legendaries) are paralysed and then only thrown at
                        // — never weakened. See `postgame::legendaries::pre_catch_action`.
                        if let Some(action) = crate::pokemon::postgame::legendaries::pre_catch_action(state, *species, &actions, Some(use_pokeball_action)) {
                            return Some(action);
                        }

                        // If enemy HP > 50%, try to weaken it first with the move that does the most
                        // damage without knocking the Pokémon out — but NOT for a Master Ball (100%
                        // catch), and skip it when our attacker heavily out-levels the target (a weak
                        // HM-slave catch), where even a "safe" hit would KO it.
                        if battle_state.enemy.remaining_hp() > 0.5
                            && best_pokeball.id != ItemId::MasterBall
                            && battle_state.player.level < battle_state.enemy.level + 12 {
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

        // Use a healing item if HP is below 25% — prefer the BIGGEST heal available. This matters against
        // a fast, super-effective attacker (e.g. the Silph rival's Alakazam vs Venusaur): a Super Potion
        // (+50) only cancels its ~50 Psychic, so the mon never rises above 25% to attack and stalemates
        // forever. A Hyper/Max Potion heals to near-full, so the mon survives the next hit above the heal
        // threshold and gets to actually fight back.
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

        // Switch to the healthiest party member if below 15% HP and a better option exists.
        // Exception: while grinding we deliberately want the lead (slot 0) to take the wild-battle
        // XP, so switching a healthy bench mon in would starve the lead of levels and the grind
        // would never finish. During a `GrindUntilLevel` step, keep the lead in (blackout recovery
        // heals and resumes if it faints).
        // (Same for `train_slot` mode: keep the trainee in so it keeps earning XP.)
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
                    // Only switch to a member that is a *genuine* alternative: meaningfully healthy
                    // (>50% of its own max HP) AND at least the active mon's level. A low-level bench
                    // mon (e.g. a lv4 Pidgey behind a lv18 Ivysaur) is a sacrificial weakling — even
                    // at full HP it faints immediately, so swapping it into a trainer battle just
                    // hands over a Pokémon and stalls the run (observed: the Mt Moon Super Nerd fight
                    // never cleared, so the fossil was never collected). This mirrors the original
                    // lone-Ivysaur behaviour: fight on and rely on blackout recovery when there is no
                    // real switch, but take a strong, healthy team-mate when one exists.
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

        // Critically low HP with no way to recover in-battle — the <25% heal block above found no
        // usable potion and the <15% switch block above found no healthy team-mate to swap in — so
        // flee the wild battle and heal at a Pokémon Center instead of fighting on until the mon
        // faints. Fainting would force in a weak bench mon (e.g. the lv-4 Pidgey) that can't win yet,
        // being alive, blocks the black-out that would otherwise heal the party — deadlocking a
        // dungeon crossing (the lone starter attriting to a faint in Mt Moon). Fleeing to heal keeps
        // the starter, and it re-crosses the dungeon in successive, progressively-levelled passes
        // (the "just the starter" recovery loop). Skipped while grinding — there we deliberately fight
        // on and rely on black-out recovery so the lead keeps earning XP.
        if !grinding
            && !in_center_less_dungeon
            && battle_state.battle_type == BattleType::Wild
            && self.heal_return.is_none()
            && battle_state.player.remaining_hp() < 0.15
        {
            if let Some(center) = self.last_pokemon_center {
                println!("[policy] HP critical, no heal/switch — fleeing to {center} to heal");
                self.heal_return = Some(center);
                return Some(BattleAction::Run);
            }
        }

        // Elite-Four tactic: if the active mon can no longer hit the enemy hard (its best available move
        // does < 1/3 of the enemy's max HP — e.g. a Blizzard/Surf-dry Vaporeon left with weak Bite against
        // one of Lance's bulky dragons) but a healthy benched team-mate has a MUCH stronger move vs this
        // enemy, switch to it. This pools both mons' strong PP + super-effective coverage across the fight
        // instead of chipping to a slow non-finish. Guarded to a big damage jump (≥1.5×) so it never thrashes.
        if battle_state.battle_type == BattleType::Trainer {
            let move_dmg = |mon: &crate::pokemon::pokemon::PokemonSummary| -> u32 { mon.moves.iter().flatten()
                .filter_map(|m| expected_damage(mon, m.name, &battle_state.enemy).map(|d| d as u32))
                .max().unwrap_or(0) };
            let active_best = move_dmg(&battle_state.player);
            if (active_best * 3) < battle_state.enemy.stats.hp as u32 {
                let best_switch = actions.iter()
                    .filter(|a| matches!(a, BattleAction::SwitchPokemon { pokemon, .. }
                        if pokemon.current_hp as u32 * 2 > pokemon.stats.hp as u32
                        && pokemon.level + 8 >= battle_state.player.level))
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

        // 1. pick the strongest move.
        //
        // (A grind-only variant — prefer the *highest-PP* move that still one-shots, since the trainee
        // vastly out-levels what it farms and the Pokémon Center round trips cost far more than the
        // fights — was tried and reverted: it changes move choice in the Route-1 starter grind too, and
        // the RNG line that comes out of that is the one the fragile early game is tuned against. Worth
        // revisiting behind a flag if a dedicated grind ever needs it; `probe_grind_to_70` is fast
        // enough without it now that the 0-PP deadlock is fixed.)
        let result = pick_best_move(&battle_state, &actions, false);
        if result.is_some() {
            return result;
        }

        // No damaging move on the active Pokémon (out of PP, or all resisted to 0 damage).
        // Prefer switching to a party member that CAN damage the enemy, rather than spamming a
        // status move — especially Leech Seed, whose HP drain keeps the active Pokémon alive
        // indefinitely and deadlocks the whole battle (observed Nugget-Bridge stall).
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

        // No party member can damage the enemy. Avoid self-healing moves (Leech Seed) so the
        // battle actually resolves (faint → black-out recovery) instead of stalling forever.
        if let Some(a) = actions.iter()
            .filter(|a| matches!(a,
                BattleAction::Fight { battle_move, .. } if battle_move.name != PokemonMoveName::LeechSeed))
            .choose(&mut self.rng)
        {
            return Some(a.clone());
        }

        // Last resort: a Fight move (Struggle if truly out of PP), else any non-item action such as a
        // switch to a team-mate or Run. Never fall through to a `UseItem`: the first bag entry is often
        // an unusable key item, and selecting it deadlocks on "This isn't the time to use that!".
        actions.iter().find(|a| matches!(a, BattleAction::Fight { .. }))
            .or_else(|| actions.iter().find(|a| !matches!(a, BattleAction::UseItem { .. })))
            .or_else(|| actions.iter().find(|a| matches!(a, BattleAction::Fight { .. })))
            .cloned()
    }

    fn pick_nickname(&mut self, _species: PokemonSpecies) -> Option<Option<String>> {
        let name = self.name_picker.pick().to_string();
        println!("[policy] pick name={}", name);
        Some(Some(name))
    }

    fn pick_move_to_forget(&mut self, current_moves: &[PokemonMove], new_move: PokemonMoveName)
        -> Option<Option<usize>>
    {
        // Forget the *weakest* move rather than the default slot-0 (which was silently discarding
        // Tackle). Value = base power, with any damaging move ranked above every status move (the +1
        // tie-break also covers fixed-damage moves like Seismic Toss that list no power); HM moves are
        // given max value so they are never forgotten (needed for field use). Because status moves
        // rank lowest, a mixed moveset keeps its damaging moves — e.g. Ivysaur learning Poisonpowder
        // forgets Growl/Leech Seed, not Tackle or Vine Whip. We always learn into the weakest slot
        // (never decline) to avoid the fragile "abandon learning?" YES/NO flow; the only mild loss is
        // a Pokémon that already knows four damaging moves learning a weak one, which is rare.
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
                    // Popped on issue, like `MovePokemonToFront`: the driver owns the operation from
                    // here to completion and `pick_field_move` is not polled again until it is done,
                    // so leaving the step queued would only re-issue it forever.
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
            // Same hand-over as `UseItemPc` above, for the same reason: the box driver owns the walk
            // to the PC tile and the A press that opens it, and the step pops on issue.
            if state.map.map == map {
                self.queue.pop_front();
                return match crate::pokemon::tile_map::pc_locations_for(map).first() {
                    Some(&pc) => Some(FieldMove::UsePcBox { op, pc }),
                    None => { println!("[policy] UsePcBox: {map} has no PC — skipping"); None }
                };
            }
        }
        if let Some(&PolicyStep::PartyScript { script, slot }) = self.queue.front() {
            // **Workstream G.** Same hand-over shape as `UsePcBox`, and pops on issue for the same
            // reason — the driver owns the walk and the whole conversation.
            if state.map.map == script.map() {
                self.queue.pop_front();
                return crate::pokemon::postgame::gifts::pick(state, script, slot);
            }
        }
        if let Some(&PolicyStep::SellToMart { map, item }) = self.queue.front() {
            // **Workstream F.** Same hand-over as `UseItemPc`, and pops on issue for the same reason.
            if state.map.map == map {
                self.queue.pop_front();
                return crate::pokemon::postgame::game_corner::pick_sale(state, item);
            }
        }
        if let Some(&PolicyStep::RedeemPrize { prize }) = self.queue.front() {
            // **Workstream F.** The vendors are bg-events, not sprites, so the driver walks to the tile
            // below one and owns the A press — same shape as the PC.
            if state.map.map == Map::GameCornerPrizeRoom {
                self.queue.pop_front();
                return crate::pokemon::postgame::game_corner::pick_prize(state, prize);
            }
        }
        if let Some(&PolicyStep::Fish { rod, map, goal }) = self.queue.front() {
            // **Workstream C.** One cast per issue: the driver returns to `Idle` after each one (and a
            // bite hands the agent to the battle handler before that), so the repetition lives here.
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
            // **Workstream I.** Re-issued each time the driver returns to `Idle`, like `UseStrength`,
            // so an interruption costs a tick rather than the step — but bounded, because a use the
            // game silently declines would otherwise retry for the whole leg. `items::pick` refuses
            // the known-declinable cases up front; `MAX_ITEM_USE_ATTEMPTS` catches the rest.
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
            // **Workstream B.** Popped on issue like `MovePokemonToFront`: the driver owns everything
            // from the START menu to the landing, and `pick_field_move` is not polled again until it
            // returns to `Idle`. It refuses impossible flights itself (indoors, unvisited town, no Fly)
            // rather than making that this file's problem.
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
        if let Some(&PolicyStep::UseFlash { slot }) = self.queue.front() {
            // **Workstream H.** Same shape as `UseStrength` below: re-issued each tick until the
            // effect shows in RAM, so an interruption costs a tick rather than the step.
            if !state.map_is_dark {
                println!("[policy] UseFlash: {} is lit — done", state.map.map);
                self.queue.pop_front();
                return None;
            }
            return Some(FieldMove::UseFieldMove { slot, move_index: field_move_index(state, slot, PokemonMoveName::Flash) });
        }
        if let Some(&PolicyStep::UseStrength { target }) = self.queue.front() {
            if state.strength_active {
                println!("[policy] UseStrength: BIT_STRENGTH_ACTIVE set — done");
                self.queue.pop_front();
                return None;
            }
            let Some(slot) = target.resolve(state) else {
                println!("[policy] UseStrength: {target:?} is not in the party — waiting");
                return None;
            };
            return Some(FieldMove::UseFieldMove { slot, move_index: field_move_index(state, slot, PokemonMoveName::Strength) });
        }
        if let Some(&PolicyStep::SolveBoulders { switch }) = self.queue.front() {
            // Done once a boulder sits on the switch (the map script then opens the barrier).
            let boulder_on_switch = state.map.sprites.iter()
                .any(|s| s.name.starts_with("Boulder") && !s.hidden && s.position == switch);
            if boulder_on_switch {
                println!("[policy] SolveBoulders: boulder on switch {switch} — done");
                self.queue.pop_front();
                return None;
            }
            // Plan the Sokoban to `switch` and emit the FIRST push; the agent executes one push, then
            // this re-plans from the new positions next tick (resumes cleanly after any interruption).
            return Self::next_boulder_push(state, switch);
        }
        if let Some(&PolicyStep::DropBoulderInHole { hole }) = self.queue.front() {
            // Count visible boulders on this floor; the step is done once one has fallen (count drops).
            let visible = state.map.sprites.iter()
                .filter(|s| s.name.starts_with("Boulder") && !s.hidden).count();
            let baseline = *self.boulder_drop_baseline.get_or_insert(visible);
            if visible < baseline {
                println!("[policy] DropBoulderInHole: a boulder fell into {hole} — done");
                self.boulder_drop_baseline = None;
                self.queue.pop_front();
                return None;
            }
            // Plan toward the hole tile (the solver accepts a hole as a push target) and emit one push.
            return Self::next_boulder_push(state, hole);
        }
        if let Some(&PolicyStep::TeachMove { item, target }) = self.queue.front() {
            // Resolve every tick, not once: a `Species` target may still be a Poké Ball on the floor
            // when the step reaches the front of the queue (the Celadon gift Eevee is), and the party
            // it indexes into is re-read here anyway.
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
            // A TM that was never picked up cannot be taught, and the menu driver would loop forever
            // looking for it in the bag. (HMs are never consumed, so a missing HM means it genuinely
            // was not collected either — the same conclusion.) Skip rather than stall.
            if !state.bag.iter().any(|b| b.id == item) {
                println!("[policy] TeachMove: {item:?} is not in the bag — skipping");
                self.queue.pop_front();
                return None;
            }
            let Some(target_slot) = resolved else {
                println!("[policy] TeachMove: {target:?} is not in the party — waiting");
                return None;
            };
            return Some(FieldMove::TeachMove { item, target_slot });
        }
        if let Some(&PolicyStep::UseRareCandy { slot }) = self.queue.front() {
            // Done once the Rare Candy is gone (consumed). Reuse the teach-move menu driver: Rare Candy
            // teaches no HM move, so its "learned" completion never fires — the agent uses the item,
            // then backs out when the item disappears from the bag list. Popping here on consumption
            // guarantees exactly one use.
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
        if let Some(&PolicyStep::Dig { slot }) = self.queue.front() {
            // Done when Dig has warped us off this map. The map we started on is remembered on the first
            // tick, so a re-issue after an interruption still terminates.
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
            return Some(FieldMove::UseFieldMove { slot, move_index: field_move_index(state, slot, PokemonMoveName::Dig) });
        }
        if let Some(&PolicyStep::EvolveWithStone { stone, target }) = self.queue.front() {
            // "Evolved" = the species we started against is no longer at the target. Captured as a
            // baseline on the first tick rather than compared to a hardcoded input mon, so this works
            // for any stone-evolver and for a `Slot` target as well as a `Species` one.
            let current = target.resolve(state).and_then(|slot| state.pokemon.get(slot as usize))
                .map(|p| p.species);
            let Some(current) = current else {
                // A `Species` target that no longer resolves has itself evolved away; a `Slot` target
                // that does not resolve is off the end of the party and nothing can be done with it.
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
                    // Pokémon Mansion: one global switch toggles every floor's gates. Flip it exactly
                    // once — complete when `mansion_switch_on` differs from the value captured when the
                    // step began — so the route can compose deterministic flips (vs. an oscillating
                    // "flip until a warp appears" loop, which the single global toggle makes unstable).
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
                // Non-Mansion (Rocket Hideout poster): done once the passage to `reveals` opens. The
                // staircase isn't a gate the runtime block map exposes, so use its event flag.
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
        if let Some(&PolicyStep::SearchHiddenItem { map, item }) = self.queue.front() {
            // **Workstream H.** Baseline the bag *now*, before the first A press, so "collected" is
            // "the count went up" rather than "the item is present" — the bag may already hold a
            // stack of the same thing.
            if state.map.map == map {
                use crate::pokemon::postgame::aides;
                let baseline = *self.hidden_item_baseline
                    .get_or_insert_with(|| aides::bag_quantity(state, item));
                match aides::pick(state, item, baseline) {
                    Some(field_move) => return Some(field_move),
                    None => {
                        println!("[policy] SearchHiddenItem: {item:?} collected on {map} — done");
                        self.hidden_item_baseline = None;
                        self.queue.pop_front();
                        return None;
                    }
                }
            }
        }
        if let Some(&PolicyStep::UseElevator { panel, floor }) = self.queue.front() {
            // The step completes once we've ridden the elevator out to another floor — i.e. once we're
            // no longer standing in an elevator room. (Any of the game's elevators, not just Rocket
            // Hideout's — Silph Co and Celadon Mart have them too.)
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
            // Done once the target has been seen and is now gone (the item's effect — e.g. waking then
            // defeating the Snorlax — removed it).
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
            // Reuse the face-a-bg-event-and-press-A mechanism; the vending menu opens with the cheapest
            // drink at the cursor, so A-mashing buys it. Persists until the drink is in the bag.
            return Some(FieldMove::CheckTrashCan { target: at, facing: None });
        }
        None
    }

    fn pick_mart_purchase(&mut self, _state: &GameState) -> Option<Option<BagItem>> {
        let result = match self.queue.front() {
            Some(PolicyStep::BuyFromMart { item, .. }) => {
                // Count this shop-open as an attempt. The `BuyFromMart` overworld arm pops the step
                // once the bag reflects the purchase (or after MAX_MART_ATTEMPTS), so we do NOT pop
                // here — a dropped YES-confirm re-opens the shop and retries. The quantity here is the
                // one the step *asked* for; the agent trims it to what the wallet can cover (see
                // `AgentState::PokemartShopping`), because Gen 1 answers an unaffordable quantity by
                // selling nothing at all.
                self.mart_attempts += 1;
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

    fn steps_remaining(&self) -> Option<usize> {
        Some(self.queue.len())
    }

    fn current_step_is_long_running(&self) -> bool {
        matches!(
            self.queue.front(),
            Some(PolicyStep::GrindUntilLevel { .. })
                | Some(PolicyStep::CatchPokemon { .. })
                // H5's sweep is one step per map and stays on it for every species that map owes —
                // dozens of encounters, most of them fled.
                | Some(PolicyStep::SweepDex { .. })
                // Collecting the Mt Moon fossil means crossing a battle-heavy floor: each wild
                // encounter interrupts the walk, and with a real (non-pimped) party those battles
                // are slow, so the single CollectItem step legitimately sits for a long while.
                | Some(PolicyStep::CollectItem(_))
                // A gym-leader fight sits on one step for the whole battle, and self-heals + re-routes
                // on a blackout (queue unchanged the whole time) — legitimately long-running.
                | Some(PolicyStep::DefeatGymLeader { .. })
                // An Elite-Four fight is a long, multi-Pokémon battle with heavy Full-Restore healing
                // (Lance's 6 dragons vs a 5-PP Blizzard can run many dozens of turns) — the single
                // BattleTrainer step legitimately sits unchanged well past the 10-minute stall window.
                | Some(PolicyStep::BattleTrainer { .. })
                // Flipping a Pokémon Mansion switch means routing across a battle-heavy floor to reach
                // the statue (wild encounters + LOS trainers interrupt the walk) — the single FlipSwitch
                // step legitimately sits unchanged for a long while.
                | Some(PolicyStep::FlipSwitch { .. })
                // A fishing session is one step across many casts and the battles they start — a
                // `Catch` goal against a two-species table routinely runs dozens of casts deep
                // (workstream C).
                | Some(PolicyStep::Fish { .. })
                // A Safari hunt is one step across every encounter of every ¥500 trip, and a trip that
                // spends its whole 502-step budget without meeting a target is an ordinary outcome
                // (workstream E).
                | Some(PolicyStep::SafariHunt { .. })
                // Walking out of the west is four map transitions on one step, and an ejection part
                // way through restarts it from the gate — legitimately longer than the stall window.
                | Some(PolicyStep::SafariExit)
                // **I3/I4** — one step covers pacing for a wild encounter *and* the eight-turn battle
                // it exists for, with the queue frozen throughout.
                | Some(PolicyStep::UseItemsInBattle { .. })
                // **L** — this step **carries its own bound** (`MAX_ENTER_WAIT` attempts), so the
                // harness's 10-minute stall window is redundant and, on a route, wrong: walking out
                // onto Route 12 and back is one poll and several minutes of game time, so a handful
                // of legitimate attempts can outlast the window. The tour's real failsafe is the
                // test's cycle budget.
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
        // Ivysaur: [Tackle(dmg), Growl(status), LeechSeed(status), VineWhip(dmg)] learning Poisonpowder.
        let moves = [mv(Tackle), mv(Growl), mv(LeechSeed), mv(VineWhip)];
        let slot = p.pick_move_to_forget(&moves, Poisonpowder).flatten().expect("should pick a slot");
        assert!(slot == 1 || slot == 2,
            "forgot slot {slot} ({:?}) — must forget a status move, not Tackle/Vine Whip", moves[slot].name);
    }

    #[test]
    fn learns_strong_move_over_status() {
        let mut p = DeterministicPolicy::new(0, Vec::<PolicyStep>::new());
        let moves = [mv(Tackle), mv(Growl), mv(LeechSeed), mv(VineWhip)];
        // Learning Razor Leaf (strong) should still forget a status slot, keeping both damaging moves.
        let slot = p.pick_move_to_forget(&moves, RazorLeaf).flatten().unwrap();
        assert!(slot == 1 || slot == 2, "should forget a status move to learn Razor Leaf");
    }

    #[test]
    fn never_forgets_hm() {
        let mut p = DeterministicPolicy::new(0, Vec::<PolicyStep>::new());
        let moves = [mv(Cut), mv(Growl), mv(LeechSeed), mv(Poisonpowder)];
        let slot = p.pick_move_to_forget(&moves, Poisonpowder).flatten().unwrap();
        assert_ne!(moves[slot].name, Cut, "must never forget an HM move (Cut)");
    }
}

#[cfg(test)]
mod policy_helper_tests {
    use super::*;

    /// ⚠️ **Every name any policy can hand the game has to be one the game will actually take**:
    /// never empty (the cartridge's own name screen refuses one), never longer than
    /// [`MAX_PLAYER_NAME`], and made only of characters the charmap has a glyph for.
    ///
    /// ⚠️ **That last check is the point, and it is not "is it alphanumeric".** What matters is that
    /// nothing falls through [`PokemonString::from_string`] to a `0x00`, which the game draws as a
    /// blank — so the assertion is made by encoding the name rather than by inspecting it. This
    /// replaces a narrower test that only ever saw the names derived from `GB_MODEL`; those are gone
    /// (`LlmPolicy` is always `AI` now), and the three lists that remain had no guard at all.
    ///
    /// ⚠️ It must also not collide with `DebugNewGamePlayerName` (`NINTEN`), which
    /// `PokemonApiTrait::game_mode` compares `wPlayerName` against to decide the intro is still up:
    /// a policy that named the trainer that would leave the agent permanently "not in game".
    #[test]
    fn every_name_a_policy_can_choose_is_one_the_game_will_take() {
        let mut names: Vec<String> = RANDOM_NAMES.iter().map(|n| n.to_string()).collect();
        names.push("HUMAN".to_string());
        #[cfg(feature = "llm")]
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

    /// The Power Plant numbers its disguised Poké Balls, and an exact name match finds none of them —
    /// which presents as `CatchPokemon` pacing a map that has no wild encounters at all.
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
