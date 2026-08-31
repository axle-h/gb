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

/// The one [`Policy::name`] that means a model is playing.
///
/// ⚠️ **`run::RunMeta::model` holds the *policy's* name under every other policy** — `random`,
/// `scripted` — because a run directory always records something for it. So "is that string a model
/// id?" is answered by asking the policy, and this is the string both askers compare against and
/// `LlmPolicy::name` returns. A list of names to *exclude* instead is a list a fourth policy joins
/// silently, which is how `scripted` would have reached the leaderboard as if it were a model.
///
/// A free constant rather than an associated one: an associated const on `Policy` costs the trait
/// its dyn compatibility, and every holder of a policy in this crate holds a `Box<dyn Policy>`.
pub const LLM_POLICY_NAME: &str = "llm";

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
    /// It is also what decides whether the run has a *model* to name at all — see
    /// [`LLM_POLICY_NAME`] and the two readers of it in [`crate::host`].
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
    fn pick_mart_purchase(&mut self, state: &GameState) -> Option<Option<BagItem>> {
        Some(None) // default: open the mart but buy nothing
    }

    /// Another purchase for the **same** mart visit, asked once the previous one has gone through
    /// and the Buy/Sell/Quit menu is back on screen. `None` closes the shop, which is what every
    /// policy but [`LlmPolicy`](crate::pokemon::llm_policy::LlmPolicy) does.
    ///
    /// ⚠️ **A defaulted second method rather than a list out of `pick_mart_purchase`, and the reason
    /// is [`DeterministicPolicy`].** That policy does *not* pop its `BuyFromMart` step here — the
    /// overworld arm pops it once the bag reflects the purchase, so a dropped YES-confirm re-opens
    /// the shop and retries. Re-asking the existing method would therefore hand back the item just
    /// bought and buy it a second time, silently, in every scripted leg that shops. Defaulting to
    /// `None` means the scripted policies keep exactly the behaviour they have.
    ///
    /// ⚠️ **The quantity is not trimmed to the wallet here**, for the same reason the first purchase
    /// is not: `drive_pokemart` does it against the ROM's own price table, because Gen 1 hands over
    /// nothing at all for an order it cannot afford.
    fn next_mart_purchase(&mut self) -> Option<BagItem> {
        None
    }

    /// Called on the level-up "Which move should be forgotten?" prompt, when a Pokémon that already
    /// knows 4 moves would learn `new_move`. `current_moves` are the 4 known moves (slot order).
    ///
    /// ⚠️ **`party_slot` is which Pokémon is doing the learning, and the prompt could not be
    /// written without it.** The turn used to open "A Pokémon is trying to learn Surf" — the
    /// indefinite article is not a style choice, it was the whole of what was known. Identifying it
    /// meant matching four move names against the party by eye, and the decision on the table is
    /// which of *this* mon's moves to lose. Every caller already has the index: `drive_forget_menu`
    /// reads it from `learning_pokemon_index` one line above the call.
    ///
    /// - `None`             → not ready yet; asked again next frame.
    /// - `Some(None)`       → decline learning; keep the current four moves.
    /// - `Some(Some(slot))` → forget the move in `slot` (0-3) and learn `new_move`.
    fn pick_move_to_forget(
        &mut self,
        _party_slot: usize,
        _current_moves: &[PokemonMove],
        _new_move: PokemonMoveName,
    ) -> Option<Option<usize>> {
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


/// Whether a step can be interrupted by a walk to a Pokémon Centre and still finish afterwards.
///
/// ⚠️ **`Map::is_overworld` is the safe answer and it is too narrow, because the fights that need a
/// heal most are indoors.** Koga's gym is six trainers and an invisible-wall maze with no Centre in
/// it: the run walked in, was poisoned by the second trainer, ticked down through the third and
/// black-ed out on a junior's Hypno — one warp from the Fuchsia Centre it was standing next to ten
/// steps earlier. What makes that detour safe is not the map, it is the *step*: these all re-derive
/// their own route every tick, so leaving and coming back costs a walk and nothing else.
///
/// ⚠️ **Everything else stays out, and the exclusions are the point.** `EnterMap` is a deliberate
/// single hop — a chain of them through Mt Moon, Silph Co or the Seafoam Islands cannot be resumed
/// from a Pokémon Centre, which is how a black-out on a cave floor strands a run for good — and
/// `BattleTrainer`, `SolveBoulders` and the pad-maze steps are the same shape.
fn step_finds_its_own_way_back(step: &PolicyStep) -> bool {
    matches!(step,
        PolicyStep::GrindUntilLevel { .. } | PolicyStep::CatchPokemon { .. }
        | PolicyStep::SweepDex { .. } | PolicyStep::Goto { .. }
        | PolicyStep::DefeatGymLeader { .. } | PolicyStep::CutTree { .. })
}

/// Whether the run should walk to a Pokémon Centre *before* the next battle rather than after it.
///
/// ⚠️ **A black-out is the same walk with the levels kept and the money halved, and the route used to
/// take it seven times a run.** Every early black-out measured on the Squirtle route was the same
/// shape: the lead goes into an encounter already worn from the last one, loses in one or two turns,
/// and the party wakes in a Centre anyway. Nothing was *wrong* — the in-battle "heal below 25%" arm
/// fires correctly, it simply had nothing in the bag to reach for, and the flee-to-heal arm cannot
/// fire in a trainer battle at all. So the decision moves out of the battle and into the overworld
/// tick before it.
///
/// Three reasons to go, and they are deliberately different:
/// * the lead is **fainted** — nothing else in this file revives it outside a grind;
/// * its attacks are **spent**, which no purchase can fix: no Gen 1 mart sells Ether or Elixer
///   (`data/items/marts.asm`), so a Centre is the only PP in the game before the S.S. Anne's floor
///   items;
/// * it is **badly hurt and the bag is empty of medicine** — with a potion in the bag the in-battle
///   arm is the cheaper answer and this stays quiet.
fn needs_a_centre(state: &GameState, grinding: bool) -> bool {
    let Some(lead) = state.pokemon.get(0) else { return false };
    if lead.current_hp == 0 { return true; }

    // ⚠️ **A grind goes home on empty, not on low, and the difference is nineteen minutes.** Out on
    // the route a fifth of a tank is the right moment to turn back, because the next fight is a
    // trainer who cannot be fled. Inside the gauntlet grind every wild *can* be fled and the trainee
    // is handed off to a tank whenever one threatens it, so turning back at a fifth cost **46 round
    // trips** from the Pokémon Mansion to Cinnabar and back — measured, `hall_of_fame_playthrough`
    // went from 31 minutes to 51 on 5% more battles.
    let (pp, max) = lead.moves.iter().flatten()
        .filter(|m| is_damaging_move(m.name))
        .fold((0u32, 0u32), |(have, cap), m| (have + m.pp as u32, cap + m.name.metadata().pp as u32));
    let dry = match grinding { true => pp == 0, false => pp * 5 <= max };
    if max > 0 && dry { return true; }

    // ⚠️ **"Is there medicine?" is the wrong question — "can it fix this?" is the right one.** A
    // Potion is +20, which is a fifth of a lv21 Wartortle and none of a lv50 Blastoise, so a bag
    // holding nothing but Potions kept this quiet while the in-battle arm chipped away and the run
    // black-ed out on Route 3 anyway. The test is whether the best thing in the bag covers what is
    // missing.
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

    // ⚠️ **Every move at zero PP is Struggle, not "no move", and reading it as no move stops the run
    // dead in silence.** `available_battle_moves` filters on `pp > 0`, so a mon that has run dry
    // offers no `Fight` at all — and in a *trainer* battle there is no `Run` either, and a party with
    // nothing else conscious offers no `SwitchPokemon`, so the whole list is bag items.
    // `pick_battle_action`'s last resort deliberately refuses to fall through to a `UseItem` (the
    // first bag entry is often a key item the game will not use, which deadlocks on "This isn't the
    // time to use that!"), so it answered `None` — and `None` from a policy means *still thinking*.
    //
    // ⚠️ **The watchdog cannot see that.** `since_last_policy_poll` is reset by `poll_policy` whatever
    // the answer is, so a policy asked every tick and answering nothing every tick looks perfectly
    // healthy: the agent sits at the main battle menu, the emulator runs and nothing is printed.
    // Three `full_playthrough` runs died in that silence — twice against Erika's Victreebel and once
    // her Vileplume, once for **seven hundred minutes of game time**. The cartridge's own answer is
    // that FIGHT with no PP anywhere uses Struggle, which resolves the battle one way or the other,
    // so offer the moves and let it. Guard: `a_party_with_no_pp_anywhere_still_gets_an_answer`, which
    // asserts on **battle actions taken** rather than on silence, for the reason above.
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
    /// The first member of **any** of these species — an evolution line named as one mon.
    ///
    /// ⚠️ **A grind across an evolution needs this and `Species` cannot do it.** `GrindUntilLevel`
    /// checks the level of the same index it trains, so a target that stops resolving part-way
    /// through never completes: an Abra caught at lv10 and trained to 16 *becomes a Kadabra* on the
    /// level that finishes the step, and `Species(Abra)` then answers `None` for ever. The starter is
    /// the same shape and got away with it only because nothing addressed it by species until after
    /// both evolutions.
    Line(&'static [PokemonSpecies]),
}

impl PartyRef {
    /// The party index this reference currently points at, or `None` if the party holds no such mon.
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
    /// ⚠️ **`PartyRef`, not a slot, and that is the `machop_slot` lesson again.** A grind late in
    /// the run happens after several `MovePokemonToFront` rotations and two catches, so the index a
    /// route was written against is not the index it executes against — and this step's completion
    /// check reads the *same* index it trains, so a wrong one trains nothing and never finishes.
    GrindUntilLevel { target_level: u8, on_map: Map, target: PartyRef },
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
    Dig { target: PartyRef },
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

/// The party member that knows `want`, and `want`'s row in that member's field-move box.
///
/// ⚠️ **A field move is answered by whoever in the party knows it, never by a slot the caller
/// guessed — and both halves of that used to be assumed.** `CuttingTree` drove the party menu onto
/// **slot 0** unconditionally, which is right only while the starter is the Cut carrier; a starter
/// that cannot learn Cut (Squirtle: `data/pokemon/base_stats/wartortle.asm` lists SURF and STRENGTH
/// and no CUT) needs an HM slave, and the driver would then have opened the menu on the wrong mon
/// for ever, since its only exit is the overworld coming back. `Surfing` hard-coded the *move index*
/// to 0, which is right only while the surfer knows exactly one field move; a Blastoise carrying
/// Surf, Strength and Dig lists three, and index 0 is whichever sits earliest in its move list
/// rather than the one asked for.
///
/// So every path that uses a field move goes through this: find the move, then drive the menus into
/// that Pokémon and that row. The first holder wins — a party with two Surfers has no interesting
/// choice to make.
pub(crate) fn field_move_carrier(
    state: &GameState,
    want: PokemonMoveName,
) -> Option<(u8, u8)> {
    state.pokemon.iter().enumerate()
        .find(|(_, p)| p.moves.iter().flatten().any(|m| m.name == want))
        .map(|(i, p)| (i as u8, field_move_index_of(p, want)))
}

/// `want`'s row in `mon`'s field-move box, for callers that hold the mon but not a whole `GameState`.
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
        // Bow: SSAnne2F → SSAnne3F → SSAnneBow (one open room, two sailors). Party is strong by now.
        s.extend([
            Self::enter(Map::SSAnne3F), Self::enter(Map::SSAnneBow),
            Self::Interact(MapSprite::SSANNEBOW_SAILOR2), Self::Interact(MapSprite::SSANNEBOW_SAILOR3),
            Self::enter(Map::SSAnne3F), Self::enter(Map::SSAnne2F),
        ]);
        s.extend([Self::enter(Map::SSAnne1F), Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity), Self::enter(Map::VermilionPokecenter), Self::Interact(MapSprite::VERMILIONPOKECENTER_NURSE), Self::enter(Map::VermilionCity), ]); // disembark + heal

        // ── Rival + Captain (HM01) ── (heal first — the rival is 6 Pokémon in one battle)
        s.extend(Self::heal_at_vermilion());
        s.extend([Self::enter(Map::VermilionDock), Self::enter(Map::SSAnne1F), Self::enter(Map::SSAnne2F)]);
        s.push(Self::enter(Map::SSAnneCaptainsRoom)); // rival battle triggers on approach to the (36,4) warp
        s.extend(std::iter::repeat(Self::Interact(MapSprite::SSANNECAPTAINSROOM_CAPTAIN)).take(4));
        // ── Disembark back to Vermilion (after HM01 the ship departs on the way out of the dock) ──
        s.extend([
            Self::enter(Map::SSAnne2F), Self::enter(Map::SSAnne1F),
            Self::enter(Map::VermilionDock), Self::enter(Map::VermilionCity),
            Self::enter(Map::VermilionPokecenter), Self::Interact(MapSprite::VERMILIONPOKECENTER_NURSE), Self::enter(Map::VermilionCity), 
        ]);
        s
    }

    /// From Cerulean City (no badge needed): cross the Nugget Bridge, fetch the **SS Ticket** from
    /// Bill, come back and **beat Misty**, then cross to Vermilion City via the **trashed-house
    /// terrace bridge** + Underground Path (Route 5 → 6), catching the Cut carrier on the way. The
    /// trashed house is the only way between Cerulean's split terraces: its back door lands in the
    /// Route-5 terrace (`enter_at(CeruleanCity, 27, 9)` — front door ~27,11 does not reach it). See
    /// `can_reach_vermilion`. Bill's guard on Route 25 clears once you meet him, opening the bridge.
    pub fn cerulean_to_vermilion_steps() -> Vec<Self> {
        let mut steps = vec![
            Self::enter(Map::CeruleanCity),
            // Poké Balls for the two catches this route now depends on: the Cut carrier on Route 25
            // below, and the Drowzee on Route 11 in `saffron_to_cinnabar_steps`. Bought here rather
            // than after the bridge because every black-out halves the wallet and the bridge is where
            // they happen. `agent::affordable` trims the order to what the money covers.
            Self::enter(Map::CeruleanMart),
            Self::BuyFromMart { item: BagItem::new(ItemId::PokeBall, 6), map: Map::CeruleanMart },
            // Top the Potions back up before the nine trainers on the bridge — the same argument as
            // the Pewter stop, at the last counter before them. Ordered *after* the balls so that
            // when the wallet runs short it is the potions that thin out, not the catch.
            Self::BuyFromMart { item: BagItem::new(ItemId::Potion, 10), map: Map::CeruleanMart },
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::Route24),
            Self::enter(Map::Route25),
            // ⚠️ **Back across the bridge to a Centre before Bill, because what runs out here is PP.**
            // Route 24 is five trainers in a row and Route 25 four more, and this leg used to fight all
            // of them, Bill, the Underground Path and Route 6 on a single tank. The trainers are beaten
            // by now, so the walk back over the bridge is only a walk.
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
            // ── The Cut carrier. See `CUT_SLAVE`: Blastoise cannot learn Cut and this route needs it
            // four times. ⚠️ **Route 25 and not Route 5, which is where this was first written and
            // where it stalled for the rest of the run.** Route 5 has an Oddish in its table, but the
            // strip the route walks is the plain path down from Cerulean, and `wander_action` paces
            // whatever open tiles are nearest — measured, 57 s of game time a lap with no encounter at
            // all, for four thousand log lines. Route 25 is grass end to end and the run walks its
            // whole length anyway.
            Self::CatchPokemon { species: PokemonSpecies::Oddish, on_map: Map::Route25,
                                 ball: Some(ItemId::PokeBall) },
            Self::enter(Map::Route24),
            // ⚠️ **Misty is fought *after* the bridge, and the reorder is the whole of how this route
            // affords Bite.** Wartortle's Water Gun is resisted by both of her Water types and hers is
            // resisted right back, which is a slugfest it loses; **Bite at lv24 is Normal in Gen 1**,
            // so it is neutral into Starmie and the fight stops being close. Getting to 24 by grinding
            // is the expensive way to buy it — Route 3's wilds are lv3-8 Pidgey and Spearow, about
            // **170 battles** from 18 — where Route 24 and 25's nine trainers hand over most of it for
            // a walk the route was making anyway. Nothing on this side of Cerulean needs the Cascade
            // Badge: the bridge, Bill, the trashed house and the Underground Path are all open from
            // the start, and the first thing that does need it is Cut, in Vermilion, two legs later.
            //
            // Whatever the trainers left short is topped up here rather than earlier, because Route 24
            // is a far better site than Route 3 for it (lv12-14 Oddish, Abra and Pidgey against lv3-8).
            Self::GrindUntilLevel { target_level: 24, on_map: Map::Route24, target: Self::STARTER_LINE },
            Self::enter(Map::CeruleanCity),
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
            Self::enter(Map::CeruleanCity),
            Self::DefeatGymLeader { leader: MapSprite::CERULEANGYM_MISTY, badge: Badge::CascadeBadge },
            // Exit the gym to the city (a single warp) before entering the Pokécenter.
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

    /// Thunder Badge (from Vermilion City after the S.S. Anne, with **HM01 Cut** in the bag): teach Cut
    /// to the starter, cut the tree sealing the gym enclosure, solve the two-switch **trash-can
    /// puzzle** (which unlocks the door), then beat Lt. Surge. All via the real UI — see
    /// `can_get_thunder_badge` (integrated) and `can_teach_cut` / `can_cut_gym_tree` /
    /// `can_beat_lt_surge` (focused). `SolveTrashCans` must precede `DefeatGymLeader`: the door is
    /// shut (Surge unreachable) until both switches are hit.
    pub fn thunder_badge_steps() -> Vec<Self> {
        let mut s = Self::heal_at_vermilion();
        s.extend([
            // ⚠️ **The carrier, not the lead.** `CuttingTree` used to drive the party menu onto slot
            // 0 unconditionally, which was right only while the starter was the Cut holder;
            // `agent::field_move_carrier` now resolves whoever knows it, so the HM can live on the
            // slave and the lead can stay the thing that fights.
            Self::TeachMove { item: ItemId::Hm01Cut, target: Self::CUT_SLAVE },
            // ⚠️ **Dig goes on before Lt. Surge, not after him, and it is the difference between
            // losing that gym twice and walking it.** Surge is Electric and the starter is Water, so
            // every one of his attacks is 2× and every one of the starter's is neutral at best —
            // measured, the run black-ed out in that gym **twice in a row at lv33**. TM28 is
            // 100-power **Ground**, which Electric takes 2× from, and Dig's first turn is spent
            // underground where a Thunderbolt cannot reach. The run has been carrying the TM since
            // the Rocket outside the Cerulean trashed house handed it back
            // (`scripts/CeruleanCity.asm`, `.beatRocketThief`), two legs ago.
            Self::TeachMove { item: ItemId::Tm28Dig, target: Self::STARTER_LINE },
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
            // ⚠️ **Heal on the way in, because the gym is the last chance.** The garden's junior
            // trainers engage by line of sight while `CutTree` is clearing the maze, so whatever HP
            // and PP the party walks in with is what it fights Erika on — and the first Squirtle run
            // reached her at 86/118 and fell asleep.
            Self::enter(Map::CeladonPokecenter),
            Self::Interact(MapSprite::CELADONPOKECENTER_NURSE),
            Self::enter(Map::CeladonCity),
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
            // Straight onto the starter, which is what retired the Eevee → Vaporeon leg: Blastoise
            // learns Surf and Strength itself (`data/pokemon/base_stats/blastoise.asm`), so the only
            // thing that leg was still buying was a second body to hang the HMs on.
            Self::TeachMove { item: ItemId::Hm03Surf, target: Self::STARTER_LINE },
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
            // ⚠️ **HM04 stays in the bag.** Strength is 80-power Normal that the starter never wants
            // to use, and an HM is the one move `pick_move_to_forget` will not drop — so teaching it
            // here costs a permanent slot in the only Pokémon this route fights with. It goes on the
            // Victory Road Machop instead, caught two tiles from the boulder it is for. Surf is the
            // exception because Surf is a 95-power STAB attack that happens to be an HM.
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
            // ⚠️ **Heal first, because Silph Co is the longest Pokémon-Centre-less stretch in the
            // game and what runs out in it is PP.** Eleven floors of Rockets, the rival and Giovanni
            // are fought on whatever the party walks in with, and the Hyper Potions below restore HP
            // and nothing else. Measured: the first Squirtle run lost Blastoise somewhere in the
            // middle of the building and finished the rival's Venusaur with a **lv19 Oddish using
            // Cut**, then blacked out on 11F one room from Giovanni — which warps to Celadon, where
            // the queue's next step is a Silph floor it cannot reach, and the run is over.
            Self::enter(Map::SaffronPokecenter),
            Self::Interact(MapSprite::SAFFRONPOKECENTER_NURSE),
            Self::enter(Map::SaffronCity),
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
            // ⚠️ **Free two bag slots before reaching for the Card Key.** The run arrives at Silph on
            // exactly **20 entries**, Gen 1's cap, and a full bag refuses every pickup in the game
            // *silently* — the step simply never completes. Measured: 3000 polls standing one tile
            // from the key, then a give-up, and the run walked on without the thing the rest of the
            // building is locked behind. The Nugget is sell-fodder this route never sells and TM34
            // Bide is the deadest weight it carries; the Bide toss used to be two legs later, for the
            // Master Ball, and that was always the wrong side of the door.
            Self::TossItem { item: ItemId::Nugget },
            Self::TossItem { item: ItemId::Tm34Bide },
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
            // ── Out, heal, and back in before Giovanni ────────────────────────────────────────
            // ⚠️ **Silph Co has no Pokémon Centre and its two hardest fights are back to back**: the
            // rival's six mons, then Giovanni one pad away. The route heals before every other fight
            // of that size and could not before this one, and it shows — the leg chain lost
            // Giovanni's Kangaskhan twice on a Blastoise that came out of the rival at half HP, and a
            // black-out on 11F is the worst place in the game to have one.
            //
            // The way out is the way in, reversed: 7F(5,3) is the same pad that brought us from
            // 3F(11,11), and 3F has the elevator. ⚠️ **Every hop is `IfReachable` and the elevator
            // steps pop when they are not in one**, so on any floor plan where this cannot be walked
            // the whole detour evaporates and the run goes straight to Giovanni exactly as before.
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
            // ⚠️ **A second slot, because the Card Key spent the one the first toss freed.** TM34 Bide
            // used to be tossed here and is now gone two legs earlier (the bag runs out at the Card
            // Key first), so this needs its own: TM11 Bubblebeam is a 65-power Water move on a party
            // whose only fighter has Surf, and it is never taught. Without it the President's speech
            // ends in "You have no room for this.", the `Interact` completes looking exactly like a
            // success, and the failure surfaces at the Seafoam Islands as "no Pokéballs left".
            Self::TossItem { item: ItemId::Tm11Bubblebeam },
            Self::Interact(MS::SILPHCO11F_SILPH_PRESIDENT),             // Master Ball + Rockets leave Saffron
            // ⚠️ **The way *out* gives up rather than stalling, because a black-out has already
            // taken it.** Losing anywhere in this building warps the run to the Saffron Centre — and
            // these four steps then describe a walk down a pad maze the run is no longer standing
            // in, which a single-hop `EnterMap` cannot resolve from a city and will sit on for ever
            // (measured: the leg chain stalled here at 23 of 31 steps, twice). Reaching Saffron is
            // the *point* of the sequence, so arriving there early makes each of them a no-op.
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
            // ⚠️ **TM14 Blizzard, at (19,25) right beside this switch, and it used to be walked past.**
            // The old note said collecting it shifted the RNG onto the losing side of the Route-22
            // rival — a coin flip the run could not afford, because the Viridian Mart does not stock
            // the Hyper Potions that leg tries to restock and the fight was had on leftovers. That
            // argument is spent: the rival is now met by a starter fifteen levels higher with a full
            // bag, and Articuno — the mon this TM used to be saved for — is no longer on the route.
            // It is the only Ice the party can get, and Ice is 2× on Lance's dragons and 4× on
            // Dragonite.
            Self::CollectItem(MapSprite::POKEMONMANSIONB1F_TM_BLIZZARD),
            // ⚠️ **Taught here rather than after the Secret Key, because a TM is consumed on use and
            // the bag has no room for both.** The Rare Candy at the top of this leg frees exactly one
            // entry; picking up Blizzard fills it again, and the *next* pickup — the Secret Key the
            // gym is locked behind — is then refused in silence. Measured: 3000 polls one tile from
            // the key, a give-up, and the run walked to a Cinnabar Gym it could not open.
            Self::TeachMove { item: ItemId::Tm14Blizzard, target: Self::STARTER_LINE },
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
            // ⚠️ **Venusaur leads Blaine, and the comment here used to say Vaporeon — read the code, not
            // the sentence.** On type alone Vaporeon is the answer: Blaine's team is all Fire, which
            // takes 2× from Surf, and Venusaur is Grass and takes 2× back. On levels it is not close —
            // the gift Eevee is still around lv25 here against a Venusaur in the fifties — and a lv25
            // lead in a gym gauntlet is a black-out, so bulk wins the argument. The step was
            // `Slot(1)` when the party was exactly `[Venusaur, Vaporeon]`, which is what the old
            // comment was describing; naming the species is what made the disagreement visible.
            Self::MovePokemonToFront { target: Self::STARTER_LINE },
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
    /// ## Strength, and the slave that used to carry it
    ///
    /// ⚠️ **This leg caught a Slowpoke for years and no longer needs to.** Neither Venusaur nor
    /// Vaporeon learns HM04, so the boulder pushes needed a body; **Blastoise learns Strength itself**
    /// (`data/pokemon/base_stats/blastoise.asm`), and it is already the lead, so the catch, the teach,
    /// the Great Balls bought for it and the party slot all go. The same line retires the Victory Road
    /// Machop. Dig comes off TM28 onto the starter for the same reason — it is the way out of here.
    pub fn seafoam_articuno_steps() -> Vec<Self> {
        // Strength is armed per floor: `BIT_STRENGTH_ACTIVE` is cleared on every map change, and the
        // route leaves and re-enters each boulder floor.
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
            // ⚠️ **This leg catches its own Strength slave again, because the route no longer carries
            // one past Cinnabar.** Strength lives on the Victory Road Machop now — caught two legs
            // *after* this one — and the starter deliberately does not learn it (an HM is a permanent
            // slot in the only mon that fights). Seafoam is not on `complete_game_steps` any more, so
            // it has to be self-contained: a Slowpoke off 1F's own table takes HM04 and TM28.
            // ⚠️ **Poké Balls, and the pin is load-bearing.** `Bag::best_pokeball` ranks by
            // effectiveness, so a fallback with a Master Ball in the bag spends *it* on the HM slave
            // and leaves nothing for the bird — which is exactly what happened when the Great Ball
            // purchase above came back "the wallet covers no more". The run carries Poké Balls left
            // over from Cerulean (the Drowzee that used to spend them is no longer caught), and a
            // Slowpoke's catch rate is 190.
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
            // ── SEAFOAM4: two of B3F's four boulders into its two holes. The planner moves (5,14) out
            // of the corridor first — it is the only tile from which (3,15) can be reached at all.
            Self::UseStrength { target: PartyRef::Species(PokemonSpecies::Slowpoke) },
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
            Self::Dig { target: PartyRef::Species(PokemonSpecies::Slowpoke) },
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
    /// What the gauntlet lead is taken to. Public so `endgame::can_finish_from_victory_road` can seed
    /// to exactly the number the grind targets rather than a second one that drifts.
    ///
    /// ⚠️ **Eighty-five for one mon, where it used to be seventy-five for three.** See
    /// `gauntlet_grind_steps`: the three-target grind was 1.4 M experience and this is 425 k, because
    /// experience is cubic and the top of one curve is cheaper than the middle of three.
    pub const GAUNTLET_LEVEL: u8 = 85;

    /// **Which starter this route plays**, as one pair of constants rather than a species name
    /// scattered through six legs.
    ///
    /// ⚠️ **Grass is the beginner's answer and this is a speed run.** Bulbasaur is 4× into Brock and
    /// 2× into Misty, which is the whole of its case, and from Cerulean onward Grass is resisted by
    /// Poison, Bug, Fire and Flying — most of what Kanto fields. Koga is where that ends: four
    /// Poison-types with Razor Leaf at 0.5× on all of them, so the lone starter does not run out of
    /// *levels*, it runs out of **turns**. The same shape black-outs the Nugget Bridge and Route 6.
    ///
    /// ⚠️ **What actually decided it was PP rather than power.** Bulbasaur's only damaging moves are
    /// Tackle's 35 PP and Vine Whip's **10 at lv13**; once both are dry `pick_best_move` returns
    /// nothing and the fall-through picks a status move *on purpose*, so the battle resolves into a
    /// black-out — which against a trainer that cannot be fled it does over and over. Squirtle has
    /// **Bubble at 30 PP from lv8 and Water Gun at 25 from lv15**, and Water is 4× into Brock exactly
    /// as Grass is. Measured end to end, that alone removed the five black-outs on the Nugget Bridge
    /// and Route 6, and **Bite at lv24 — which is *Normal* in Gen 1, so neutral into Starmie** —
    /// removed Misty's five: twelve down to four.
    ///
    /// ⚠️ **Blastoise cannot cut a tree, and that is the price of the swap rather than an objection
    /// to it.** `data/pokemon/base_stats/wartortle.asm` lists SURF and STRENGTH and no CUT, where
    /// `ivysaur.asm` has CUT; this route needs Cut four times (the Vermilion Gym tree, Celadon's gym
    /// maze twice, Route 2 on the way to Cinnabar). So the lone-starter party is gone and
    /// [`Self::CUT_SLAVE`] is what replaces it. What Blastoise buys back is the rest of the party:
    /// it carries **Surf and Strength itself**, which retires the whole Eevee → Vaporeon leg (a
    /// Celadon round trip and a Water Stone), the Seafoam Slowpoke and the Victory Road Machop —
    /// three catches and two HM slaves for one.
    const STARTER_BALL: MapSprite = MapSprite::OAKSLAB_SQUIRTLE_POKE_BALL;
    /// The starter, named as its whole line.
    ///
    /// ⚠️ **A line and not a species, because a dozen steps address it and two of them run before it
    /// is a Blastoise.** Squirtle evolves at 16 and Wartortle at 36, and the grind that reaches 24 for
    /// Bite, the Cut-era teaches and the Safari HMs are all on the near side of that — so
    /// `Species(Blastoise)` would simply answer `None` and the step would wait for ever.
    const STARTER_LINE: PartyRef = PartyRef::Line(&[
        PokemonSpecies::Squirtle, PokemonSpecies::Wartortle, PokemonSpecies::Blastoise]);

    /// **The Cut carrier, and the reason the party is not just a starter any more.**
    ///
    /// ⚠️ **Blastoise cannot learn Cut and four legs of this route need it.**
    /// `data/pokemon/base_stats/wartortle.asm` lists SURF and STRENGTH and no CUT, where
    /// `ivysaur.asm` has it — so the Bulbasaur route got Cut, Surf and Strength out of one mon and
    /// this one cannot. An HM slave is the whole difference, and it is cheap: Oddish is 20% of Route
    /// 5's grass, which the route already walks through on its way to the Underground Path, and it
    /// is the earliest thing on the map that learns Cut (`grep CUT data/pokemon/base_stats/*.asm`:
    /// the Oddish and Bellsprout lines, Paras, Sandshrew, Krabby, Tentacool and the two starters that
    /// are not this one).
    ///
    /// ⚠️ **It is named by species and never by slot**, and it evolves at 21 — which does not matter,
    /// because the only step that names it is the teach immediately after the catch, and every *use*
    /// of Cut resolves the carrier out of the live party (`agent::field_move_carrier`). Gloom learns
    /// Cut too.
    const CUT_SLAVE: PartyRef = PartyRef::Species(PokemonSpecies::Oddish);

    /// The Victory Road boulder slave, named by species because it is caught *inside* the leg that
    /// uses it — the slot it lands in depends on how many mons the run arrived with, which is exactly
    /// what the old `machop_slot` argument was guessing at (and its two callers guessed differently).
    const MACHOP: PartyRef = PartyRef::Species(PokemonSpecies::Machop);

    /// **The Psychic**, and the one place this route argues with the brief it was given.
    ///
    /// Gen 1 Psychic is the strongest attacking type in the game — resisted by nothing that matters,
    /// 2× on the Poison and Fighting that Agatha and Bruno are made of — so a psychic in the gauntlet
    /// is worth a party slot. **Abra is the wrong one, and it is wrong for a mechanical reason rather
    /// than a stats one**: a wild Abra knows Teleport and nothing else, and Teleport *ends a wild
    /// battle*. A trainee that leads a grind therefore escapes every encounter it is put into and
    /// earns nothing, and the only way round it is the turn-one switch that halves the payout and
    /// costs the turn — which is precisely what `pick_field_move`'s lead-with-the-trainee rule was
    /// written to delete. Sixteen levels of that is not a saving.
    ///
    /// Drowzee is the same type from a mon that can fight the moment it is caught: Pound and
    /// Hypnosis on arrival, Confusion at 17, **Psychic at 32** (Kadabra's is 38), Hypno at 26 with
    /// 115 Special and far more bulk than Kadabra's 40 HP. And it is free: Route 11 is already on the
    /// walk from Vermilion to Diglett's Cave in [`Self::saffron_to_cinnabar_steps`], so the catch is
    /// a step rather than a detour.
    const PSYCHIC_LINE: PartyRef = PartyRef::Line(&[PokemonSpecies::Drowzee, PokemonSpecies::Hypno]);

    pub fn victory_road_1f_steps() -> Vec<Self> {
        let mut steps = Self::victory_road_1f_approach_steps();
        steps.extend(Self::victory_road_1f_climb_steps());
        steps
    }

    /// Viridian → the Route-22 rival → Route 23 → VR1F, ending with a Machop caught and taught
    /// Strength. Everything up to the point where the party is standing on the floor it grinds on.
    pub fn victory_road_1f_approach_steps() -> Vec<Self> {
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
            Self::MovePokemonToFront { target: Self::STARTER_LINE },
            Self::enter(Map::Route22),
            Self::enter(Map::Route22Gate),           // walk west → rival ambush → gate to Route 23
            Self::Interact(MapSprite::ROUTE22GATE_GUARD), // walk to (5,2): badge check + flips the dynamic warp
            Self::enter(Map::Route23),
            Self::goto(Map::VictoryRoad1F),
            // ⚠️ **The boulder slave, caught two tiles from the boulder it is for.** Blastoise *can*
            // learn Strength, and for a while it did — but an HM is the one move
            // `pick_move_to_forget` will never drop, so 80-power Normal would sit in a permanent slot
            // of the only Pokémon this route fights with. A Machop off Victory Road's own wild table
            // costs one catch and keeps that slot for an attack.
            Self::CatchPokemon { species: PokemonSpecies::Machop, on_map: Map::VictoryRoad1F, ball: None },
            Self::TeachMove { item: ItemId::Hm04Strength, target: Self::MACHOP },
            // The catch leaves the Machop leading, and the nine VR trainers below are not its fight.
            Self::MovePokemonToFront { target: Self::STARTER_LINE },
        ]
    }

    /// The boulder onto (17, 13) and the climb to VR2F.
    pub fn victory_road_1f_climb_steps() -> Vec<Self> {
        vec![
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
            // Return trip: climb back to VR3F and come down on the **exit** side.
            Self::enter(Map::VictoryRoad3F),
            // ⚠️ **(27,7), not the (22,16) the trip in uses, and the difference is the whole exit.**
            // VR3F has two ways down and only one of them lands in the pocket the Route 23 warp is in.
            // Asking for (22,16) here puts the player at (25,14), from which the only reachable warps
            // are VR1F(1,1) and VR3F — so the last step stalls for good. The mainline survived it by
            // accident: `EnterMap` falls back to re-routing over the incremental world graph, which by
            // then knew the (27,7) landing, and a leg test's fresh agent does not. Naming the landing
            // the run actually needs makes it a single hop again, which is what `enter_at` is for.
            Self::enter_at(Map::VictoryRoad2F, 27, 7),
            // Out the exit beside it → Route 23 → Indigo Plateau → the Elite Four lobby.
            Self::enter(Map::Route23),
            Self::enter(Map::IndigoPlateau),
            Self::enter(Map::IndigoPlateauLobby),
        ]
    }

    /// The Elite Four gauntlet, from the Indigo Plateau lobby to the Champion: stock up, heal, then
    /// Lorelei → Bruno → Agatha → Lance → the rival. Validated by `can_beat_elite_four`.
    ///
    /// ⚠️ **The two leads are named by species and the slot arguments are gone.** They used to be
    /// `u8`s the caller worked out, on the argument that `MovePokemonToFront` rotates the party — and
    /// a caller computing an index it cannot verify is exactly the `machop_slot` bug that made
    /// `can_solve_victory_road_1f` unfixable for months. `PartyRef::Species` is resolved against the
    /// live party at the moment the step runs, which is the only time the answer is knowable.
    ///
    /// `ice_lead` takes over before Lance. The battle policy only switches when the active mon has *no*
    /// damaging move left, so without the swap Vaporeon stays in and chips away with Bite once
    /// Blizzard's 5 PP is gone — which is exactly how the first Articuno attempt ran Lance's room out of
    /// the clock. Ice Beam is 4× on Dragonair/Dragonite, 2× on Gyarados and Aerodactyl, and 2× again on
    /// the Champion's Pidgeot/Exeggutor/Rhydon/Gyarados.
    /// Bring the two fighters the gauntlet is built around up to weight, on the Victory Road floor
    /// the run is already standing on.
    ///
    /// ⚠️ **The mainline arrives far weaker than the fixture the Elite Four was proved on.**
    /// `can_beat_elite_four` runs from `at-indigo-articuno.bin` — Articuno lv71, Venusaur lv70,
    /// Vaporeon lv70 — while a fresh `complete_game_steps` reaches Victory Road with Venusaur lv60,
    /// Articuno lv51 and a Vaporeon that is still **lv26**, because the mainline catches the bird at
    /// 50 and never grinds it. That gap, not the boulder puzzle, is what "the plateau is out of
    /// reach from here" actually meant.
    ///
    /// ⚠️ **The Pokémon Mansion, and the four rejected sites are each rejected for a different
    /// measured reason.** This one took five attempts, so all of them are written down — and the fifth
    /// is the one that looks right on paper, so read it before proposing it again.
    ///
    /// *Route 23* looks ideal, next door to the plateau — but its grass is not reachable from either
    /// end the run can stand on (probed from (9, 1): `grass_tiles=true`, `grass_reachable=0`, and the
    /// only two actions are ways off the map). `GrindUntilLevel` fell through to the cave-pacing
    /// branch, paced to the one thing it could reach, was routed back because that is not `on_map`,
    /// and repeated it with no battles at all. `has_grass_tiles` now keeps a route out of that branch.
    ///
    /// *Victory Road 2F* is a cave, so pacing is right there — but it is in `pick_battle_action`'s
    /// center-less-dungeon list, which flees every wild outright so the boulder traversal keeps its
    /// PP. Measured: **375 encounters, 375 runs, zero experience.** `GrindUntilLevel` is exempt from
    /// that now, which was worth fixing regardless, and VR2F is still wrong for the next reason.
    ///
    /// *Victory Road 1F* is a cave and is *not* in that list, so both problems go away — and it fails
    /// on distance. The trainee is switched in to earn the experience, wears down over ten or twenty
    /// battles with nothing healing it between, and faints; the detour then routes to the nearest
    /// Pokémon Centre, which from inside Victory Road is **Viridian, four maps away**. Measured on the
    /// mainline: three round trips and not one level gained.
    ///
    /// *Cerulean Cave* is the fifth, and it is the one the numbers actually point at — which is why it
    /// is written down at length rather than left for someone to rediscover.
    /// `wild::tests::probe_grind_sites` ranks every encounter block in the ROM out of the cartridge's
    /// own tables, and the ranking is not close: **Cerulean Cave 1F pays 1055 experience a knockout
    /// against the Mansion's 588**, at the same 10/256 encounter rate, with wilds at lv46-53 instead of
    /// lv28-39; B1F pays 1264 and 2F 1110. It is also next door to a Pokémon Centre. Experience per
    /// *knockout* is the figure that matters here rather than per step, because a grind's time goes
    /// into battles: the measured leg is 1552 wild battles in 1229 s, which is about 40 s of cartridge
    /// time an encounter cycle, under 7 s of it the walk between them.
    ///
    /// ⚠️ **And it is not gated by a script, whatever it looks like.** `wNumHoFTeams` is read by
    /// exactly four things in the ROM — the League PC, the save screen, the ceremony and Bill's PC —
    /// and none of them is in Cerulean; `CeruleanCity_Script`'s only two coordinate triggers are the
    /// Rocket thief and the rival. The man beside the door is `CERULEANCITY_SUPER_NERD3` and his whole
    /// contribution is the line "The #MON LEAGUE champion is the only person who is allowed in!",
    /// which is text and nothing else.
    ///
    /// ⚠️ **It is gated by his body, and that is what actually stops this.** He stands at (4,12), one
    /// tile below `warp_event 4, 11, CERULEAN_CAVE_1F`, on a ledge-ringed terrace whose only approach
    /// to the door is through him. Probed from a walked-in, pre-Champion save standing on the terrace's
    /// water at (19,1), the whole reachable set is three actions — the Route 4 connection, the Route 24
    /// water crossing, and the man himself at 26 steps. **The cave warp is not in it at all.** So a
    /// mid-run grind there is not a route problem to be solved; there is no route.
    /// (`postgame::legendaries::mewtwo_steps` *does* get in, from a save that has been Champion for a
    /// while — so he evidently moves once the game is finished. Nothing in this route is ever in that
    /// state, and the Elite Four is precisely what the grind is for.)
    ///
    /// ⚠️ **The walk itself works and was not the obstacle**, which is worth knowing if anyone revisits
    /// this. Cinnabar → Cerulean as twenty-two explicit `enter` hops — Route 21 by Surf, Viridian,
    /// Route 2's gate, Diglett's Cave, Vermilion, Saffron's two gate houses — crossed first time from a
    /// cold fixture. Two things it taught: a `Goto` cannot do that job from a leg test, because
    /// `route_toward` reads the incremental world graph and a fixture builds a fresh agent that has
    /// observed nothing (measured: the step sat on `CinnabarIsland (11, 13)` for a whole budget); and
    /// every gate crossing has to be an `enter_at`, because a gate warps to `LAST_MAP` so a plain
    /// `enter` back onto the route is satisfied by the door just walked in through.
    ///
    /// So: the Mansion, still. It is a building, so every step rolls an encounter and there is no grass
    /// to be unreachable; its wilds are lv30-39 rather than Victory Road's lv22-36; it is not in the
    /// center-less list; and Cinnabar's Centre is **one** map from its door. The run is already here —
    /// `seafoam_articuno_steps` ends on Cinnabar Island with the last party member caught — so the
    /// grind costs no travel to reach, which is the other thing every Victory Road placement paid for.
    /// Of everything the run *can* stand on before the Elite Four, it is the best there is: the four
    /// Mansion floors and Victory Road's three are the only entries above 450 experience a knockout in
    /// the whole ranking, and the Mansion's are the ones with a Centre beside them.
    ///
    /// ⚠️ **Its one real cost is poison, and it is the worst site in the game for it.** Half of 1F's
    /// encounter slots are Poison-type — Koffing at 40%, and Grimer behind it — and Gen 1 ticks
    /// overworld poison at 1 HP every four steps and cures it nowhere but a Centre, so the trainee
    /// walks home over and over. `wild::poison_share` is the column that says so, and the
    /// `trip #` counter on the fainted-trainee line beside it is what a run actually costs.
    pub fn gauntlet_grind_steps() -> Vec<Self> {
        /// What the two leads are taken to before the gauntlet.
        ///
        /// ⚠️ **Deliberately well over the fight rather than level with it, and the margin is the
        /// feature.** The Elite Four tops out at Lance's lv62 Dragonite and the rival's lv65, and a
        /// party that merely matches them makes the gauntlet a coin flip: two ungrinded attempts at
        /// Venusaur lv60 / Articuno lv51 lost in different rooms, one to the rival's Exeggutor and
        /// one to a Hyper Beam crit from Lance's Gyarados. A coin flip is not something
        /// `full_playthrough` can assert on.
        ///
        /// ⚠️ **It is also what makes a re-entry story unnecessary.** A blackout inside the gauntlet
        /// warps the player out to the Indigo Plateau, and the queue's next step is the *next room* —
        /// which is only reachable back through the ones already beaten, and there are no steps left
        /// to redo them. The run does not recover; it spun 29,915 polls on `EnterMap(ChampionsRoom)`
        /// before the harness called it. Rather than teach the route to re-walk five rooms, the
        /// cheaper and more honest fix is to not lose.

        vec![
            // ⚠️ **The heal is not a courtesy, it is what makes the grind survivable.** It sets
            // `last_pokemon_center`, which is the only thing the trainee-fainted detour below routes
            // to — and a trainee wears down over ten or twenty wild battles and then faints, so that
            // detour runs over and over. One map each way is the difference between a grind and a
            // walking simulator.
            // ⚠️ **Out of Blaine's gym first.** `volcano_badge_steps` ends standing *inside* it, and
            // this leg used to follow `seafoam_articuno_steps`, whose own first step was this hop.
            // With Seafoam gone the grind inherited a queue that opens on a Pokémon Centre two warps
            // away, which `EnterMap` — a deliberate single hop — cannot resolve.
            Self::enter(Map::CinnabarIsland),
            Self::enter(Map::CinnabarPokecenter),
            Self::Interact(MapSprite::CINNABARPOKECENTER_NURSE),
            Self::enter(Map::CinnabarIsland),
            // ⚠️ **The run arrives at its longest grind with ¥37,655 and not one healing item, and
            // that — not the site — is what all the walking back to the Centre was.** Measured on
            // `post-articuno.bin`: seventeen bag entries, none of them a potion or a status cure. So
            // `pick_battle_action`'s "heal below 25%" arm has nothing to reach for and never fires
            // once in 1552 battles, and the trainee is left to be ticked to death by a Ponyta's burn
            // or a Koffing's Smog — which is exactly what happened, twelve times, each costing a
            // four-warp round trip to Cinnabar and back. **Full Heals** are the cure and Cinnabar
            // stocks them (`data/items/marts.asm`: it is the only mart on the route that does);
            // the Hyper Potions are what the 25% arm needs to exist at all.
            //
            // Buying is the whole of the fix on the route side; using them is `pick_field_move`'s
            // status arm and the battle arm that was always there. The money is otherwise dead:
            // `elite_four_steps`' twelve Full Restores are famously never used (57 Fights, one switch,
            // zero items).
            //
            // ⚠️ **On the *fixture* this buys twenty and the trips go to zero. On the mainline it buys
            // three and they do not — and the difference is the whole fixture-versus-mainline trap.**
            // `post-articuno.bin` carries **¥37,655**; the run that earns its own way to the same point
            // arrives with about **¥2,000**, so `agent::affordable` trims the order to what the wallet
            // covers and `can_grind_for_the_gauntlet` measures a grind the real route cannot pay for
            // (0 trips from the fixture, 13 on `hall_of_fame_playthrough`). Do not quote the leg test's
            // number as the run's.
            //
            // ⚠️ **And the reason it is poor is the black-outs, which is a loop worth seeing.** A
            // black-out halves the money; the route has eleven, all of them before this shop; so the
            // medicine that would prevent them is the thing they make unaffordable. Fixing the early
            // game is therefore upstream of fixing this — see the black-out table on `game_steps`.
            Self::BuyFromMart { item: BagItem::new(ItemId::FullHeal, 40), map: Map::CinnabarMart },
            Self::BuyFromMart { item: BagItem::new(ItemId::HyperPotion, 10), map: Map::CinnabarMart },
            Self::enter(Map::CinnabarIsland),
            Self::enter(Map::PokemonMansion1F),
            // ⚠️ **One fighter, taken further, and it is *cheaper* than three at seventy-five —
            // experience is cubic, so the top of one curve costs less than the middle of three.**
            // Measured on the three-target version: Hypno 26→75 is about 404 k experience, Articuno
            // 50→75 about 610 k and the starter 60→75 about 400 k — **1.4 M** in all, and thirty
            // minutes of the run. The starter alone from 60 to 85 is about **425 k**, under a third,
            // and the levels buy more than the bodies they replace: the Elite Four tops out at
            // Lance's lv62 Dragonite and the rival's lv65, so a lv85 lead one-shots almost everything
            // it meets rather than trading turns with it.
            //
            // ⚠️ **That makes PP the binding constraint rather than power.** Five rooms is about
            // twenty-six knockouts against Surf's 15, Blizzard's 5 and Dig's 10 — which is why the
            // starter's slots are not spent on HMs it never attacks with (see
            // `safari_zone_strength_steps`) and why the lobby nurse in `elite_four_steps` matters.
            //
            // ⚠️ **This replaced "three fighters or you lose the Champion's room".** That was true of
            // three mons at *seventy-five*: the party behind them was a lv26 Vaporeon, a lv30
            // Slowpoke and a lv24 Machop, so the moment the second lead fell three fodder mons
            // fainted in a row. Depth of bench was never the answer — height was.
            Self::GrindUntilLevel { target_level: Self::GAUNTLET_LEVEL, on_map: Map::PokemonMansion1F,
                target: Self::STARTER_LINE },
            Self::enter(Map::CinnabarIsland),
        ]
    }

    pub fn elite_four_steps() -> Vec<Self> {
        vec![
            // ¥3000 and ¥1500 each — 12 + 4 is ¥42,000, which is inside what the grinded
            // `at-indigo-articuno` fixture arrives with (~¥50k) and well outside what the mainline
            // does (¥9,710 at Victory Road 2F, plus whatever VR2F/VR3F's trainers pay).
            //
            // ⚠️ **Asking for more than the wallet holds is safe, and the comment here used to say
            // the opposite.** It claimed `BuyFromMart` gives up rather than buying fewer — that was
            // true of the *step*, and stopped being true when `agent::affordable` was added to trim
            // an order down to what the money covers (which is itself the fix for
            // `silph_co_card_key_steps` ordering ¥18,000 of Hyper Potions on ¥7,838 and buying zero
            // every run). So this stays an ask rather than a budget: a rich party gets twelve, and
            // the mainline gets as many as it can pay for.
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
            // (No swap for Lance. Articuno used to come in here for its Ice STAB against his
            // dragons; TM14 Blizzard is on the starter now — see `mansion_secret_key_steps` — and Ice
            // is 2× on Dragon and on Flying, so Dragonite takes 4× from the mon already out.)
            Self::enter(Map::LancesRoom),
            Self::BattleTrainer { trainer: MapSprite::LANCESROOM_LANCE },
            Self::enter(Map::ChampionsRoom),
            Self::BattleTrainer { trainer: MapSprite::CHAMPIONSROOM_RIVAL },
        ]
    }


    /// The full deterministic playthrough. Every forward map transition is an explicit `EnterMap`;
    /// on-map tasks (`Interact`/`Buy`/`Grind`/`Catch`) self-route over the incrementally-observed
    /// graph.
    ///
    /// The party is **one Blastoise and two HM slaves** — an Oddish for Cut ([`Self::CUT_SLAVE`]) and
    /// a Victory Road Machop for Strength ([`Self::MACHOP`]). Only the starter ever fights, and the
    /// gauntlet grind takes it far enough above the Elite Four to do it alone; see
    /// [`Self::gauntlet_grind_steps`] for why one mon taken further is cheaper than three taken less
    /// far. Surf is the one HM it carries, because Surf is a 95-power STAB attack that happens to be
    /// an HM.
    pub fn complete_game_steps() -> Vec<Self> {
        Self::game_steps(true)
    }

    /// The same route stopped at the eighth badge and Victory Road 2F, with no gauntlet grind and no
    /// Elite Four.
    ///
    /// ⚠️ **This exists so the pre-commit gate stays affordable, and that is a real trade.**
    /// `full_playthrough` runs this in about five minutes; [`Self::complete_game_steps`] takes **50**,
    /// most of it the ~1830 wild battles `gauntlet_grind_steps` needs. A half-hour gate is
    /// one nobody runs — which is exactly how `full_playthrough` came to sit broken for a long time
    /// behind a doc comment claiming it was green — so the long one gets its own `hall-of-fame`
    /// feature and this one keeps the job it has always had.
    ///
    /// ⚠️ **What is given up is named rather than hidden:** nothing in the fast tier proves the
    /// endgame *composes onto a run that earned its own party*. `can_grind_for_the_gauntlet` proves
    /// the grind from a fixture and `can_finish_from_victory_road` proves the rest from another, with
    /// the levels seeded between them; only `hall_of_fame_playthrough` joins them up.
    pub fn eight_badge_steps() -> Vec<Self> {
        Self::game_steps(false)
    }

    /// Pallet Town, the starter, Brock, the Route 3 grind and into Mt Moon — everything the route
    /// does before [`Self::mt_moon_traversal`] puts it in Cerulean.
    ///
    /// Its own function so a fixture can be *produced* from it: `post-cascade.bin` and everything
    /// downstream of it is a committed chain rooted in a state no test builds, which is fine until
    /// the mainline party changes under it — and swapping the starter changes every one of them at
    /// once. `regen_early_game_fixture` plays this and saves the root.
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

            // ── LONE STARTER (no caught second mon). A weak, low-level catch (the old Route 1 Pidgey)
            // is worse than nothing in the attrition dungeons: when the starter faints there, the game
            // forces the weakling in, and — being alive but unable to win *or* flee a trainer battle —
            // it blocks the black-out that would otherwise heal the whole party, hard-deadlocking the
            // crossing (observed in Mt Moon). With just the starter, a faint triggers a black-out →
            // heal → re-enter and re-fight already a little stronger, which clears Mt Moon and the
            // Nugget Bridge by convergent recovery. (So we also skip buying Poké Balls above.)
            //
            // ⚠️ **The convergent recovery used to cost twelve black-outs a run, and the cause was
            // never the levels — it was an empty bag and a dry move.** The first healing item this
            // route bought was at *Vermilion*, so black-outs 1-9 were all fought with nothing for
            // `pick_battle_action`'s "HP critical" arm to reach for; and the lone starter's only
            // damaging move is Vine Whip's **10 PP**, after which the fall-through picks a status move
            // on purpose so the battle resolves into a black-out — which against a trainer that cannot
            // be fled it does over and over.
            //
            // ⚠️ **Five variants were measured and every one of them reduced the black-outs and
            // broke the run somewhere else. Read this before trying a sixth.** The route is not robust
            // here, it is *tuned*: it survives on one RNG stream, and anything added before Vermilion
            // re-rolls that stream onto a different pre-existing fault.
            //
            // | change | black-outs | where it broke |
            // |---|---|---|
            // | Potions at Pewter + Cerulean | 12 → 3 over the first 13% | Nugget Bridge, Vine Whip dry |
            // | Potions at Pewter only | — | Route 6, five black-outs on one screen |
            // | + a Cerulean Centre heal after Bill | **12 → 4 over the whole route** | Silph Co 5F |
            // | + tossing the Potions at Vermilion | 12 → 4 | Celadon Gym |
            // | the Centre heal on its own | 12 → 8 | Mt Moon, then Celadon |
            //
            // What each one taught, because none of it is about the purchase:
            // * A potion refills HP and **what runs out is the move** — Tackle's 35 and Vine Whip's 10,
            //   across five Nugget Bridge trainers, Bill, the Underground Path and Route 6 on one tank.
            //   A Pokémon Centre restores PP and is the only thing on this half of the map that does:
            //   **no mart in Gen 1 stocks Ether or Elixer** (`data/items/marts.asm`), they are floor
            //   items, and the nearest to this route are on the S.S. Anne, three legs too late.
            // * ⚠️ **The bag is the binding constraint and a full one refuses pickups in silence.** The
            //   run reaches Silph Co on exactly 20 entries, Gen 1's cap. One extra Potion stack meant
            //   the Card Key at 5F (21,16) could not be picked up, and `CollectItem` — whose only
            //   completion is the sprite disappearing, and which has no give-up — stood the player at
            //   (20,16) *one tile away* for 16,763 polls until the cycle budget ran out.
            // * The Centre heal is worth having and is not sufficient on its own: placed after Bill it
            //   misses Mt Moon and the outbound bridge, which is the 8 in the last row.
            //
            // So the honest prerequisite is neither medicine nor a shopping list: it is a starter that
            // can still damage something when its one good move is dry. Until that exists, this leg
            // keeps the stream it is tuned to — and the same argument is why swapping the starter for
            // Squirtle (Bubble at lv8 is 30 PP against Vine Whip's 10 at lv13, and Blastoise's Cut +
            // Surf + Strength would retire the whole Eevee leg) cannot be a first move: it re-rolls
            // every stream in the run at once and would surface all five of the above together.

            // ── Grind the starter on Route 1 ──
            Self::enter(Map::Route1),
            // ⚠️ **Twelve rather than thirteen, and the level is bounded by what there is to fight
            // *with*.** Squirtle learns **Bubble at lv8** — 4× into every one of Brock's Rock/Ground
            // mons, the same multiplier Bulbasaur's Vine Whip had at lv13 — so the badge is already
            // won by 10, and everything past that is a lone starter grinding with an empty bag,
            // because **no mart before Pewter sells a Potion** (`data/items/marts.asm`: Viridian's
            // counter is POKE_BALL, ANTIDOTE, PARLYZ_HEAL, BURN_HEAL). Ten was tried and is two
            // levels too thin for Viridian Forest's Bug Catchers, who cannot be fled.
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
            // Exit the gym to the city first (a single warp): every forward `enter` must be one
            // direct transition. Jumping straight to the Pokécenter from inside the gym is a 2-hop
            // path that would rely on routing through a never-before-observed gym-exit landing.
            Self::enter(Map::PewterCity),
            Self::enter(Map::PewterPokecenter),
            Self::Interact(MapSprite::PEWTERPOKECENTER_NURSE),
            Self::enter(Map::PewterCity),
            // ⚠️ **The first medicine the route can buy, and for a long time it bought none until
            // Vermilion.** `pick_battle_action`'s "heal below 25%" arm works and simply had nothing to
            // reach for, which is what every early black-out was: the lead walks into an encounter
            // worn from the last one and loses in two turns. Pewter is the first mart on the route
            // that stocks Potions at all — Viridian's counter is POKE_BALL, ANTIDOTE, PARLYZ_HEAL,
            // BURN_HEAL (`data/items/marts.asm`) — and `agent::affordable` trims the order to
            // whatever Brock's prize money covers.
            Self::enter(Map::PewterMart),
            Self::BuyFromMart { item: BagItem::new(ItemId::Potion, 10), map: Map::PewterMart },
            Self::enter(Map::PewterCity),

            // ── Route 3 grind → heal at the Mt Moon Pokécenter ──
            Self::enter(Map::Route3),
            // ⚠️ **Twenty-two, and the four extra levels are bought here because the next fight after
            // Mt Moon is the rival.** He ambushes the north exit of Cerulean before any of the Nugget
            // Bridge experience has been earned, and his Bulbasaur is Grass into a Water starter:
            // measured, the run lost that fight at lv21 twice, to Leech Seed and a run of missed
            // Tackles. ⚠️ **Not inside Mt Moon, which is where this was tried first and is cheaper per
            // battle.** A black-out on a cave floor warps the run to the Mt Moon Centre, and the
            // traversal's next step is an `enter_at` *between two of its own floors* — a warp
            // `route_toward` cannot find from outside, so the run stalls there for good. Route 3 is
            // outdoors, one connection from Pewter, and every recovery works.
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
        // ── Pallet Town → the starter → Brock → the Route 3 grind → into Mt Moon ──
        let mut steps = Self::pallet_to_cerulean_steps();
        // ── Cross Mt Moon → Cerulean City ──
        steps.extend(Self::mt_moon_traversal());

        steps.extend([
            // ── Heal in Cerulean ──
            Self::enter(Map::CeruleanPokecenter),
            Self::Interact(MapSprite::CERULEANPOKECENTER_NURSE),
        ]);

        // ── Nugget Bridge → Bill (SS Ticket) → Misty → trashed-house bridge → Vermilion City ──
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
        // (No Eevee leg. It existed to put Surf on a second body and to answer the Silph rival's
        // Alakazam; Blastoise carries Surf itself and takes the Alakazam on bulk, so what is left is
        // a Celadon round trip and a Water Stone.)
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
        // (No Seafoam detour. It existed to add Articuno as the Elite Four's Ice sweeper, and a
        // single over-levelled starter does not need a second sweeper — see `gauntlet_grind_steps`.
        // `seafoam_articuno_steps` is still here and still tested; it is simply not on the route.)
        // ── The gauntlet grind, on Cinnabar's doorstep while the party is finally complete ──
        // ⚠️ **Here rather than at Victory Road, and the four attempts are in `gauntlet_grind_steps`.**
        // This is the first point in the route where all three Elite Four fighters are in the party,
        // and it is a map away from the Pokémon Centre the faint detour needs. It is also the whole
        // cost difference between the two step lists, which is why it is the thing `finish` gates.
        if finish {
            steps.extend(Self::gauntlet_grind_steps());
        }
        // ── Cinnabar → Viridian Gym → Earth Badge (Giovanni), the 8th and final gym badge ──
        steps.extend(Self::earth_badge_steps());
        // ── Victory Road 1F: catch a Strength HM-slave, solve the boulder puzzle, climb to VR2F ──
        steps.extend(Self::victory_road_1f_approach_steps());
        steps.extend(Self::victory_road_1f_climb_steps());
        if finish {
            // ── VR2F/VR3F: the interconnected Strength puzzle, out to the Indigo Plateau lobby ──
            steps.extend(Self::victory_road_2f_3f_steps());
            // ── Lorelei → Bruno → Agatha → Lance → the rival → the Hall of Fame ──
            steps.extend(Self::elite_four_steps());
        }

        steps
    }
}

/// The scripted route's own cursor, kept beside the save it belongs to.
///
/// ⚠️ **A scripted run had no memory at all, and a restart did not merely interrupt it — it
/// desynchronised it.** `web::serve` builds the policy with `PolicyStep::complete_game_steps()` on
/// every process start, so a rollout resumed the *save* wherever it was and restarted the *route* at
/// step 0, in Red's bedroom. Several hundred steps then fail to resolve against a game halfway
/// across Kanto, pop one after another, and the run ends somewhere arbitrary. `POST /api/new-run`
/// was the same bug from the other side: it starts a fresh game and, before this, left the queue
/// half-consumed against it.
///
/// ⚠️ **A cursor rather than the queue itself.** `complete_game_steps()` is a pure function, so the
/// route can always be rebuilt; what cannot be rebuilt is how far along it the game is. Serialising
/// `PolicyStep` would mean `Serialize` on every type it reaches — `Map`, `MapSprite`, `ItemId`,
/// `PartyRef`, the Safari and Game Corner enums — for a number that fits in a `u32`.
#[cfg(feature = "web")]
mod scripted_progress {
    use std::path::Path;

    pub const FILE: &str = "scripted-progress.json";

    /// FNV-1a over each step's `Debug`, which is what makes the cursor safe to trust.
    ///
    /// ⚠️ **Not `DefaultHasher`**, whose output is explicitly not stable across Rust releases — the
    /// same reason `llm::history` compares the system prompt by text rather than by hash. A
    /// toolchain bump must not silently invalidate a run's cursor, because the failure it causes is
    /// the parked run below rather than a wrong answer.
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

    /// `(completed, total, route)`, or `None` for a run that has never written one — which is what a
    /// brand-new run looks like, and is correctly read as "start at the beginning".
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
            // A cursor that cannot be written is a run that will restart badly later, not one that
            // should stop now — so it is loud and not fatal, exactly like a failed checkpoint.
            println!("[policy] could not record scripted progress: {failure}");
        }
    }

    pub fn clear(dir: &Path) {
        let _ = std::fs::remove_file(dir.join(FILE));
    }
}

/// Where a scripted run records its place, and what it last recorded there.
#[cfg(feature = "web")]
struct ScriptedCursor {
    dir: std::path::PathBuf,
    /// The route entire, so `restart` can put it back. ⚠️ **`POST /api/new-run` was the same
    /// desync from the other side**: it starts the game over and, without this, left the queue
    /// half-consumed against a fresh save. `Policy::restart`'s default is a no-op and this policy
    /// never overrode it.
    full_route: Vec<PolicyStep>,
    /// The length of the route this cursor is counted against — half of what makes it safe to trust.
    total: usize,
    route: u64,
    /// `usize::MAX` until the first write, so a run that resumes at step 0 still records one.
    written: usize,
}

pub struct DeterministicPolicy {
    rng: StdRng,
    /// The seed both `rng` and `name_picker` were built from, kept so [`Policy::restart`] can
    /// rebuild this policy exactly as a fresh process would build it.
    seed: u64,
    queue: VecDeque<PolicyStep>,
    /// Where to record how far along the route this run is, and what it last recorded. `None`
    /// everywhere but `gb serve` — a test builds its own queue and has nothing to resume into.
    /// See [`scripted_progress`].
    #[cfg(feature = "web")]
    progress: Option<ScriptedCursor>,
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
    /// `(money, quantity held)` as the last `BuyFromMart` shop visit was opened.
    ///
    /// ⚠️ **A purchase the wallet cannot cover in full is a *success*, and the completion check used
    /// to call it a failure.** `agent::affordable` trims an order to what the money buys — the fix
    /// for `silph_co_card_key_steps` ordering ¥18,000 of potions on ¥7,838 and getting zero — but
    /// the step's own check is "the bag holds ≥ the quantity asked for", which a trimmed purchase can
    /// never satisfy. So a poor party re-opened the shop `MAX_MART_ATTEMPTS` times and printed "gave
    /// up" over a purchase that had worked as far as the money went. Observed at the Indigo Plateau
    /// on ¥9,710: twelve Full Restores asked, three bought on the first visit, three more visits that
    /// could buy nothing, "gave up", and then four more wasted on the Revives it could no longer
    /// afford at all. Comparing this against the next visit is what tells "the shop is not working"
    /// from "the wallet is empty", without needing the ROM price table here.
    mart_baseline: Option<(u32, u8)>,
    /// Consecutive ticks the heal-return detour has been unable to move: no route to the Pokémon
    /// Centre it is aiming at, or no Nurse in sight after arriving. Bounded because the detour is
    /// otherwise a silent permanent stall — see the ⚠️ on the branch in `pick_overworld_action`.
    heal_route_stuck: u32,
    /// Set once a heal detour has given up because the Centre could not be routed to, and cleared by
    /// the next actual heal. While it is set the low-PP flee stops arming another detour — see the ⚠️
    /// where it is assigned.
    heal_unreachable: bool,
    /// Where a heal detour set off from, so it can put the run back. See the return block in
    /// `pick_overworld_action`.
    heal_came_from: Option<Map>,
    /// Consecutive polls spent waiting for a nurse to finish. See `MAX_HEAL_WAITS`.
    heal_waits: u32,
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
    /// Polls the current `CollectItem` has spent on an item that has not been picked up, so a pickup
    /// the game silently refuses gives up with a reason instead of spinning. See
    /// [`Self::MAX_COLLECT_ITEM_WAITS`].
    collect_item_waits: u32,
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
    /// Black-outs this run has had, and whether the newest one is still waiting to be reported.
    ///
    /// ⚠️ **A black-out costs a route far more than the walk back, and nothing counted them.** It
    /// halves the money, warps the party to the last Centre — which is how a run that was two maps
    /// into Victory Road ends up in Viridian — and inside the Elite Four it is terminal, because the
    /// queue's next step is the next room and there are no steps left to redo the ones before it. The
    /// fix for a black-out is always upstream of it (a grind, a heal, a different lead), so the only
    /// thing worth recording is *where the run was standing and what it was carrying* when it lost.
    ///
    /// Reported from the overworld poll rather than from [`Policy::on_event`], because the event has
    /// no [`GameState`] to say what the party was. Where it *happened* comes from
    /// [`Self::last_battle_map`] instead, for the reason written there.
    blackouts: u32,
    blackout_pending: bool,
    /// Round trips a `GrindUntilLevel` has made to a Pokémon Centre because its trainee fainted.
    grind_heal_trips: u32,
    /// The map the last battle was fought on, which is the one a black-out has to be reported against.
    ///
    /// ⚠️ **Recorded from `pick_battle_action` rather than from the overworld poll, because the
    /// cartridge's own sentence arrives *late*.** The first version kept the last overworld map and
    /// printed it beside the map at report time; both came out as the map the run had already walked
    /// back to (`lost on MtMoonB2F, woke on MtMoonB2F` for a black-out that warps to Route 4's Centre),
    /// because the "blacked out" text box is only committed once the reader is flushed, several
    /// overworld decisions after the warp. A battle map cannot be overwritten by the walk home, so it
    /// still says where the run actually lost.
    last_battle_map: Option<Map>,
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
    /// Ticks the heal-return detour waits before concluding it cannot get to the Pokémon Centre and
    /// handing back to the main queue. Same units and the same reason as `MAX_GYM_ROUTE_WAIT`
    /// (20 ms each, ~8 s of game time — long enough to cover a black-out warp and its dialogue).
    const MAX_HEAL_ROUTE_WAIT: u32 = 400;
    const MAX_MART_ATTEMPTS: u32 = 4;
    /// How many times to hand one `UseBagItem` step to the driver before giving up (workstream I).
    /// A use the game declines consumes nothing, so without a bound the step retries for the whole
    /// leg — the same shape as the full-bag trap, and just as quiet.
    const MAX_ITEM_USE_ATTEMPTS: u32 = 4;

    /// How many polls a `CollectItem` may spend on an item that will not be picked up.
    ///
    /// ⚠️ **Generous on purpose, because the step legitimately waits.** An item ball can be hidden
    /// until its guard is beaten (the Rocket Hideout Lift Key, the Silph Scope) and a pickup can be
    /// interrupted by a wild battle on the approach tile (the Mt Moon fossil), so a small bound would
    /// abandon errands that were about to work. This is only there to stop the *silent* case — a
    /// pickup the game refuses — burning a whole run's cycle budget, which it did: 16,763 polls.
    const MAX_COLLECT_ITEM_WAITS: u32 = 3000;
    /// Polls a nurse gets to finish healing before the route carries on without her. A heal is a few
    /// seconds of cartridge time; this is about thirty at the agent's 20 ms tick.
    const MAX_HEAL_WAITS: u32 = 1500;
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

    /// Resume this route where the last process left it, and keep recording as it advances.
    ///
    /// ⚠️ **Only `gb serve` calls this**, because only a run directory can answer "how far along
    /// were we". Every test builds its queue and its expectations together and must keep starting
    /// from step 0.
    ///
    /// Four outcomes, and the last two are the ones worth reading:
    /// - **no file, on a run starting from the beginning of the game** — nothing to resume. Start
    ///   at step 0, and record a cursor immediately so the next process has one.
    /// - **it matches** — skip that many steps and carry on. This is a rollout surviving.
    /// - **the route has changed under it** — the queue is emptied and the policy parks. ⚠️ **Not
    ///   "start from 0"**, which is the very failure this exists to prevent: a cursor is only
    ///   meaningful against the list it was counted on, and replaying a different route from the
    ///   beginning against a mid-game save is how a run gets destroyed rather than merely stopped. A
    ///   parked run is obvious, keeps its save, and is one `POST /api/new-run` from playing again.
    /// - ⚠️ **no file, on a run being *resumed*** — parks, for that same reason, and this is not a
    ///   hypothetical. A run started before cursors existed has no file and a save in the middle of
    ///   the game, and "no file means a new game" read that as Red's bedroom: the rollout of
    ///   2026-08-28 resumed a run standing in Victory Road, restarted the route at
    ///   `EnterMap { RedsHouse1F }`, and sat there failing to route for 745 polls until a human
    ///   started a new run. **Which of the two it is cannot be inferred from the file's absence**,
    ///   so the caller — who knows whether it just created this directory — passes it in.
    #[cfg(feature = "web")]
    pub fn resuming_in(mut self, run_dir: &std::path::Path, from_the_beginning: bool) -> Self {
        let total = self.queue.len();
        let route = scripted_progress::fingerprint(self.queue.make_contiguous());
        let full_route: Vec<PolicyStep> = self.queue.iter().cloned().collect();
        match scripted_progress::load(run_dir) {
            None if from_the_beginning =>
                println!("[policy] no scripted progress on disk — starting the route from the beginning"),
            None => {
                println!(
                    "[policy] ⚠️ this run is being resumed but recorded no scripted progress, so \
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
                    "[policy] ⚠️ the scripted route has changed under this run ({saved_total} steps \
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

    /// Write the cursor when it has moved. Called from the overworld poll, which is the busiest one
    /// — the comparison is an integer and the write only happens on a step boundary, of which a
    /// whole playthrough has a few hundred.
    #[cfg(feature = "web")]
    fn record_progress(&mut self) {
        let remaining = self.queue.len();
        let Some(cursor) = self.progress.as_mut() else { return };
        let completed = cursor.total.saturating_sub(remaining);
        if cursor.written == completed { return }
        cursor.written = completed;
        scripted_progress::save(&cursor.dir, completed, cursor.total, cursor.route);
    }

    #[cfg(not(feature = "web"))]
    fn record_progress(&mut self) {}

    pub fn new(seed: u64, steps: impl IntoIterator<Item = PolicyStep>) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
            seed,
            queue: steps.into_iter().collect(),
            #[cfg(feature = "web")]
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
            blackouts: 0,
            blackout_pending: false,
            grind_heal_trips: 0,
            last_battle_map: None,
        }
    }

    /// Route one hop toward `target` over the **incremental** world graph.
    ///
    /// The graph only contains sections the agent has already visited (accurate, sprite-resolved),
    /// so this succeeds for backtracking / already-explored territory (heal-return, reaching a map
    /// the explicit `EnterMap` steps have already led through) and returns `None` for a not-yet-
    /// visited target — the signal that the deterministic policy is under-specified.
    pub(crate) fn route_toward(world_graph: &WorldGraph, actions: &[OverworldAction], target: Map) -> Option<OverworldAction> {
        // ⚠️ **A transition to the target on *this* map is the shortest path, and asking the graph
        // first could miss it.** `pick_shortest_path_action` reads the incremental world graph, whose
        // nodes are keyed on the entry the agent actually landed on, so a player who has walked far
        // enough from that entry to fall outside `bfs_nodes`' `SNAP_THRESHOLD` gets `None` — while
        // the door or the connection it wants is sitting in the very `actions()` list passed in.
        // That is what "no route from Route3 to PewterPokecenter" was, on a map whose western
        // connection *is* Pewter.
        Self::enter_map_action(actions, target, None)
            .or_else(|| world_graph.pick_shortest_path_action(actions, target))
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

    /// Count black-outs. See [`Self::blackouts`] for why a route cares.
    ///
    /// ⚠️ **The cartridge saying so is the only reliable signal, and it says so exactly once.**
    /// `wBattleResult` is `LOSE` for a moment and then cleared by
    /// `ResetStatusAndHalveMoneyOnBlackout`, and the party is healed before any `pick_*` could read
    /// it, so a black-out is indistinguishable from a heal by the time the policy is next asked.
    /// `llm::battle_report::is_blackout` is the same test on the same sentence; it is repeated here
    /// rather than called because that module is behind the `llm` feature and this file is not.
    fn on_event(&mut self, event: &crate::pokemon::agent::AgentEvent) {
        if let crate::pokemon::agent::AgentEvent::TextBox { message } = event
            && message.contains("blacked out")
        {
            self.blackouts += 1;
            self.blackout_pending = true;
        }
    }

    fn pick_overworld_action(&mut self, state: &GameState, world_graph: &WorldGraph) -> Option<OverworldAction> {
        // ⚠️ **The cursor is written here rather than at every `pop_front`.** There are around forty
        // of those and a missed one is a silent regression to the desync `resuming_in` describes;
        // this is one place, on the poll that every step passes through, and it costs an integer
        // comparison per tick.
        self.record_progress();
        // Back in the overworld = the previous battle is over; clear the per-battle grind participation flag.
        self.trainee_participated = false;
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

        // ── Go and heal before the next fight, not after it ───────────────────
        // See `needs_a_centre`. ⚠️ **Only outdoors**, which is what `Map::is_overworld` is for: from a
        // town or a route the walk back is a walk, and from inside a building it abandons a chain of
        // single-hop `EnterMap` steps that resolve from nowhere else. ⚠️ And only while
        // `heal_unreachable` is clear, or this re-arms the detour that just gave up — the same latch
        // the low-PP flee needed, for the same reason.
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
        }

        // ── Heal-return detour ────────────────────────────────────────────────
        // When the active Pokémon ran low on PP in a wild battle we fled and
        // stored the target Pokémon Center in `heal_return`.  Route there over the
        // incrementally-built graph (the pokecenter and the way back are already known,
        // since we walked here) and talk to the Nurse before resuming the main queue.
        //
        // ⚠️ **The detour is bounded, because the graph it routes over can be missing the way
        // back.** `route_toward` reads the *incremental* world graph, whose nodes are keyed on the
        // entry the agent actually landed on — so a section reached some other way (walked to from
        // a neighbouring section, or arrived in by a black-out warp) has no node, and every exit
        // leading to one is a dangling target the BFS dead-ends on. Mt Moon B2F is the case that
        // shipped: the deployed run of 2026-08-28 blacked out, walked back in through B1F's (5,5),
        // and fled a Zubat in the fossil chamber whose only two exits land on B1F at (23,3) and
        // (21,17), 20 and 28 tiles from the one observed node and so past `SNAP_THRESHOLD` both.
        // `route_toward` answered `None`, this branch returned it, and — being an unconditional
        // early return above the `[policy]` print — the run went **silent and motionless for
        // hours** with the emulator still running. Every other step that routes carries a bound
        // (`gym_route_stuck`, `enter_stuck`, `interact_skip_waits`); this one did not.
        if let Some(pokecenter) = self.heal_return {
            if state.map.map == pokecenter {
                // Arrived — find and interact with the Nurse.
                if let Some(action) = actions.iter().find(|a| a.tile == MetaTile::Sprite("Nurse")) {
                    self.heal_return = None;
                    self.heal_route_stuck = 0;
                    return Some(action.clone());
                }
                // (falls through to the give-up below)
                // Pokecenter map but Nurse tile not visible yet — wait, but not for ever: the
                // sprite is a tile or two away on a map that always has one, so this is the
                // arrival settling rather than a state to sit in.
                self.heal_route_stuck += 1;
                if self.heal_route_stuck < Self::MAX_HEAL_ROUTE_WAIT {
                    return None;
                }
                println!("[policy] no Nurse in sight on {pokecenter} — carrying on without the heal");
                self.heal_return = None;
                self.heal_route_stuck = 0;
            } else if let Some(action) = Self::route_toward(world_graph, &actions, pokecenter)
                // ⚠️ **Then the town it stands in**, which is a strictly easier question and the one
                // that actually gets a hurt party home: towns are joined by walkable connections, so
                // every hop of that walk is a transition the current map's own `actions()` offers,
                // and the door is in the town's. See `Map::pokemon_center_town`.
                .or_else(|| pokecenter.pokemon_center_town()
                    .filter(|&town| town != state.map.map)
                    .and_then(|town| Self::route_toward(world_graph, &actions, town)))
            {
                // Still travelling — take the next step toward the pokecenter.
                self.heal_route_stuck = 0;
                return Some(action);
            } else {
                // Right after a black-out warp the map and its actions are briefly unsettled, so
                // wait rather than abandoning on the first miss — the same reason `gym_route_stuck`
                // waits. Past the bound the centre is genuinely unroutable from here and the main
                // queue is the better answer: it is a *route*, so its next step walks out of the
                // dungeon, which is where the centre is. Fainting on the way is not a failure —
                // the black-out heals the party and the queue picks up where it was.
                self.heal_route_stuck += 1;
                if self.heal_route_stuck < Self::MAX_HEAL_ROUTE_WAIT {
                    return None;
                }
                println!("[policy] no route from {} to {} to heal — carrying on with the route",
                    state.map.map, pokecenter);
                self.heal_return = None;
                self.heal_route_stuck = 0;
                // ⚠️ **Latch it, or the give-up is undone by the very next battle.** Handing back to
                // the route is only half an answer while the *reason* the detour was armed is still
                // true: the low-PP arm in `pick_battle_action` re-arms `heal_return` on the next wild
                // encounter, the detour fails to route again, and the run flees everything in between.
                // Measured on a Route 3 grind that had wandered past `bfs_nodes`' 8-tile
                // `SNAP_THRESHOLD` from its entry node: **2750 flees against 217 fights**, 1759 "no
                // route" lines, and a grind that never gained the two levels it was asked for before
                // the cycle budget died. Fighting on is the right behaviour once the Centre is out of
                // reach — that is exactly what black-out recovery is for — so the flee stays disabled
                // until something clears it, which a heal or a new run does.
                self.heal_unreachable = true;
            }
        }

        // ── Walk back to where the detour set off from ────────────────────────
        // ⚠️ **A heal detour that does not *return* strands the step it interrupted.** `EnterMap` is
        // a deliberate single hop, so a queue sitting on `EnterMap { PalletTown }` cannot be resumed
        // from inside the Cinnabar Pokémon Centre two maps away — and its recovery, re-routing over
        // the incremental world graph, needs an edge the run may only ever have walked the other way
        // (Pallet → Route 21 → Cinnabar records nothing about coming back). Measured: a detour off
        // Route 21 healed and then sat in the Centre for the rest of the budget.
        //
        // ⚠️ **Only for a step that cannot find its own way**, or this walks back to a grind map the
        // grind was about to route to anyway; and bounded by the same counter as the outward leg,
        // because the way back can be missing from the graph exactly as the way there can.
        if let Some(from) = self.heal_came_from {
            let front_reroutes = self.queue.front().is_some_and(step_finds_its_own_way_back);
            if self.heal_return.is_some() || from == state.map.map || front_reroutes {
                if self.heal_return.is_none() { self.heal_came_from = None; }
            } else if let Some(action) = Self::route_toward(world_graph, &actions, from) {
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
                        // The grind mon fainted. When training a bench slot the lead (Venusaur) keeps
                        // the party alive, so there's no black-out to auto-heal it — detour to the last
                        // Pokémon Center, then the grind resumes with a revived mon.
                        if pokemon.current_hp == 0 {
                            // ⚠️ **The detour's give-up has to be honoured *here*, or the give-up is
                            // not one.** `MAX_HEAL_ROUTE_WAIT` correctly abandons a heal it cannot
                            // route — but this arm re-arms `heal_return` on the very next tick,
                            // because the trainee is still fainted and this step is still at the
                            // front. Measured on a Route 11 grind whose return edge to Vermilion the
                            // agent had never walked: **158 trips, none of which moved a tile**, and
                            // the run sat there until its budget ran out. The right give-up is the
                            // same one every other unroutable branch makes — hand the route back —
                            // and a grind the party cannot heal for is one this run is not going to
                            // finish anyway.
                            if self.heal_unreachable {
                                println!("[policy] grind mon (slot {slot}) is fainted and no Pokémon \
                                    Centre can be routed to from {} — giving up on the grind",
                                    state.map.map);
                                self.heal_unreachable = false;
                                self.queue.pop_front();
                                continue;
                            }
                            if let Some(center) = self.last_pokemon_center {
                                // Counted, because the round trip is the grind's *other* cost and the
                                // only way to price a site against another one is to know how often it
                                // sends the trainee home. A poisoned trainee walks back over and over:
                                // Gen 1 ticks 1 HP every four steps in the overworld and cures it
                                // nowhere but a Centre, which is why `wild::poison_share` exists.
                                self.grind_heal_trips += 1;
                                println!("[policy] grind mon (slot {slot}) fainted — routing to {center} to heal (trip #{})",
                                    self.grind_heal_trips);
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
                    } else if !state.map.has_grass_tiles() && let Some(action) = actions.iter()
                        .filter(|a| match a.tile {
                            // Pace to the farthest reachable object so wild encounters (which fire on
                            // EVERY step in a cave/building) keep coming as the trainee ping-pongs across the
                            // map. EXCLUDE item-ball sprites (`PictureId::PokeBall`): walking onto one triggers
                            // a pickup that aborts on a full bag and loops forever (Pokémon Mansion).
                            //
                            // ⚠️ **A warp is only paced to when there is nothing else, and it used to be
                            // preferred because it is usually farthest.** The claim it rested on — "taking
                            // one just changes floor and `GrindUntilLevel` routes back" — assumes every
                            // floor can route back, and Victory Road's cannot: grinding on VR1F paced up
                            // the stairs and stranded the party on VR2F at (25, 14), a pocket whose only
                            // action is a warp deeper in, from which `on_map` is unreachable. The arm above
                            // then routed for ever. Staying on the floor is both safer and better grinding,
                            // since a floor change costs the transit rather than spending it on steps.
                            // ⚠️ **Boulders are excluded beside the item balls, for a neighbouring
                            // reason.** An item ball aborts the walk on a full bag; a boulder cannot
                            // be walked onto at all, so the approach ends in "This requires STRENGTH
                            // to move!" against a target the pacer will pick again next tick. Victory
                            // Road is full of them.
                            MetaTile::Sprite(name) => !state.map.sprites.iter().any(|s| s.name == name
                                && matches!(s.picture_id, crate::pokemon::sprite::PictureId::PokeBall
                                                        | crate::pokemon::sprite::PictureId::Boulder)),
                            _ => false,
                        })
                        .max_by_key(|a| a.route.len())
                        .cloned()
                        // ⚠️ **When the floor offers nothing to pace between, wander — never a warp.**
                        // The fallback here used to be "the farthest reachable warp", which on a floor
                        // whose only sprites are item balls means *every* pacing decision is a door.
                        // Measured on the gauntlet grind: **4435 map changes**, of which 1097 were the
                        // Mansion's exit onto Cinnabar Island and 1103 the staircase to 2F — the
                        // trainee walked out of the building and back in over two thousand times in
                        // 1552 battles, and the arm above dutifully routed it back each time. Every one
                        // of those is a screen fade and a map reload bought for nothing: the encounter
                        // roll is on the *step*, so pacing on the spot finds battles at exactly the
                        // same rate.
                        //
                        // `wander_action` is the existing answer and `CatchPokemon` and `SweepDex` have
                        // both used it for as long as they have existed — it targets only
                        // Empty/Grass/Water and never a warp or a connection, and `agent.rs` turns a
                        // plain-floor destination straight into `PacingForEncounters` between two
                        // neighbouring tiles. This arm was simply never pointed at it.
                        .or_else(|| state.map.wander_action())
                    {
                        Some(action)
                    } else {
                        // ⚠️ **The pacing branch above is for a *cave*, and letting a route into it
                        // is an infinite loop that generates nothing.** Encounters outdoors fire only
                        // in grass, so on a route whose grass cannot be reached the farthest
                        // "reachable object" is the way off the map: the trainee walks out, the arm
                        // above routes it back because it is no longer `on_map`, and the two repeat
                        // for ever without a single battle. Observed on Route 23 at (14, 33), which
                        // is where Victory Road drops you and where the grass is not reachable:
                        // VictoryRoad2F ↔ Route23, thousands of times, queue never moving.
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
                        // ⚠️ **Approach the side the trainer is actually looking at, not always from
                        // below.** This was `PlayerFacingDirection::Up` — stand underneath — which is
                        // right for every *gym* trainer, because they all face DOWN, and is why it was
                        // never wrong until something outside a gym needed it. The Nugget Bridge is the
                        // counter-example: its five trainers alternate sides of the bridge facing LEFT
                        // and RIGHT across it (`data/maps/objects/Route24.asm`), so standing below one
                        // is not in its line of sight, no battle starts, and the "already beaten" arm
                        // below pops the step having fought nobody. The sprite carries its own facing,
                        // so there is nothing to guess: face the opposite way to it.
                        let approach = match sprite.facing {
                            crate::pokemon::sprite::SpriteFacing::Down => PlayerFacingDirection::Up,
                            crate::pokemon::sprite::SpriteFacing::Up => PlayerFacingDirection::Down,
                            crate::pokemon::sprite::SpriteFacing::Left => PlayerFacingDirection::Right,
                            crate::pokemon::sprite::SpriteFacing::Right => PlayerFacingDirection::Left,
                        };
                        match state.map.route_to_face_dir(pos, Some(approach)) {
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
                        // ⚠️ **A heal is finished when the party is full, not when the conversation
                        // lands — and this step popped on the latter.** `Interact` completes the
                        // instant it issues the walk, so every `Interact(NURSE)` in the route was a
                        // *request* to heal rather than a heal: the very next step ran while the
                        // nurse was still talking, and whether the party actually came back full
                        // depended on how long that next step happened to take. Caught by asserting
                        // it on a fixture, which came out carrying **Water Gun on 6 of 25 PP** one
                        // step after a Pokémon Centre. Bounded, because a nurse that never finishes
                        // must hand the route back rather than hold it for ever.
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
                            self.collect_item_waits = 0;
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
                            self.collect_item_waits = 0;
                            self.queue.pop_front();
                            continue;
                        }
                        // ⚠️ **Bounded, because the completion test is "the sprite went away" and a
                        // refused pickup never satisfies it.** Every item on the floor is a sprite:
                        // walk up, press A, text box, back to the overworld — and the *only* difference
                        // between a pickup that worked and one the game declined is whether the sprite
                        // is still there. So a decline is invisible to this step and it re-issues the
                        // walk for ever. Measured: a run stood at Silph Co 5F (20,16), **one tile from
                        // the Card Key at (21,16)**, for 16,763 polls until the cycle budget died —
                        // with no line in the log saying anything was wrong.
                        //
                        // ⚠️ **And the cause is almost always the bag, which is why the message says
                        // so.** Gen 1's bag is 20 *entries* and a full one refuses every pickup in the
                        // game silently (`agent::check_pending_pickup` is the other half of this, and
                        // reports it per attempt). A route that adds an item without freeing a slot
                        // breaks a pickup several legs later, which is not a connection anyone makes
                        // from a stalled run. Handing back to the queue is the right give-up — the next
                        // step walks on, and a missing key item fails loudly wherever it is needed.
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
                        self.mart_baseline = None;
                        self.queue.pop_front();
                        continue;
                    } else if let Some((money, held)) = self.mart_baseline
                        && self.mart_attempts > 0
                        && state.money == money
                        && held == state.bag.iter().find(|i| i.id == item.id).map_or(0, |i| i.quantity)
                    {
                        // ⚠️ **A visit that moved neither the money nor the bag is the wallet talking,
                        // not a dropped confirm.** `agent::affordable` buys as many as the money
                        // covers and no more, so once it covers none the next visit is identical to
                        // this one — and the retry loop below would spend three more of them before
                        // announcing a failure that had already happened. See `mart_baseline`.
                        println!(
                            "[policy] bought {} of {} from {} — the wallet covers no more",
                            held, item, map,
                        );
                        self.mart_attempts = 0;
                        self.mart_baseline = None;
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
                        self.mart_baseline = None;
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
        // Where a black-out will be reported against — see `last_battle_map`.
        self.last_battle_map = Some(state.map.map);

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
                // ⚠️ **A grind is the one step whose whole purpose is the encounter**, so the
                // obstacle reasoning above is exactly inverted for it — the same way `CatchPokemon`
                // is exempt one line up. Without this a `GrindUntilLevel` inside a center-less
                // dungeon runs from every wild it works so hard to find: measured on Victory Road 2F
                // at **375 encounters, 375 runs and not one point of experience**, with the step
                // still at the front of the queue and the harness eventually calling it a stall.
                Some(PolicyStep::GrindUntilLevel { .. }) => false,
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
            Some(&PolicyStep::GrindUntilLevel { target, .. }) if battle_state.battle_type == BattleType::Wild =>
                target.resolve(state),
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
        // ⚠️ **The threshold depends on whether the bag can answer, and the leave is worth taking
        // even with nowhere to go.** At 15% the run was dying to the *next* hit: a lv7 Squirtle on
        // Route 1 walked into a Pidgey at about a third of its HP, above the bar, and Gust finished
        // it in one turn. With a potion in the bag the in-battle heal arm above is the cheaper answer
        // and 15% is right; with an empty bag there is nothing else coming, so leave at a third.
        // ⚠️ And the `last_pokemon_center` that used to gate this arm gated the wrong half — running
        // is worth doing whether or not there is a Centre to walk to afterwards, and on the first
        // errands out of Pallet there is not one yet.
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
                // ⚠️ **The level gate here was doing the damage gate's job badly, and it kept the one
                // mon that could win out of the fight.** "Within eight levels of the active" is a
                // proxy for "not a sacrificial weakling", and two lines below there is a direct test
                // of exactly that — the bench mon must do at least 1.5× the active's damage *and*
                // three-shot this enemy. A lv20 Hypno against Erika's Grass/Poison passes both
                // comfortably (Psychic is 2× on Poison) and failed the proxy against a lv45
                // Blastoise, so the run lost that gym with its answer sitting on the bench. Health is
                // still required: a fainted or nearly-fainted body cannot cash the damage in.
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
        let last_resort = actions.iter().find(|a| matches!(a, BattleAction::Fight { .. }))
            .or_else(|| actions.iter().find(|a| !matches!(a, BattleAction::UseItem { .. })))
            .or_else(|| actions.iter().find(|a| matches!(a, BattleAction::Fight { .. })))
            .cloned();
        if last_resort.is_none() {
            // ⚠️ **A policy that answers `None` for ever is indistinguishable from one that is
            // thinking, and that is exactly how it presents**: the agent sits in
            // `BattleState::AwaitingPolicy` showing the main battle menu, the emulator runs, the
            // watchdog never fires (it *is* being polled) and **nothing is printed**. Two
            // `full_playthrough` runs ended that way, in silence, in Erika's gym. So the one path out
            // of this function that answers nothing says so.
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
        if let Some(&PolicyStep::GrindUntilLevel { on_map, target, .. }) = self.queue.front() {
            // ⚠️ **The trainee **leads** the grind rather than being switched into it, and that is
            // worth two separate things.** `pick_battle_action`'s training block puts a bench trainee
            // in on turn 1 of every battle, which costs the turn *and* halves the experience:
            // `DivideExpDataByNumMonsGainingExp` splits a knockout between everything that took the
            // field, so a lead that switches out has still participated and still takes its half. On
            // the gauntlet grind that is the difference between one wild battle per level-step and
            // two, plus a free enemy attack in each — and it is exactly what a run watching itself
            // grind Articuno sees: every battle opens with a switch, and only the Venusaur leg (which
            // is already slot 0) runs at full speed.
            //
            // A direct RAM reorder, so it costs no menus and no game time. It is not re-issued: the
            // next poll resolves `target` to slot 0 and the guard below is false — the same
            // "completion is visible in RAM" shape `UseStrength` and `UseFlash` have, rather than a
            // latch that a restart would forget.
            //
            // ⚠️ **Only on the grind map and only while the trainee is standing.** A fainted trainee
            // is about to send the queue off on the heal detour in `pick_overworld_action`, and
            // promoting it there would put a fainted mon in front for the walk.
            if state.map.map == on_map
                && let Some(slot) = target.resolve(state)
                && slot != 0
                && state.pokemon.get(usize::from(slot)).is_some_and(|mon| mon.current_hp > 0)
            {
                println!("[policy] grind: leading with slot {slot} so it takes the whole battle and the whole XP");
                return Some(FieldMove::ReorderParty { slot });
            }
            // ⚠️ **Cure the tick here, or pay for it as a four-warp round trip to a Pokémon Centre.**
            // Poison and burn are the only things that damage a trainee *between* battles, and nothing
            // in Gen 1 clears either outside a Centre — so an uncured status is a slow countdown to a
            // faint, and the fainted-trainee detour below is what collects it. Measured before this
            // arm existed: twelve trips across the gauntlet grind, **4.9% of the whole leg**, every one
            // of them a Ponyta's burn or a Koffing's Smog rather than anything that happened in a
            // battle — the trainee won the fight that preceded each one.
            //
            // Between battles rather than in one, because a Full Heal used in battle costs the turn and
            // this costs only the bag menu. The completion test is the status itself clearing, which is
            // RAM the next poll reads — same shape as the reorder above, so a use the game somehow
            // declines cannot loop more than the driver's own escape allows.
            if state.map.map == on_map
                && let Some(slot) = target.resolve(state)
                && let Some(mon) = state.pokemon.get(usize::from(slot))
                && mon.current_hp > 0
                && matches!(mon.status, crate::pokemon::status::PokemonStatus::Poisoned
                                      | crate::pokemon::status::PokemonStatus::Burned)
                // ⚠️ **On sight, and *not* on a low-HP threshold — a Full Heal restores no HP.** The
                // first version waited until half health to save items, by analogy with the in-battle
                // arm's 25%. That analogy is wrong: a potion gives HP back and a status cure does not,
                // so curing at half leaves the trainee at half **for good** — nothing restores HP
                // outside a battle or a Centre — and the next poisoning starts from there and finishes
                // the job. Curing the moment the status lands is what keeps the HP, which is why the
                // fixture run goes from twelve Centre trips to none: a trainee that one-shots this
                // floor loses almost nothing to the battles themselves, and the tick is the whole of
                // the damage.
                && let Some(cure) = [ItemId::FullHeal, ItemId::Antidote].into_iter()
                    .find(|&id| state.bag.iter().any(|i| i.id == id && i.quantity > 0))
                // An Antidote is the cheap cure and only answers poison; a Full Heal answers both.
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
            // **Workstream H.** Same shape as `UseStrength` below: re-issued each tick until the
            // effect shows in RAM, so an interruption costs a tick rather than the step.
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
            let Some(slot) = target.resolve(state) else {
                println!("[policy] UseStrength: {target:?} is not in the party — waiting");
                return None;
            };
            let (slot, move_index) = field_move_carrier(state, PokemonMoveName::Strength)
                .unwrap_or((slot, field_move_index(state, slot, PokemonMoveName::Strength)));
            return Some(FieldMove::UseFieldMove { slot, move_index });
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
            // A machine aimed at a Pokémon outside its learnset is refused by the cartridge back into
            // the party menu with the cursor untouched, which the driver has no exit from — see the
            // ⚠️ on `FieldMove::TeachMove` in `agent.rs`. The agent refuses it there too, but this is
            // the layer that stops a scripted leg re-issuing the same impossible step every tick.
            if state.pokemon.get(target_slot as usize)
                .is_some_and(|mon| !crate::pokemon::learnset::can_learn(mon.species, item)) {
                println!("[policy] TeachMove: {target:?} cannot learn {item:?} — skipping");
                self.queue.pop_front();
                return None;
            }
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
        if let Some(&PolicyStep::Dig { target }) = self.queue.front() {
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
            let Some(slot) = target.resolve(state) else {
                println!("[policy] Dig: {target:?} is not in the party — waiting");
                return None;
            };
            let (slot, move_index) = field_move_carrier(state, PokemonMoveName::Dig)
                .unwrap_or((slot, field_move_index(state, slot, PokemonMoveName::Dig)));
            return Some(FieldMove::UseFieldMove { slot, move_index });
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

    fn pick_mart_purchase(&mut self, state: &GameState) -> Option<Option<BagItem>> {
        let result = match self.queue.front() {
            Some(PolicyStep::BuyFromMart { item, .. }) => {
                // Count this shop-open as an attempt. The `BuyFromMart` overworld arm pops the step
                // once the bag reflects the purchase (or after MAX_MART_ATTEMPTS), so we do NOT pop
                // here — a dropped YES-confirm re-opens the shop and retries. The quantity here is the
                // one the step *asked* for; the agent trims it to what the wallet can cover (see
                // `AgentState::PokemartShopping`), because Gen 1 answers an unaffordable quantity by
                // selling nothing at all.
                self.mart_attempts += 1;
                // Snapshot what this visit starts with, so the arm in `pick_overworld_action` can
                // tell a visit that bought nothing because the wallet is empty from one that bought
                // nothing because the confirm was dropped. See `mart_baseline`.
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

    /// ⚠️ **A new game is a new route, and this used to be the trait's no-op default.**
    /// `POST /api/new-run` resets the cartridge to Red's bedroom; a policy that kept its
    /// half-consumed queue would then run the *end* of the route against the *start* of the game,
    /// which is `resuming_in`'s disaster wearing different clothes. The route is rebuilt from the
    /// copy taken when the cursor was set up, and the file is deleted rather than rewritten — the
    /// run directory is a different one by now, and the `run_dir` handed in is the authority on
    /// where the next cursor goes.
    /// ⚠️ **And the queue was only the half of it that had already gone wrong.** Everything else
    /// this policy remembers is scoped to a run and none of it was being cleared, so a new game
    /// started in a live process inherited the dead one's memory: `gym_beaten` would skip gyms the
    /// fresh save has not beaten, `last_pokemon_center` and `heal_return` would send Red's bedroom
    /// detouring to Mt Moon, and `train_slot` would switch in a party slot that does not exist.
    /// It survived only because the one deployed `POST /api/new-run` happened to follow a process
    /// that had wedged at step 0 with all of it still empty. So the policy is rebuilt from its
    /// seed rather than patched field by field — a new field is then untainted by construction,
    /// which is the property a list of assignments cannot promise.
    #[cfg(feature = "web")]
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
        let slot = p.pick_move_to_forget(0, &moves, Poisonpowder).flatten().expect("should pick a slot");
        assert!(slot == 1 || slot == 2,
            "forgot slot {slot} ({:?}) — must forget a status move, not Tackle/Vine Whip", moves[slot].name);
    }

    #[test]
    fn learns_strong_move_over_status() {
        let mut p = DeterministicPolicy::new(0, Vec::<PolicyStep>::new());
        let moves = [mv(Tackle), mv(Growl), mv(LeechSeed), mv(VineWhip)];
        // Learning Razor Leaf (strong) should still forget a status slot, keeping both damaging moves.
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

#[cfg(test)]
mod heal_detour_tests {
    use super::*;
    use crate::pokemon::integration_tests::fixture::TestFixture;

    /// ⚠️ **The heal-return detour must hand back rather than wait for ever, and this is the
    /// deployed run of 2026-08-28.** It fled a wild battle on Mt Moon B2F on 4 HP, aimed itself at
    /// the Mt Moon Pokémon Centre, and `route_toward` had nothing to answer with: the incremental
    /// world graph is keyed on the entry the agent *landed* on, the fossil chamber's only two exits
    /// land on B1F 20 and 28 tiles from the single observed node, and `SNAP_THRESHOLD` is 8. The
    /// branch returned that `None` unconditionally — above the `[policy]` print — so the run went
    /// silent and motionless for hours with the emulator still running and the page still live.
    ///
    /// A fresh `WorldGraph` reproduces the condition exactly and from any fixture: it knows nothing,
    /// so there is no route to anywhere. What is asserted is that the wait is **bounded** and that
    /// the main queue is consulted again on the other side of it — the queue is a *route*, so its
    /// next step is the way out of the dungeon, which is where the Pokémon Centre is.
    #[test]
    fn a_heal_detour_that_cannot_route_hands_back_to_the_route() {
        let mut fixture = TestFixture::new(
            include_bytes!("data/mt-moon.bin"), std::time::Duration::from_secs(1), vec![]);
        let state = fixture.game_state();
        // ⚠️ **Cinnabar, not the Mt Moon Centre this case was found on, and the change is a fix
        // rather than a dodge.** The detour now falls back to the Centre's *town*
        // (`Map::pokemon_center_town`) and `route_toward` takes a transition off the current map's
        // own `actions()` before asking the graph — so from inside Mt Moon it can now walk *out*
        // toward Route 4, which is exactly what the deployed run should have done. What still cannot
        // be answered from a cave floor is a Centre on an island the run has never seen, and that is
        // the shape this test is really about: an unroutable detour must hand the route back rather
        // than hold it.
        let centre = Map::CinnabarPokecenter;
        assert_ne!(state.map.map, centre, "the fixture must be somewhere the detour has to travel to");

        // The step is `enter` the map the player is already on, which pops on sight — so the queue
        // shrinking is proof the detour let go, whatever the fixture happens to be standing on.
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

    /// ⚠️ **The bound must not fire on a detour that is working**, or a heal several maps away is
    /// abandoned part-way and the fix is worse than the bug. The counter is reset by every step the
    /// detour actually takes, so a long walk to the Nurse never runs it down.
    ///
    /// Cerulean rather than Mt Moon because the graph has to be able to *answer*: the centre is a
    /// warp out of the city the fixture is standing in, so one observed node is a route. ⚠️ **The
    /// negative test above cannot be inverted to make this one** — from inside Mt Moon no centre is
    /// an edge target of any observed node, so it returns `None` for the honest reason and an
    /// `if is_some()` around the assertions would quietly assert nothing at all.
    #[test]
    fn a_heal_detour_that_is_moving_is_never_abandoned() {
        let mut fixture = TestFixture::new(
            include_bytes!("data/back-in-cerulean.bin"), std::time::Duration::from_secs(1), vec![]);
        let state = fixture.game_state();
        let centre = Map::CeruleanPokecenter;
        assert_eq!(state.map.map, Map::CeruleanCity, "the fixture moved out from under this test");

        let mut policy = DeterministicPolicy::new(42, vec![PolicyStep::enter(state.map.map)]);
        policy.heal_return = Some(centre);
        // A graph that can route: one observed node here, carrying the exits the agent can actually
        // see from where it stands — the centre's door among them.
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
}

/// The scripted route's cursor: what makes a rollout survivable, and what stops a changed route
/// being replayed over a game that is part-way through the old one.
#[cfg(all(test, feature = "web"))]
mod scripted_progress_tests {
    use super::*;
    use crate::run::tests::Scratch;

    fn route() -> Vec<PolicyStep> {
        vec![
            PolicyStep::enter(Map::PalletTown),
            PolicyStep::enter(Map::Route1),
            PolicyStep::enter(Map::ViridianCity),
            PolicyStep::enter(Map::Route2),
            PolicyStep::enter(Map::PewterCity),
        ]
    }

    /// A process starting this run **from the beginning of the game** — `Origin::Fresh`.
    fn policy_in(dir: &std::path::Path, steps: Vec<PolicyStep>) -> DeterministicPolicy {
        DeterministicPolicy::new(42, steps).resuming_in(dir, true)
    }

    /// A process **resuming** this run from a checkpoint — `Origin::Resumed`, the rollout case.
    fn resumed_policy_in(dir: &std::path::Path, steps: Vec<PolicyStep>) -> DeterministicPolicy {
        DeterministicPolicy::new(42, steps).resuming_in(dir, false)
    }

    /// ⚠️ **The whole point, in three lines.** Before this a restart mid-route rebuilt the queue at
    /// step 0 while the save resumed where it was, so the deployed scripted run would have replayed
    /// Red's bedroom against a game halfway to Vermilion.
    #[test]
    fn a_restarted_process_resumes_the_route_where_it_left_off() {
        let scratch = Scratch::new("scripted-progress");

        let mut first = policy_in(&scratch.0, route());
        assert_eq!(first.steps_remaining(), Some(5));
        // Three steps land. `record_progress` is driven by the overworld poll in the real thing;
        // here the queue is advanced directly so the test is about the cursor and nothing else.
        first.queue.drain(..3);
        first.record_progress();

        let second = resumed_policy_in(&scratch.0, route());
        assert_eq!(second.steps_remaining(), Some(2), "the route resumes at step 3 of 5");
        assert_eq!(second.queue.front(), Some(&PolicyStep::enter(Map::Route2)));
    }

    /// A run that has never recorded a cursor is a new game, and must start at the beginning rather
    /// than be treated as an error.
    #[test]
    fn a_run_with_no_cursor_starts_at_the_beginning() {
        let scratch = Scratch::new("scripted-progress-fresh");
        let policy = policy_in(&scratch.0, route());
        assert_eq!(policy.steps_remaining(), Some(5));
        // ⚠️ And it records one immediately, so the *next* restart resumes rather than guessing.
        assert!(scratch.0.join(scripted_progress::FILE).exists(), "step 0 is still a cursor");
    }

    /// ⚠️ **A run being *resumed* with no cursor parks too, and this one shipped.** A run started
    /// before cursors existed has no file and a save in the middle of the game; "no file means a new
    /// game" walked the rollout of 2026-08-28 into restarting the route in Red's bedroom against a
    /// save standing in Victory Road, where it failed to route for 745 polls. The absence of the
    /// file cannot tell the two apart, so the caller says which it is.
    #[test]
    fn a_resumed_run_with_no_cursor_parks_rather_than_replaying_the_route() {
        let scratch = Scratch::new("scripted-progress-cursorless");
        let parked = resumed_policy_in(&scratch.0, route());
        assert_eq!(parked.steps_remaining(), Some(0), "parked");
        assert!(parked.is_exhausted(), "a parked policy stops answering, which is what parks the run");
    }

    /// ⚠️ **A changed route parks the run; it must never replay from 0.** A cursor counts steps in
    /// one particular list, so against a different list it means nothing — and "start over" against
    /// a mid-game save is the failure the whole mechanism exists to prevent, not a fallback from it.
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

    /// ⚠️ **`POST /api/new-run` is the same desync from the other side.** `Policy::restart` defaults
    /// to a no-op and this policy never overrode it, so a new game got the *end* of the old route.
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
        // The cursor follows the run: the old directory's is gone, the new one's says step 0.
        assert!(!scratch.0.join(scripted_progress::FILE).exists(), "the old run's cursor is not left behind");
        assert!(next.0.join(scripted_progress::FILE).exists(), "the new run records its own");
    }

    /// ⚠️ **And a new run must forget what the old one learned, not just its queue.** Every field
    /// here is scoped to a run, and `restart` used to reset only the queue — so a game started over
    /// in a live process kept the dead run's badges (skipping gyms the fresh save has not beaten),
    /// its last Pokémon Centre, a heal detour aimed across the map, and a training slot pointing at
    /// a party member that no longer exists. Rebuilding from the seed is what makes a field added
    /// later untainted without anyone remembering to come back here.
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

    /// ⚠️ **Not `DefaultHasher`.** The fingerprint is compared across processes and across builds,
    /// so it has to be a function of the steps alone — a toolchain bump that changed it would park
    /// every running scripted deployment.
    #[test]
    fn the_route_fingerprint_is_about_the_route_and_nothing_else() {
        assert_eq!(scripted_progress::fingerprint(&route()), scripted_progress::fingerprint(&route()));
        let mut different = route();
        different[0] = PolicyStep::enter(Map::ViridianCity);
        assert_ne!(scripted_progress::fingerprint(&route()), scripted_progress::fingerprint(&different));
        // The real one, so a fingerprint that silently collapsed to a constant would show up here.
        assert_ne!(
            scripted_progress::fingerprint(&PolicyStep::complete_game_steps()),
            scripted_progress::fingerprint(&route()),
        );
    }
}
