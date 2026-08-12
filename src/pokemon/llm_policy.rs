//! **W4 / §7.2** — the [`Policy`] the LLM drives.
//!
//! Modelled on `ConsolePolicy`, which has been the reference for a non-blocking asynchronous policy
//! since the start: kick the question off on the first call, return `None` every tick, `try_recv`
//! until the answer lands. The differences are all consequences of one rule:
//!
//! > **A turn is keyed by the decision kind it is answering, and only a poll for that same kind may
//! > advance it.**
//!
//! That is what makes it safe for the emulator to keep running while the model thinks. The agent
//! asks for an overworld action, the model spends eight seconds on it, and meanwhile a trainer spots
//! the player: the very next poll is `pick_battle_action`, the kind no longer matches, the stale turn
//! is cancelled and a battle turn starts. A battle decision can never be applied to an overworld
//! state, and no tokens are spent finishing a completion that is already answering a dead question.
//!
//! ⚠️ **`pending` is the re-issue guard and it is load bearing.** `agent.update` polls the policy up
//! to fifty times per emulated second (see W0.3b — deliberately not throttled). Without the guard,
//! one decision point would spawn fifty LLM turns. Both `service_tools` and the `pick_*` path have to
//! be cheap no-ops when there is nothing to do, because both run at that rate.
//!
//! ⚠️ **`pick_field_move` shares the `Overworld` kind and must never become a kind of its own.** It
//! is called on *every* idle overworld tick immediately before `pick_overworld_action`; given its own
//! kind the two would cancel each other fifty times a second and no turn would ever complete. W5's
//! `use_field_move` is therefore an *outcome* of an overworld turn: the decision is stashed and this
//! method hands it over on the next tick without touching `pending`, `waiting` or `site`.

use std::sync::atomic::Ordering;

use crate::joypad::JoypadButton;
use crate::llm::prompt::{self, ApiSnapshot, TurnContext};
use crate::llm::tools::{self, DecisionKind, Terminal};
use crate::llm::worker::{ToolBatchResult, TurnHandles, TurnRequest};
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::agent::AgentEvent;
use crate::pokemon::bag::BagItem;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::policy::{FieldMove, Policy};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::world_graph::WorldGraph;
use crate::pokemon::{GameState, PokemonApi};

pub struct LlmPolicy {
    handles: TurnHandles,
    /// The kind the in-flight turn answers, and its generation. `None` means nothing is in flight,
    /// which is the only state in which a new turn may start.
    pending: Option<(DecisionKind, u64)>,
    /// A `wait` decision in progress: which question it was answering, and how many agent ticks are
    /// left on it.
    ///
    /// Counted down by the `pick_*` methods only — `service_tools` and `pick_field_move` run on the
    /// same ticks and must not consume it. Keyed by kind for the same reason a turn is: a wait is an
    /// answer to the question that was asked, so a battle starting part-way through an overworld
    /// wait should not have to sit out the remainder of it.
    waiting: Option<(DecisionKind, u16)>,
    /// Everything the agent has said since the last turn was built, folded into the next one.
    /// Rendered on arrival — see [`prompt::describe_event`].
    events: Vec<String>,
    /// The half of the situation that needs a `PokemonApi`. Refreshed at the poll immediately before
    /// a turn is built, which is the only moment this policy is handed one.
    snapshot: ApiSnapshot,
    /// The last `GameState` seen at a poll where a turn could start.
    ///
    /// ⚠️ **Two of the five poll sites are handed no state at all** — `pick_nickname` gets a species
    /// and `pick_move_to_forget` gets four moves — so a turn asked from either of them has to build
    /// its situation from somewhere. `service_tools` runs immediately before every one of the five,
    /// with the state the agent has just read, so this is that state and it is never more than one
    /// tick old.
    state: Option<Box<GameState>>,
    /// Which of the five poll sites was asked last. The three menu prompts are invisible in a
    /// `GameState`, so this is the only thing that can tell `service_tools` which question a batch
    /// belongs to — see [`Self::observed_kind`].
    site: Option<DecisionKind>,
    /// A decided [`FieldMove`], waiting for the `pick_field_move` that will collect it.
    ///
    /// ⚠️ It has to be stashed rather than returned, because `pick_overworld_action` — the site that
    /// decided it — cannot return a field move, and `pick_field_move` runs *before* it on the next
    /// tick rather than after it on this one.
    field_move: Option<FieldMove>,
    /// Raw presses waiting for the agent to collect them at the top of its next tick
    /// ([`Policy::take_manual_input`]).
    manual: Vec<JoypadButton>,
    /// Prepended to the next turn: what went wrong with the last decision, in the model's own terms.
    note: Option<String>,
    /// **W9** — `GB_STUCK_TIMEOUT_SECS`, handed to the agent once at construction
    /// ([`Policy::stuck_timeout`]). `None` turns the watchdog off entirely.
    stuck_timeout: Option<std::time::Duration>,
}

impl LlmPolicy {
    pub fn new(handles: TurnHandles, stuck_timeout: Option<std::time::Duration>) -> Self {
        Self {
            handles,
            stuck_timeout,
            pending: None,
            waiting: None,
            events: Vec::new(),
            snapshot: ApiSnapshot::default(),
            state: None,
            site: None,
            field_move: None,
            manual: Vec::new(),
            note: None,
        }
    }

    /// Which question *this* poll site is asking.
    ///
    /// The plan has `service_tools` compare the pending kind against "the kind about to be asked",
    /// which the seam's signature does not carry. Two things answer it between them:
    ///
    /// - **The three menu prompts are not in the state**, and neither is W9's `Stuck`. A naming
    ///   screen, a mart's Buy/Sell menu and the forget-move prompt all look like an ordinary
    ///   overworld or battle `GameState`, and a wedged agent looks like whatever it was doing when
    ///   it wedged — so the only evidence is which site ran last. That is [`Self::site`], and it is
    ///   right for every poll of a decision point except the first after the site changes.
    /// - **A battle is in the state**, and is the whole difference between the other two kinds — so
    ///   they are read from it, which detects a battle starting one tick *earlier* than `site` would.
    ///
    /// Being wrong either way costs one wasted round trip, never a decision applied to the wrong
    /// state: the `pick_*` re-checks the kind before it accepts an outcome.
    fn observed_kind(&self, state: &GameState) -> DecisionKind {
        match self.site {
            Some(site) if site.is_inferred_from_the_site() => site,
            _ => match state.battle.is_some() {
                true => DecisionKind::Battle,
                false => DecisionKind::Overworld,
            },
        }
    }

    /// The shared half of `pick_overworld_action` and `pick_battle_action`.
    ///
    /// Returns the decision to apply, or `None` for "not ready — ask again next tick", which is
    /// every one of: waiting out a `wait`, a turn still in flight, and a turn only just started.
    fn advance(&mut self, kind: DecisionKind, context: TurnContext<'_>) -> Option<Terminal> {
        // Recorded before anything else: this is what tells the *next* tick's `service_tools` which
        // question a tool batch belongs to.
        self.site = Some(kind);

        match self.waiting {
            Some((waiting_on, ticks)) if waiting_on == kind => {
                self.waiting = (ticks > 1).then_some((kind, ticks - 1));
                return None;
            }
            // The wait was answering the other question. It is spent, not carried over.
            Some(_) => self.waiting = None,
            None => {}
        }

        match self.pending {
            // The turn in flight is answering this very question.
            Some((pending, id)) if pending == kind => match self.handles.outcomes.try_recv() {
                Ok(outcome) if outcome.id == id => {
                    self.pending = None;
                    Some(outcome.decision)
                }
                // An outcome from a turn already abandoned. It crossed the cancellation on the wire;
                // dropping it is the whole point of stamping turns with a generation.
                Ok(_) => None,
                Err(_) => None,
            },
            // A different question is being asked now, or none was. Cancelling costs the tokens
            // already spent — §17's risk 2b, which is why `TurnCancelled` is an event, not a silence.
            _ => {
                self.start_turn(kind, context);
                None
            }
        }
    }

    /// Bump the generation — which is what cancels anything in flight — and send a fresh turn.
    fn start_turn(&mut self, kind: DecisionKind, context: TurnContext<'_>) {
        // Everything that reads `self` immutably happens inside this block, so the mutations below
        // it are free of the borrow. `situation` and `headline` come out owned.
        let Some((mut situation, headline)) = ({
            // No state has been observed yet, so there is nothing to describe. `service_tools` runs
            // immediately before every poll site, so this is only ever true before the first tick.
            self.state.as_deref().map(|state| {
                let menu = match kind {
                    DecisionKind::Overworld => tools::overworld_menu(state),
                    DecisionKind::Battle => tools::battle_menu(state),
                    DecisionKind::MartPurchase => tools::mart_menu(&self.snapshot),
                    DecisionKind::ForgetMove => match context {
                        TurnContext::ForgetMove { current, .. } => tools::forget_menu(current),
                        _ => Vec::new(),
                    },
                    // The naming screen offers no choices; the tool's own arguments are the menu.
                    // Neither does W9's `Stuck`, and there the absence *is* the situation.
                    DecisionKind::Nickname | DecisionKind::Stuck => Vec::new(),
                };
                let situation =
                    prompt::situation(kind, state, &self.snapshot, &self.events, &menu, context);
                let headline = format!(
                    "{} — {} at ({}, {})",
                    kind.label(),
                    state.map.map,
                    state.map.player_position.x,
                    state.map.player_position.y,
                );
                (situation, headline)
            })
        }) else {
            return;
        };

        let id = self.handles.next_generation();
        if let Some(note) = self.note.take() {
            situation = format!("{note}\n\n{situation}");
        }
        self.events.clear();

        if self.handles.turns.send(TurnRequest { id, kind, situation, headline }).is_ok() {
            self.pending = Some((kind, id));
        }
        // If the send failed the worker has gone. `pending` stays `None`, so the next poll tries
        // again — and keeps trying, which is the right shape: the run is broken, and the operator
        // finds out from the worker's own error rather than from the agent quietly standing still.
    }

    /// The decision could not be carried out. Tell the model why on its next turn rather than
    /// silently doing nothing — §7.4: an id with no match is a message, not a panic and not a no-op.
    fn reject(&mut self, note: String) {
        self.note = Some(note);
    }
}

impl Policy for LlmPolicy {
    fn name(&self) -> &'static str { "llm" }

    /// ⚠️ Runs at every poll of every decision point — fifty times a second — so the common path
    /// here is a snapshot and an empty `try_recv`.
    fn service_tools(&mut self, state: &GameState, api: &mut PokemonApi<'_>, graph: &WorldGraph) {
        let live = self.handles.current_generation();
        let asking = self.observed_kind(state);

        // This is the one moment the policy is handed a `PokemonApi`, and the only source of the
        // situation a turn started from any of the five poll sites will be built from.
        //
        // ⚠️ **Unconditional, and W4's "only when nothing is pending" guard was wrong.** Every
        // version of that guard has to predict whether *this* poll is the first of a new decision
        // point, and it cannot: the site is only known once the `pick_*` after this one runs. Two
        // cases broke it — a battle interrupting an overworld turn built its menu from the overworld
        // state it replaced, and a mart opening during an overworld turn rendered a stock list read
        // before the player reached the shop. The cost of being right is a `GameState` clone and one
        // VRAM text decode per poll; `LlmPolicy` only ever runs at **1× real time** (it is the
        // livestream's policy), so that is fifty of each per wall-clock second and the emulator under
        // it is doing nothing else with the other 95% of the time.
        self.snapshot = ApiSnapshot::read(api);
        self.state = Some(Box::new(state.clone()));

        while let Ok(batch) = self.handles.tool_calls.try_recv() {
            let current = batch.turn == live
                && self.pending.is_some_and(|(kind, id)| kind == asking && id == batch.turn);
            let result = match current {
                // ⚠️ **All-or-nothing, from one observed state.** Every call in the batch is answered
                // against the same `state`, which is what guarantees `read_party` and `read_map` in
                // one assistant message agree — and what makes the worker's single-step rollback
                // sufficient, since a batch can never be half-answered.
                true => ToolBatchResult::Answered(
                    batch.calls.iter().map(|call| tools::service_read(call, state, api, graph)).collect(),
                ),
                // The tool is never executed. The worker rolls back one step and abandons the turn.
                false => ToolBatchResult::Cancelled,
            };
            let _ = self.handles.tool_results.send(result);
        }
    }

    fn pick_overworld_action(&mut self, state: &GameState, _graph: &WorldGraph) -> Option<OverworldAction> {
        match self.advance(DecisionKind::Overworld, TurnContext::None)? {
            Terminal::ChooseAction { id } => {
                // ⚠️ Resolved against a **freshly recomputed** action list, never against the one the
                // menu was rendered from: `actions()` is sorted by `MetaTile` and the world has been
                // running for however long the turn took.
                match tools::resolve_overworld(state, &id) {
                    Some(action) => Some(action),
                    None => {
                        self.reject(format!(
                            "`{id}` is no longer available — the game moved on while you were \
                             deciding. Here is the current situation; pick again."
                        ));
                        None
                    }
                }
            }
            // Stashed, not returned: this method's return type is a walk, and a field move is not
            // one. `pick_field_move` collects it on the next tick — 20 ms later — and hands it
            // straight to the agent.
            Terminal::UseFieldMove(request) => {
                match tools::resolve_field_move(state, &request) {
                    Ok(field_move) => self.field_move = Some(field_move),
                    Err(complaint) => self.reject(complaint),
                }
                None
            }
            Terminal::PressButtons { buttons } => {
                self.manual.extend(buttons);
                None
            }
            Terminal::Wait { ticks } => {
                self.waiting = Some((DecisionKind::Overworld, ticks));
                None
            }
            // Unreachable while the tools array is scoped per kind (§7.5), which is exactly why the
            // scoping is the first line of defence rather than the only one.
            other => {
                self.reject(format!("`{other:?}` cannot be used in the overworld."));
                None
            }
        }
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        match self.advance(DecisionKind::Battle, TurnContext::None)? {
            Terminal::ChooseBattleAction { id } => match tools::resolve_battle(state, &id) {
                Some(action) => Some(action),
                None => {
                    self.reject(format!(
                        "`{id}` is no longer a legal battle action — the battle moved on while you \
                         were deciding. Here is the current menu; pick again."
                    ));
                    None
                }
            },
            Terminal::PressButtons { buttons } => {
                self.manual.extend(buttons);
                None
            }
            Terminal::Wait { ticks } => {
                self.waiting = Some((DecisionKind::Battle, ticks));
                None
            }
            other => {
                self.reject(format!("`{other:?}` cannot be used in a battle."));
                None
            }
        }
    }

    /// ⚠️ **Not a decision point, and must never become one.** This runs on every idle overworld tick
    /// immediately before `pick_overworld_action`; it neither starts a turn nor touches `pending`,
    /// `waiting` or `site`. All it does is hand over what an overworld turn already decided.
    fn pick_field_move(&mut self, _state: &GameState) -> Option<FieldMove> {
        self.field_move.take()
    }

    fn pick_nickname(&mut self, species: PokemonSpecies) -> Option<Option<String>> {
        match self.advance(DecisionKind::Nickname, TurnContext::Nickname(species))? {
            Terminal::SetNickname { name } => Some(name),
            Terminal::Wait { ticks } => {
                self.waiting = Some((DecisionKind::Nickname, ticks));
                None
            }
            other => {
                self.reject(format!("`{other:?}` cannot answer the naming screen."));
                None
            }
        }
    }

    fn pick_mart_purchase(&mut self, _state: &GameState) -> Option<Option<BagItem>> {
        match self.advance(DecisionKind::MartPurchase, TurnContext::None)? {
            // ⚠️ The quantity is **not** trimmed to the wallet here — `assert_pokemart_state` does
            // that against the ROM's own price table, because Gen 1 hands over *nothing* for an
            // order it cannot afford and the agent has been trimming since long before this policy.
            Terminal::BuyItem { item } => Some(item),
            Terminal::Wait { ticks } => {
                self.waiting = Some((DecisionKind::MartPurchase, ticks));
                None
            }
            other => {
                self.reject(format!("`{other:?}` cannot answer a mart menu."));
                None
            }
        }
    }

    fn pick_move_to_forget(
        &mut self,
        current_moves: &[PokemonMove],
        new_move: PokemonMoveName,
    ) -> Option<Option<usize>> {
        let context = TurnContext::ForgetMove { current: current_moves, new: new_move };
        match self.advance(DecisionKind::ForgetMove, context)? {
            Terminal::ForgetMove { slot } => match slot {
                // A slot the mon does not have would be navigated to and never reached, so the
                // cursor drive would loop until the prompt timed out. Declining is the safe answer,
                // and the model is told why on its next turn.
                Some(slot) if slot as usize >= current_moves.len() => {
                    self.reject(format!(
                        "Slot {slot} is not one of the {} moves that Pokémon knows, so nothing was \
                         forgotten and the new move was declined.",
                        current_moves.len(),
                    ));
                    Some(None)
                }
                Some(slot) => Some(Some(slot as usize)),
                None => Some(None),
            },
            Terminal::Wait { ticks } => {
                self.waiting = Some((DecisionKind::ForgetMove, ticks));
                None
            }
            other => {
                self.reject(format!("`{other:?}` cannot answer the forget-move prompt."));
                None
            }
        }
    }

    /// **W9 / §14** — the sixth kind, asked by the watchdog rather than by a poll site.
    ///
    /// Structurally an ordinary turn: [`Self::advance`] keys it, cancels anything in flight for a
    /// different question, and counts down a `wait` the same way. The two differences are that it
    /// returns nothing — a jammed agent can carry out no decision, so the answer leaves by
    /// [`Policy::take_manual_input`] — and that it is asked on every tick of the jam rather than at
    /// a decision point, which is what gives the turn's tool batch somewhere to be serviced.
    ///
    /// ⚠️ **A `wait` here is not free of consequence.** It sits out `ticks` and then the watchdog
    /// asks again, because the agent is still stuck; that is the intended shape (the model may
    /// reasonably believe the game needs a moment), but a model that answers `wait` forever spends a
    /// turn every few seconds doing it. `TurnCancelled` and this kind's share of the turn count are
    /// what make that visible.
    fn pick_unstick(&mut self, _state: &GameState, jam: crate::pokemon::policy::Jam<'_>) {
        let context = TurnContext::Stuck { agent_state: jam.agent_state, stuck_for: jam.stuck_for };
        match self.advance(DecisionKind::Stuck, context) {
            Some(Terminal::PressButtons { buttons }) => self.manual.extend(buttons),
            Some(Terminal::Wait { ticks }) => self.waiting = Some((DecisionKind::Stuck, ticks)),
            Some(other) => self.reject(format!(
                "`{other:?}` cannot be used while the agent is stuck — only `press_buttons` and \
                 `wait` can."
            )),
            None => {}
        }
    }

    fn stuck_timeout(&self) -> Option<std::time::Duration> {
        self.stuck_timeout
    }

    /// **`POST /api/new-run`** — the emulator has reloaded the game from the start under us.
    ///
    /// Everything this policy holds is about a decision in the game that just ended: a turn in
    /// flight deciding a battle that no longer exists, a `field_move` stashed for a
    /// `pick_field_move` that will never come, presses queued for a player who is somewhere else
    /// entirely. All of it goes.
    ///
    /// ⚠️ **Bump the generation first.** It is what cancels the in-flight turn, and it is also what
    /// makes the outcome already on the wire safe: a stale `TurnOutcome` reaching a later poll no
    /// longer matches any pending id, so it is dropped instead of being applied to the new game.
    /// The worker is told separately — its history and the model's notes are its own to replace, and
    /// it does so at the top of its next turn (see [`Restart`](crate::llm::worker::Restart)).
    fn restart(&mut self, run_dir: Option<&std::path::Path>) {
        self.handles.next_generation();
        if let Ok(mut cell) = self.handles.restart.lock() {
            *cell = Some(crate::llm::worker::Restart {
                run_dir: run_dir.map(|path| path.to_path_buf()),
            });
        }
        self.pending = None;
        self.waiting = None;
        self.events.clear();
        self.snapshot = ApiSnapshot::default();
        self.state = None;
        self.site = None;
        self.field_move = None;
        self.manual.clear();
        self.note = None;
    }

    /// Collected by the agent at the top of its next tick, ahead of the state machine.
    fn take_manual_input(&mut self) -> Vec<JoypadButton> {
        std::mem::take(&mut self.manual)
    }

    /// The narrative between decisions: dialogue, a battle starting, and above all the abort reasons
    /// that tell a model to stop re-picking a route that cannot be walked.
    fn on_event(&mut self, event: &AgentEvent) {
        // A conversation can run for hundreds of boxes while no decision is asked for. The renderer
        // keeps the most recent twenty; this keeps the buffer from growing without bound in between.
        const MAX_BUFFERED: usize = 64;
        if self.events.len() >= MAX_BUFFERED {
            self.events.remove(0);
        }
        self.events.push(prompt::describe_event(event));
    }
}

impl Drop for LlmPolicy {
    /// The emulator thread is ending. Bump the generation so a worker blocked mid-stream stops
    /// rather than finishing a completion nobody will read; dropping the channels then ends its loop.
    fn drop(&mut self) {
        self.handles.generation.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use crate::game_boy::GameBoy;
    use crate::llm::LlmError;
    use crate::llm::client::ChatEndpoint;
    use crate::llm::config::LlmConfig;
    use crate::llm::protocol::{ChatRequest, Completion, FunctionCall, Message, Role, ToolCall};
    use crate::llm::worker;
    use crate::pokemon::PokemonApiTrait;
    use crate::pokemon::actions::OverworldAction;
    use crate::web::published::{Published, RunStatus, UiEvent, UiEventBody};

    // ── A scripted endpoint ──────────────────────────────────────────────────────────────────────

    /// One reply, and whether it makes the caller wait for permission first — which is how a test
    /// gets a turn to be genuinely *in flight* while it does something else.
    struct Reply {
        completion: Completion,
        release: Option<Arc<AtomicBool>>,
    }

    #[derive(Default)]
    struct Scripted {
        replies: Mutex<VecDeque<Reply>>,
        seen: Mutex<Vec<ChatRequest>>,
    }

    impl ChatEndpoint for Scripted {
        fn stream_completion(
            &self,
            request: &ChatRequest,
            on_delta: &mut dyn FnMut(&str),
            cancelled: &dyn Fn() -> bool,
        ) -> Result<Completion, LlmError> {
            self.seen.lock().unwrap().push(request.clone());
            let Some(reply) = self.replies.lock().unwrap().pop_front() else {
                // Out of script. `Cancelled` leaves the turn unanswered, so an over-running test ends
                // in its pump's timeout — a clear failure — rather than in a panic on another thread.
                return Err(LlmError::Cancelled);
            };
            if let Some(release) = reply.release {
                // A real stream checks `cancelled` on every line; so does this one, which is what
                // makes the cancellation path the same path production takes.
                while !release.load(Ordering::SeqCst) {
                    if cancelled() {
                        return Err(LlmError::Cancelled);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            if !reply.completion.content.is_empty() {
                on_delta(&reply.completion.content);
            }
            Ok(reply.completion)
        }
    }

    fn says(text: &str) -> Reply {
        Reply { completion: Completion { content: text.into(), ..Completion::default() }, release: None }
    }

    fn calls(pairs: &[(&str, &str)]) -> Reply {
        let tool_calls = pairs
            .iter()
            .enumerate()
            .map(|(i, (name, arguments))| ToolCall {
                id: format!("call_{i}"),
                kind: "function".into(),
                function: FunctionCall { name: (*name).into(), arguments: (*arguments).into() },
            })
            .collect();
        Reply { completion: Completion { tool_calls, ..Completion::default() }, release: None }
    }

    /// A reply that says something *and* calls a tool. The prose is what makes it possible to write a
    /// turn of a chosen size, which is how the compaction test reaches its threshold without a
    /// hundred thousand tokens of fixture.
    fn saying_calls(text: &str, pairs: &[(&str, &str)]) -> Reply {
        let mut reply = calls(pairs);
        reply.completion.content = text.to_string();
        reply
    }

    fn held(mut reply: Reply, release: &Arc<AtomicBool>) -> Reply {
        reply.release = Some(Arc::clone(release));
        reply
    }

    // ── The rig ──────────────────────────────────────────────────────────────────────────────────

    /// A real `GameState` without a running emulator: the fixture is loaded and read once, which
    /// costs milliseconds. These tests are about the turn protocol, not about the game moving.
    struct Rig {
        gb: GameBoy,
        graph: WorldGraph,
        endpoint: Arc<Scripted>,
        published: Arc<Published>,
        events: std::sync::mpsc::Receiver<UiEvent>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    /// Oak's lab just after the starter is chosen: a party of one, and a map with several reachable
    /// actions, which is what an overworld menu needs to be worth asking about.
    const FIXTURE: &[u8] = include_bytes!("data/oaks-lab-just-got-squirtle.bin");

    /// Mid-battle, which is the whole difference between the two decision kinds — and therefore the
    /// only way to exercise the cancellation path honestly.
    const IN_BATTLE: &[u8] = include_bytes!("data/battle-state.bin");

    impl Rig {
        fn new(script: Vec<Reply>) -> (Self, LlmPolicy) {
            Self::with_config(script, |_| {})
        }

        /// [`Self::new`] with the chance to change the config first — which in practice means
        /// `context_limit`, because a compaction test that had to fill a real 128 k window would
        /// have to send a hundred thousand tokens of fixture through a scripted endpoint.
        fn with_config(script: Vec<Reply>, tweak: impl FnOnce(&mut LlmConfig)) -> (Self, LlmPolicy) {
            let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
            gb.load_state(FIXTURE).expect("the committed fixture loads");

            let endpoint = Arc::new(Scripted {
                replies: Mutex::new(script.into_iter().collect()),
                seen: Mutex::new(Vec::new()),
            });
            let published = Published::new();

            // The broadcast receiver is drained onto an mpsc so a test can read the whole run's
            // events at the end without racing the ring buffer.
            let (tx, events) = std::sync::mpsc::channel();
            let mut broadcast = published.subscribe_events();
            std::thread::spawn(move || {
                while let Ok(event) = broadcast.blocking_recv() {
                    if tx.send(event).is_err() {
                        break;
                    }
                }
            });

            let mut config = LlmConfig {
                base_url: "http://scripted".into(),
                api_key: "none".into(),
                model: "scripted".into(),
                context_limit: 128_000,
                temperature: 1.0,
                max_tool_steps: 4,
                stuck_timeout: Some(Duration::from_secs(300)),
            };
            tweak(&mut config);
            // Read off before the worker takes the config, exactly as `web/mod.rs` does.
            let config_stuck_timeout = config.stuck_timeout;
            let (worker, handles) = worker::channels(
                Box::new(Forwarding(Arc::clone(&endpoint))),
                config,
                Arc::clone(&published),
                // No run directory: the note tools work, they simply keep nothing (W6b).
                crate::llm::notes::Notes::open(None),
            );
            let handle = worker.spawn().expect("the worker thread starts");

            let rig = Rig {
                gb,
                graph: WorldGraph::new(),
                endpoint,
                published,
                events,
                worker: Some(handle),
            };
            (rig, LlmPolicy::new(handles, config_stuck_timeout))
        }

        /// A trainer just spotted the player. Swapping the loaded state is exactly what that looks
        /// like from the policy's side, and it costs no emulation.
        fn enter_battle(&mut self) {
            self.gb.load_state(IN_BATTLE).expect("the committed battle fixture loads");
            assert!(self.state().battle.is_some(), "battle-state.bin should be mid-battle");
        }

        fn state(&mut self) -> GameState {
            PokemonApi::new(&mut self.gb).game_state().expect("the fixture has a readable state")
        }

        /// One agent tick's worth of policy: the tool poll, then the decision poll — in the order
        /// `agent.rs` calls them.
        fn tick_overworld(&mut self, policy: &mut LlmPolicy) -> Option<OverworldAction> {
            let state = self.state();
            let mut api = PokemonApi::new(&mut self.gb);
            policy.service_tools(&state, &mut api, &self.graph);
            drop(api);
            policy.pick_overworld_action(&state, &self.graph)
        }

        fn tick_battle(&mut self, policy: &mut LlmPolicy) -> Option<crate::pokemon::battle::BattleAction> {
            let state = self.state();
            let mut api = PokemonApi::new(&mut self.gb);
            policy.service_tools(&state, &mut api, &self.graph);
            drop(api);
            policy.pick_battle_action(&state)
        }

        /// The three menu prompts, each in the order `agent.rs` polls it: `service_tools`, then the
        /// one `pick_*` that site asks. `ask` runs the second half so one helper serves all three.
        fn tick_prompt<T>(
            &mut self,
            policy: &mut LlmPolicy,
            ask: impl FnOnce(&mut LlmPolicy, &GameState) -> Option<T>,
        ) -> Option<T> {
            let state = self.state();
            let mut api = PokemonApi::new(&mut self.gb);
            policy.service_tools(&state, &mut api, &self.graph);
            drop(api);
            ask(policy, &state)
        }

        /// **W9** — one tick of a jammed agent, in the order `agent.rs::run_watchdog` does it:
        /// `service_tools`, then `pick_unstick`. Note what it does *not* do — return anything. The
        /// answer to a stuck turn leaves by `take_manual_input`.
        fn tick_stuck(&mut self, policy: &mut LlmPolicy, agent_state: &str) {
            let state = self.state();
            let mut api = PokemonApi::new(&mut self.gb);
            policy.service_tools(&state, &mut api, &self.graph);
            drop(api);
            let jam = crate::pokemon::policy::Jam {
                agent_state,
                stuck_for: Duration::from_secs(300),
            };
            policy.pick_unstick(&state, jam);
        }

        /// Poll a menu prompt like the agent does until it answers or time runs out.
        fn pump_prompt<T>(
            &mut self,
            policy: &mut LlmPolicy,
            mut ask: impl FnMut(&mut LlmPolicy, &GameState) -> Option<T>,
        ) -> Option<T> {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if let Some(answer) = self.tick_prompt(policy, &mut ask) {
                    return Some(answer);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            None
        }

        fn pump_battle(&mut self, policy: &mut LlmPolicy, budget: Duration)
            -> Option<crate::pokemon::battle::BattleAction>
        {
            let deadline = Instant::now() + budget;
            while Instant::now() < deadline {
                if let Some(action) = self.tick_battle(policy) {
                    return Some(action);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            None
        }

        /// Poll like the agent does — fifty times a second — until a decision lands or time runs out.
        fn pump_overworld(&mut self, policy: &mut LlmPolicy) -> Option<OverworldAction> {
            self.pump_overworld_for(policy, Duration::from_secs(5))
        }

        fn pump_overworld_for(&mut self, policy: &mut LlmPolicy, budget: Duration) -> Option<OverworldAction> {
            let deadline = Instant::now() + budget;
            while Instant::now() < deadline {
                if let Some(action) = self.tick_overworld(policy) {
                    return Some(action);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            None
        }

        fn requests(&self) -> Vec<ChatRequest> {
            self.endpoint.seen.lock().unwrap().clone()
        }

        fn wait_for_requests(&self, count: usize, budget: Duration) {
            let deadline = Instant::now() + budget;
            while self.endpoint.seen.lock().unwrap().len() < count && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        fn drained_events(&self) -> Vec<UiEventBody> {
            self.events.try_iter().map(|event| event.body).collect()
        }

        /// Everything published up to and including the first event `wanted` accepts, or everything
        /// published within `budget` if it never arrives.
        ///
        /// The worker publishes on its own thread, so "the decision landed" does not mean everything
        /// that turn published has been seen — the status that follows it certainly has not.
        fn events_until(
            &self,
            budget: Duration,
            wanted: impl Fn(&UiEventBody) -> bool,
        ) -> Vec<UiEventBody> {
            let deadline = Instant::now() + budget;
            let mut seen: Vec<UiEventBody> = Vec::new();
            loop {
                seen.extend(self.events.try_iter().map(|event| event.body));
                if seen.iter().any(&wanted) || Instant::now() >= deadline {
                    return seen;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        fn statuses(events: &[UiEventBody]) -> Vec<RunStatus> {
            events
                .iter()
                .filter_map(|event| match event {
                    UiEventBody::Run { status } => Some(status.clone()),
                    _ => None,
                })
                .collect()
        }

        fn push(&self, replies: Vec<Reply>) {
            self.endpoint.replies.lock().unwrap().extend(replies);
        }

        /// The first menu id the model would be offered.
        fn first_action_id(&mut self) -> String {
            let state = self.state();
            tools::overworld_menu(&state).first().expect("Oak's lab has reachable actions").id.clone()
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            // The policy is dropped by the test before this; that bumps the generation and closes the
            // channels, which ends the worker's loop.
            let _ = self.published.publish_event(UiEventBody::Notice { level: "info", message: "done".into() });
            if let Some(handle) = self.worker.take() {
                let _ = handle.join();
            }
        }
    }

    /// `Arc<Scripted>` is not itself a `ChatEndpoint`; this forwards to it so the test can keep a
    /// handle on what the worker saw.
    struct Forwarding(Arc<Scripted>);

    impl ChatEndpoint for Forwarding {
        fn stream_completion(
            &self,
            request: &ChatRequest,
            on_delta: &mut dyn FnMut(&str),
            cancelled: &dyn Fn() -> bool,
        ) -> Result<Completion, LlmError> {
            self.0.stream_completion(request, on_delta, cancelled)
        }
    }

    fn last_user_message(request: &ChatRequest) -> &str {
        request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .and_then(Message::text)
            .expect("every request carries a user message")
    }

    /// Every `tool_call` in the history has a `tool` message answering it. This is the invariant the
    /// endpoint enforces with a 400, and the one §7.3's single-step rollback exists to preserve.
    fn history_is_well_formed(request: &ChatRequest) {
        let answered: std::collections::HashSet<&str> = request
            .messages
            .iter()
            .filter_map(|m| m.tool_call_id.as_deref())
            .collect();
        for message in &request.messages {
            for call in &message.tool_calls {
                assert!(
                    answered.contains(call.id.as_str()),
                    "`{}` ({}) was never answered — this request would 400",
                    call.id,
                    call.function.name,
                );
            }
        }
    }

    // ── The tests ────────────────────────────────────────────────────────────────────────────────

    /// The whole happy path, and the re-issue guard with it: the agent polls the policy fifty times a
    /// second, and exactly **one** turn must come of that.
    #[test]
    fn one_decision_point_is_one_turn_and_its_answer_is_executed() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        rig.endpoint
            .replies
            .lock()
            .unwrap()
            .push_back(calls(&[("choose_action", &format!(r#"{{"id":"{id}"}}"#))]));

        let action = rig.pump_overworld(&mut policy).expect("the decision lands");
        assert_eq!(tools::overworld_id(&rig.state(), &action), id);

        let requests = rig.requests();
        assert_eq!(requests.len(), 1, "the `pending` guard let {} turns out", requests.len());
        // …and the turn it did send was the overworld one, with the overworld tools.
        let offered: Vec<&str> = requests[0].tools.iter().map(|t| t.function.name).collect();
        assert!(offered.contains(&"choose_action") && !offered.contains(&"choose_battle_action"));
        assert!(last_user_message(&requests[0]).contains(&id), "the menu must carry the id it expects back");
    }

    /// **`POST /api/new-run`, from the policy's side.** A turn completes; the game is restarted
    /// underneath; the next turn must be a conversation about the *new* game.
    ///
    /// The assertion that matters is the message count. A turn's history grows — system prompt, the
    /// situation, the assistant's reply, the tool result — so a second turn on a live conversation
    /// sends strictly more messages than the first. After a restart it sends exactly as many as a
    /// first turn does, which is the only externally visible proof that the worker threw the old
    /// history away rather than compacting it, trimming it, or carrying it on.
    #[test]
    fn a_restart_starts_the_conversation_again() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        let reply = || calls(&[("choose_action", &format!(r#"{{"id":"{id}"}}"#))]);
        rig.endpoint.replies.lock().unwrap().push_back(reply());

        assert!(rig.pump_overworld(&mut policy).is_some(), "the first turn resolves");
        let first = rig.requests().len();
        assert_eq!(first, 1);
        let messages_in_first_turn = rig.requests()[0].messages.len();

        // The emulator thread calls this, through `PokemonAgent::restart`, on the reset tick.
        policy.restart(None);

        rig.endpoint.replies.lock().unwrap().push_back(reply());
        assert!(rig.pump_overworld(&mut policy).is_some(), "a turn still runs after the restart");
        rig.wait_for_requests(2, Duration::from_secs(5));
        let requests = rig.requests();
        assert_eq!(requests.len(), 2, "the second turn never reached the endpoint");
        assert_eq!(
            requests[1].messages.len(), messages_in_first_turn,
            "the second turn carried the old game's history: {:?}",
            requests[1].messages.iter().map(|m| m.role).collect::<Vec<_>>(),
        );
    }

    /// ⚠️ A restart must cancel the turn in flight, or an answer about the old game is applied to the
    /// new one — the same hazard `a_kind_change_cancels_the_turn_in_flight` covers, reached the other
    /// way. The generation is what does it, and this pins that `restart` bumps it.
    #[test]
    fn a_restart_cancels_the_turn_in_flight() {
        let release = Arc::new(AtomicBool::new(false));
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        rig.endpoint.replies.lock().unwrap().push_back(held(
            calls(&[("choose_action", &format!(r#"{{"id":"{id}"}}"#))]),
            &release,
        ));

        rig.tick_overworld(&mut policy);
        rig.wait_for_requests(1, Duration::from_secs(5));
        let generation = policy.handles.current_generation();

        policy.restart(None);
        assert!(policy.handles.current_generation() > generation,
                "the generation must move, or the in-flight turn survives the restart");

        // The held reply is released into a turn that no longer exists; it must not become an action.
        release.store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        rig.endpoint.replies.lock().unwrap().push_back(
            calls(&[("choose_action", &format!(r#"{{"id":"{id}"}}"#))]),
        );
        assert!(rig.pump_overworld(&mut policy).is_some(), "the run carries on after the restart");
    }

    /// ⚠️ `pick_field_move` shares the `Overworld` kind. It runs immediately before
    /// `pick_overworld_action` on **every** idle tick; if it were a kind of its own the two would
    /// cancel each other fifty times a second and no turn would ever complete.
    #[test]
    fn field_move_polls_do_not_cancel_the_overworld_turn() {
        let release = Arc::new(AtomicBool::new(false));
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        rig.endpoint.replies.lock().unwrap().push_back(held(
            calls(&[("choose_action", &format!(r#"{{"id":"{id}"}}"#))]),
            &release,
        ));

        // Fifty ticks of the real call order while the turn is in flight.
        for _ in 0..50 {
            let state = rig.state();
            assert_eq!(policy.pick_field_move(&state), None, "nothing has been decided to hand over");
            rig.tick_overworld(&mut policy);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(rig.requests().len(), 1, "the turn was re-issued");

        release.store(true, Ordering::SeqCst);
        assert!(rig.pump_overworld(&mut policy).is_some(), "the held turn still resolves");
        assert_eq!(rig.requests().len(), 1);
    }

    /// §7.2's whole point. An overworld turn is in flight; a trainer spots the player; the very next
    /// poll is for a battle. The stale turn must die and a battle turn must replace it.
    #[test]
    fn a_kind_change_cancels_the_turn_in_flight() {
        let release = Arc::new(AtomicBool::new(false));
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            replies.push_back(held(calls(&[("choose_action", &format!(r#"{{"id":"{id}"}}"#))]), &release));
            replies.push_back(calls(&[("choose_battle_action", r#"{"id":"run"}"#)]));
        }

        rig.tick_overworld(&mut policy);
        rig.wait_for_requests(1, Duration::from_secs(2));
        let generation = policy.handles.current_generation();

        // A trainer spots the player. The very next poll is for a battle.
        rig.enter_battle();
        assert!(rig.tick_battle(&mut policy).is_none(), "the battle turn has only just been asked");
        assert!(policy.handles.current_generation() > generation, "the generation must move to cancel");

        // The held reply is abandoned where it stands — the endpoint saw the cancellation rather
        // than a completed stream, so releasing it afterwards changes nothing.
        rig.wait_for_requests(2, Duration::from_secs(2));
        release.store(true, Ordering::SeqCst);

        let requests = rig.requests();
        assert_eq!(requests.len(), 2);
        let offered: Vec<&str> = requests[1].tools.iter().map(|t| t.function.name).collect();
        assert!(offered.contains(&"choose_battle_action") && !offered.contains(&"choose_action"),
                "the replacement turn is a battle turn");
        // …built from the state the battle is in, not from the overworld state it replaced. The menu
        // is the whole point of the turn, and a stale one would offer actions that cannot be taken.
        let asked = last_user_message(&requests[1]);
        assert!(asked.contains("### Battle menu") && asked.contains("`run`"), "{asked}");

        // …and it is the battle decision that lands, from a fresh `battle_options`.
        let action = rig.pump_battle(&mut policy, Duration::from_secs(2)).expect("the battle turn decides");
        assert_eq!(tools::battle_id(&action), "run");
    }

    /// §2.1 and §7.3 together: several reads in one assistant message are answered **all at once,
    /// from one observed `GameState`** — which is what lets a cancelled turn roll back exactly one
    /// step and still leave a history the endpoint will accept.
    #[test]
    fn a_parallel_read_batch_is_answered_from_one_observation() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            replies.push_back(calls(&[("read_map", "{}"), ("read_party", "{}"), ("read_trainer", "{}")]));
            replies.push_back(calls(&[("choose_action", &format!(r#"{{"id":"{id}"}}"#))]));
        }

        assert!(rig.pump_overworld(&mut policy).is_some(), "the second step decides");

        let requests = rig.requests();
        assert_eq!(requests.len(), 2);
        history_is_well_formed(&requests[1]);

        let results: Vec<&str> = requests[1]
            .messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .filter_map(Message::text)
            .collect();
        assert_eq!(results.len(), 3, "every call in the batch was answered, and in one go");

        let map = rig.state().map.map;
        let expected = format!("\"{map}\"");
        assert!(results[0].contains(&expected), "read_map: {}", &results[0][..results[0].len().min(200)]);
        assert!(results[1].contains("\"slot\":0"), "read_party: {}", results[1]);
        assert!(results[2].contains("\"badges\""), "read_trainer: {}", results[2]);
    }

    /// **W9 / §14** — a stuck turn is an ordinary turn in every respect except how its answer
    /// leaves: it may read first, and the press it ends with goes out through the escape hatch.
    ///
    /// ⚠️ **The read is the assertion that matters.** `service_tools` decides whether a batch belongs
    /// to the turn in flight by comparing the pending kind against the kind it thinks is being
    /// asked — and a `Stuck` turn looks, in the `GameState`, exactly like the overworld it wedged
    /// in. If `observed_kind` did not know that only the site can tell, every batch would come back
    /// `Cancelled`, every turn would restart, and the run would spend money in a loop for as long as
    /// the jam lasted. That failure mode is invisible in a test that answers without reading.
    #[test]
    fn a_stuck_turn_may_read_first_and_its_press_reaches_the_agent() {
        let (mut rig, mut policy) = Rig::new(vec![
            calls(&[("read_map", "{}")]),
            calls(&[("press_buttons", r#"{"buttons":["a"]}"#)]),
        ]);

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut pressed = Vec::new();
        while Instant::now() < deadline && pressed.is_empty() {
            rig.tick_stuck(&mut policy, "script");
            pressed = policy.take_manual_input();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(pressed, vec![JoypadButton::A], "the nudge never came back out of the policy");

        let requests = rig.requests();
        assert_eq!(requests.len(), 2, "a read, then the decision");
        history_is_well_formed(&requests[1]);
        let answered: Vec<&str> =
            requests[1].messages.iter().filter(|m| m.role == Role::Tool).filter_map(Message::text).collect();
        assert_eq!(answered.len(), 1, "the read batch of a stuck turn has to be answered, not cancelled");
        assert!(answered[0].contains(&format!("\"{}\"", rig.state().map.map)), "{}", answered[0]);

        // The turn was scoped as a stuck one: no menu tool on offer, and the situation says what the
        // agent believed it was doing rather than describing a decision it cannot carry out.
        let offered: Vec<&str> =
            requests[0].tools.iter().map(|tool| tool.function.name).collect();
        assert!(offered.contains(&"press_buttons") && offered.contains(&"wait"));
        assert!(!offered.contains(&"choose_action"), "a wedged agent cannot walk anywhere: {offered:?}");
        let situation = requests[0].messages.last().and_then(Message::text).unwrap_or_default();
        assert!(situation.contains("`script`"), "the situation must name the state it is stuck in");
        assert!(situation.contains("300 seconds"), "…and how long it has been stuck: {situation:.400}");
    }

    /// A jam that clears while the model is still thinking: the very next real decision point
    /// cancels the stuck turn, exactly as a battle cancels an overworld one (§7.2). Without this the
    /// press would arrive after the agent had moved on and be applied to a game somewhere else.
    #[test]
    fn a_stuck_turn_is_cancelled_the_moment_the_agent_asks_a_real_question() {
        let release = Arc::new(AtomicBool::new(false));
        let (mut rig, mut policy) = Rig::new(vec![
            held(calls(&[("press_buttons", r#"{"buttons":["a"]}"#)]), &release),
            calls(&[("wait", r#"{"ticks":1}"#)]),
        ]);

        rig.tick_stuck(&mut policy, "script");
        rig.wait_for_requests(1, Duration::from_secs(5));

        // The jam clears: the agent reaches an ordinary overworld poll while the stuck turn is still
        // streaming.
        assert!(rig.tick_overworld(&mut policy).is_none(), "the overworld turn has not answered yet");
        release.store(true, Ordering::SeqCst);

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            rig.tick_overworld(&mut policy);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(policy.take_manual_input().is_empty(),
                "a press decided for a jam that has cleared must not be delivered afterwards");
        assert!(rig.drained_events().iter().any(|event| matches!(event, UiEventBody::TurnCancelled { .. })),
                "a cancelled turn is an event, never a silence (§17 risk 2b)");
    }

    /// §7.3's rollback. A batch is cancelled mid-turn; the assistant message whose calls were never
    /// serviced is dropped, so the next request has no orphaned `tool_call`.
    #[test]
    fn a_cancelled_batch_leaves_the_history_well_formed() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            replies.push_back(calls(&[("read_map", "{}")]));
            replies.push_back(calls(&[("choose_battle_action", r#"{"id":"run"}"#)]));
        }

        // Start the overworld turn and let the model ask for a read…
        rig.tick_overworld(&mut policy);
        rig.wait_for_requests(1, Duration::from_secs(2));
        // …then change the question before the poll that would have answered it. `service_tools`
        // sees a batch belonging to an overworld turn while the site it is standing at is a battle,
        // and refuses to run it — which is the signal the worker rolls back on.
        rig.enter_battle();
        rig.tick_battle(&mut policy);
        rig.wait_for_requests(2, Duration::from_secs(2));

        let requests = rig.requests();
        assert_eq!(requests.len(), 2, "the battle turn was sent");
        history_is_well_formed(&requests[1]);
        assert!(
            !requests[1].messages.iter().any(|m| !m.tool_calls.is_empty()),
            "the unanswered assistant message should have been rolled back, not carried forward",
        );
    }

    /// §7.5's fallback: one nudge quoting the contract, then the contract is enforced for the model.
    #[test]
    fn a_reply_with_no_tool_call_is_nudged_once_then_forced_to_wait() {
        let (mut rig, mut policy) = Rig::new(vec![
            says("I think I will head north and see what happens."),
            says("Yes, north is definitely the way."),
        ]);

        // The forced `wait` resolves the turn; the pump then starts a second turn, which runs out of
        // script and hangs — so this pumps for a bounded time and asserts on what was published.
        rig.pump_overworld_for(&mut policy, Duration::from_secs(2));

        let requests = rig.requests();
        assert!(requests.len() >= 2, "the model got exactly one nudge before being overruled");
        assert!(last_user_message(&requests[1]).contains("no tool call"), "{}", last_user_message(&requests[1]));
        assert!(last_user_message(&requests[1]).contains("choose_action"), "the nudge quotes the contract");

        // …and it is *visible*. A model that cannot hold the contract has to show up as a rate, not
        // as a game that mysteriously stands still.
        let reasons: Vec<String> = rig
            .drained_events()
            .into_iter()
            .filter_map(|event| match event {
                UiEventBody::TurnCancelled { reason, .. } => Some(reason),
                _ => None,
            })
            .collect();
        assert!(
            reasons.iter().any(|reason| reason.contains("no tool call")),
            "the forced wait was not reported to the UI: {reasons:?}",
        );
    }

    /// §7.5's other fallback, and the one that is easy to get subtly wrong: a model that reads and
    /// reads and never commits is told to decide **while it still has a request left to decide in**.
    ///
    /// ⚠️ The assertion that matters is which request carries the sentence. Appended on the final
    /// iteration — where it used to be — "call a terminal tool now to end the turn" is a message the
    /// model first sees on the *next* turn, after this one has already been forced to a wait. The
    /// turn would still resolve, so nothing looked broken; it just spent a whole turn's tokens
    /// telling the model something it could not act on.
    #[test]
    fn a_turn_that_only_reads_is_told_to_decide_while_it_still_can() {
        let id = {
            let (mut rig, _) = Rig::new(vec![]);
            rig.first_action_id()
        };
        // Four steps: three of reading, and the fourth is the one the warning is for.
        let (mut rig, mut policy) = Rig::with_config(
            vec![
                calls(&[("read_map", "{}")]),
                calls(&[("read_party", "{}")]),
                calls(&[("read_bag", "{}")]),
                calls(&[("choose_action", &format!(r#"{{"id":"{id}"}}"#))]),
            ],
            |config| config.max_tool_steps = 4,
        );

        let action = rig.pump_overworld(&mut policy).expect("the last request is a real decision");
        assert_eq!(tools::overworld_id(&rig.state(), &action), id);

        let requests = rig.requests();
        assert_eq!(requests.len(), 4, "the whole budget was used");
        assert!(
            last_user_message(&requests[3]).contains("used every read"),
            "the final request must carry the instruction it is the answer to: {}",
            last_user_message(&requests[3]),
        );
        assert!(
            !last_user_message(&requests[2]).contains("used every read"),
            "…and not before that, or the budget is a step shorter than it says",
        );
    }

    /// A `wait` answers the question that was asked. A battle starting part-way through an overworld
    /// wait must not have to sit out the remainder of it — three seconds of game time is a long while
    /// to stand at a battle menu doing nothing.
    #[test]
    fn a_wait_from_one_kind_does_not_delay_the_other() {
        let (mut rig, mut policy) = Rig::new(vec![
            calls(&[("wait", r#"{"ticks":150}"#)]),
            calls(&[("choose_battle_action", r#"{"id":"run"}"#)]),
        ]);

        // Pump until the wait has been decided and is being counted down.
        rig.wait_for_requests(1, Duration::from_secs(2));
        for _ in 0..20 {
            assert!(rig.tick_overworld(&mut policy).is_none(), "a wait never yields an action");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(rig.requests().len(), 1, "the wait must not re-issue the turn either");

        rig.enter_battle();
        let action = rig.pump_battle(&mut policy, Duration::from_secs(2)).expect("the battle is asked at once");
        assert_eq!(tools::battle_id(&action), "run");
        assert_eq!(rig.requests().len(), 2);
    }

    /// §7.4's ⚠️: an id that no longer resolves is a message back to the model, never a panic and
    /// never a silent no-op.
    #[test]
    fn an_unresolvable_id_is_explained_on_the_next_turn() {
        let (mut rig, mut policy) = Rig::new(vec![
            calls(&[("choose_action", r#"{"id":"PalletTown:99,99:Warp"}"#)]),
            calls(&[("wait", r#"{"ticks":1}"#)]),
        ]);

        rig.pump_overworld_for(&mut policy, Duration::from_secs(2));

        let requests = rig.requests();
        assert!(requests.len() >= 2, "a fresh turn is asked after an unresolvable id");
        let reopened = last_user_message(&requests[1]);
        assert!(reopened.contains("no longer available"), "{reopened}");
        assert!(reopened.contains("PalletTown:99,99:Warp"), "the model is told which id failed");
    }

    // ── W5 ───────────────────────────────────────────────────────────────────────────────────────

    /// ⚠️ **A field move is decided by an overworld turn and collected by a different method.**
    /// `pick_overworld_action` cannot return one — its return type is a walk — so the decision is
    /// parked and `pick_field_move` takes it on the next tick. This pins both halves: that the
    /// overworld poll answers `None` rather than pretending, and that the very next field-move poll
    /// hands over the move the model actually asked for.
    #[test]
    fn a_field_move_decision_is_collected_by_the_next_field_move_poll() {
        let (mut rig, mut policy) = Rig::new(vec![calls(&[(
            "use_field_move",
            r#"{"move":"reorder_party","slot":0}"#,
        )])]);

        // The overworld poll never yields an action for this…
        assert!(rig.pump_overworld_for(&mut policy, Duration::from_secs(2)).is_none());
        // …and `pick_field_move`, which W4 always answered `None`, now has the answer.
        let state = rig.state();
        assert_eq!(policy.pick_field_move(&state), Some(FieldMove::ReorderParty { slot: 0 }));
        assert_eq!(policy.pick_field_move(&state), None, "it is taken, not repeated every tick");
    }

    /// A field move that cannot be carried out is a sentence back to the model, exactly as an
    /// unresolvable action id is — never a `FieldMove` handed to the agent that quietly does nothing.
    #[test]
    fn an_impossible_field_move_is_explained_rather_than_attempted() {
        let (mut rig, mut policy) = Rig::new(vec![
            // Nobody in Oak's lab is facing a tree, and the starter does not know Cut.
            calls(&[("use_field_move", r#"{"move":"cut"}"#)]),
            calls(&[("wait", r#"{"ticks":1}"#)]),
        ]);

        rig.pump_overworld_for(&mut policy, Duration::from_secs(2));
        assert_eq!(policy.pick_field_move(&rig.state()), None, "nothing was handed to the agent");

        let requests = rig.requests();
        assert!(requests.len() >= 2, "a fresh turn is asked after a field move that could not run");
        assert!(last_user_message(&requests[1]).contains("facing"), "{}", last_user_message(&requests[1]));
    }

    /// The escape hatch's policy half: a `press_buttons` decision leaves the presses where the agent
    /// collects them, and taking them empties the queue so they cannot be delivered twice.
    #[test]
    fn press_buttons_leaves_the_presses_for_the_agent_to_collect() {
        let (mut rig, mut policy) = Rig::new(vec![calls(&[(
            "press_buttons",
            r#"{"buttons":["b","start","a"]}"#,
        )])]);

        assert!(rig.pump_overworld_for(&mut policy, Duration::from_secs(2)).is_none());
        assert_eq!(
            policy.take_manual_input(),
            [JoypadButton::B, JoypadButton::Start, JoypadButton::A],
        );
        assert!(policy.take_manual_input().is_empty(), "a collected press is not queued again");
    }

    /// The three menu prompts, each asked as its own turn with its own scoped tools, and each
    /// answered into the shape its `pick_*` returns.
    ///
    /// ⚠️ The important part is that a batch **serviced during one of these** is answered rather than
    /// cancelled. `observed_kind` cannot see a naming screen in a `GameState`, so an earlier version
    /// read every one of these turns as `Overworld`, cancelled its first read, restarted the turn,
    /// and looped for as long as the prompt was open.
    #[test]
    fn the_menu_prompts_are_their_own_turns_and_can_use_read_tools() {
        let (mut rig, mut policy) = Rig::new(vec![
            calls(&[("read_party", "{}")]),
            calls(&[("set_nickname", r#"{"name":"Bubbles"}"#)]),
        ]);

        let answer = rig
            .pump_prompt(&mut policy, |policy, _| policy.pick_nickname(PokemonSpecies::Squirtle))
            .expect("the naming screen is answered");
        assert_eq!(answer, Some("Bubbles".to_string()));

        let requests = rig.requests();
        assert_eq!(requests.len(), 2, "one read step, then the decision — not a restart loop");
        history_is_well_formed(&requests[1]);
        let offered: Vec<&str> = requests[0].tools.iter().map(|t| t.function.name).collect();
        assert!(offered.contains(&"set_nickname") && !offered.contains(&"choose_action"));
        assert!(last_user_message(&requests[0]).contains("Squirtle"), "the species is in the situation");
        // The read really was serviced, from the live fixture.
        let results: Vec<&str> =
            requests[1].messages.iter().filter(|m| m.role == Role::Tool).filter_map(Message::text).collect();
        assert_eq!(results.len(), 1);
        assert!(results[0].contains("\"slot\":0"), "read_party: {}", results[0]);
    }

    /// The mart's stock is the menu, and it comes from the ROM through `ApiSnapshot` — nothing in
    /// `GameState` has it. A turn that offered `buy_item` without one would be asking the model to
    /// guess what the shop sells.
    #[test]
    fn a_mart_turn_answers_with_a_purchase() {
        let (mut rig, mut policy) = Rig::new(vec![calls(&[(
            "buy_item",
            r#"{"item":"Potion","quantity":3}"#,
        )])]);

        let answer = rig
            .pump_prompt(&mut policy, |policy, state| policy.pick_mart_purchase(state))
            .expect("the mart menu is answered");
        assert_eq!(answer, Some(BagItem::new(crate::pokemon::item::ItemId::Potion, 3)));

        let offered: Vec<&str> = rig.requests()[0].tools.iter().map(|t| t.function.name).collect();
        assert!(offered.contains(&"buy_item") && !offered.contains(&"choose_action"));
    }

    /// ⚠️ The forget prompt fires **mid-battle**, and answering it means cancelling the battle turn
    /// in flight — which is correct, because the prompt is the live question. This pins that the
    /// cancellation happens and that the answer is the slot the model named.
    #[test]
    fn a_forget_prompt_pre_empts_the_battle_turn_it_interrupts() {
        let release = Arc::new(AtomicBool::new(false));
        let (mut rig, mut policy) = Rig::new(vec![]);
        rig.enter_battle();
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            replies.push_back(held(calls(&[("choose_battle_action", r#"{"id":"run"}"#)]), &release));
            replies.push_back(calls(&[("forget_move", r#"{"slot":2}"#)]));
        }

        rig.tick_battle(&mut policy);
        rig.wait_for_requests(1, Duration::from_secs(2));
        let generation = policy.handles.current_generation();

        let moves: Vec<PokemonMove> = [
            PokemonMoveName::Tackle,
            PokemonMoveName::TailWhip,
            PokemonMoveName::Bubble,
            PokemonMoveName::WaterGun,
        ]
        .into_iter()
        .map(PokemonMove::with_max_pp)
        .collect();

        let answer = rig
            .pump_prompt(&mut policy, |policy, _| policy.pick_move_to_forget(&moves, PokemonMoveName::Bite))
            .expect("the forget prompt is answered");
        assert_eq!(answer, Some(2));
        release.store(true, Ordering::SeqCst);

        assert!(policy.handles.current_generation() > generation, "the battle turn must have been cancelled");
        let requests = rig.requests();
        assert_eq!(requests.len(), 2);
        let asked = last_user_message(&requests[1]);
        assert!(asked.contains("Bite"), "the incoming move is in the situation: {asked}");
        assert!(asked.contains("`2` — Bubble"), "the four known moves are the menu: {asked}");
    }

    /// A slot the Pokémon does not have would send the menu cursor somewhere it can never arrive, so
    /// it is declined — and the model is told why rather than left watching a prompt that never
    /// closes.
    #[test]
    fn a_forget_slot_the_pokemon_does_not_have_declines_instead_of_hanging() {
        let (mut rig, mut policy) = Rig::new(vec![calls(&[("forget_move", r#"{"slot":3}"#)])]);
        let moves: Vec<PokemonMove> =
            [PokemonMoveName::Tackle, PokemonMoveName::Growl].into_iter().map(PokemonMove::with_max_pp).collect();

        let answer = rig
            .pump_prompt(&mut policy, |policy, _| policy.pick_move_to_forget(&moves, PokemonMoveName::Bite))
            .expect("it is answered rather than left hanging");
        assert_eq!(answer, None, "declining keeps all the moves it has");
    }
    // ── W6 ───────────────────────────────────────────────────────────────────────────────────────

    /// §9's status. A viewer should be able to tell, at any instant, whether the run is waiting on
    /// the endpoint, reading the game, or playing — and the sequence must come back to `Playing`,
    /// because a status that gets stuck on `AwaitingLlm` is worse than none at all.
    #[test]
    fn the_run_status_follows_the_turn_and_settles_back_to_playing() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        rig.push(vec![
            calls(&[("read_map", "{}")]),
            saying_calls("North it is.", &[("choose_action", &format!(r#"{{"id":"{id}"}}"#))]),
        ]);

        rig.pump_overworld(&mut policy).expect("the decision lands");
        let events =
            rig.events_until(Duration::from_secs(2), |event| {
                matches!(event, UiEventBody::Run { status: RunStatus::Playing })
            });

        assert_eq!(Rig::statuses(&events), [
            RunStatus::AwaitingLlm { kind: "overworld" },
            RunStatus::RunningTool { name: "read_map".into() },
            RunStatus::AwaitingLlm { kind: "overworld" },
            RunStatus::Streaming,
            RunStatus::Playing,
        ]);
    }

    /// §9 end to end, through the real worker: a history over the threshold is summarised, and the
    /// **next** turn opens on the summary rather than on everything that came before it.
    ///
    /// The size comes from the model's own prose rather than from the fixture, because a compaction
    /// test that had to fill a real context window would have to send it through a scripted endpoint
    /// one turn at a time.
    #[test]
    fn a_full_context_is_summarised_and_the_next_turn_carries_the_summary() {
        let (mut rig, mut policy) = Rig::with_config(vec![], |config| config.context_limit = 6_000);
        let id = rig.first_action_id();
        let choose = format!(r#"{{"id":"{id}"}}"#);
        rig.push(vec![
            calls(&[("choose_action", &choose)]),
            calls(&[("choose_action", &choose)]),
            calls(&[("choose_action", &choose)]),
            // ~3 200 tokens of reasoning in one turn, which is what puts it over 70% of 6 000.
            saying_calls(&"I am thinking very hard about this. ".repeat(320), &[("choose_action", &choose)]),
            says("I am in Oak's lab with a Squirtle, about to leave for Route 1."),
            calls(&[("choose_action", &choose)]),
        ]);

        for turn in 1..=4 {
            rig.pump_overworld(&mut policy).unwrap_or_else(|| panic!("turn {turn} did not land"));
        }
        let events = rig
            .events_until(Duration::from_secs(5), |event| matches!(event, UiEventBody::Compacted { .. }));
        let compaction = events
            .iter()
            .find_map(|event| match event {
                UiEventBody::Compacted { before, after, summarised, .. } => Some((*before, *after, *summarised)),
                _ => None,
            })
            .expect("four turns of that should have filled a 6 000-token window");
        let (before, after, summarised) = compaction;
        assert!(summarised, "eviction cannot help a history with no pictures in it");
        assert!(after < before, "the compaction saved nothing: {before} → {after}");
        assert!(
            Rig::statuses(&events).contains(&RunStatus::Compacting),
            "a compaction is visible while it happens",
        );

        // The fifth turn is the point of the exercise: it opens on the system prompt and the summary.
        rig.pump_overworld(&mut policy).expect("the run continues after a compaction");
        let requests = rig.requests();
        let last = requests.last().expect("requests were sent");
        assert_eq!(last.messages[0].role, Role::System, "the system prompt is never compacted");
        assert!(
            last.messages[1].text().unwrap_or_default().starts_with("## The story so far"),
            "the summary is the second message: {:?}",
            last.messages[1].text(),
        );
        assert!(
            last.messages[1].text().unwrap_or_default().contains("exactly one terminal tool call"),
            "§9's ⚠️ — the contract has to survive the compaction",
        );
        // ⚠️ What is kept is the *tail*, so the turn that filled the window is still there — it is the
        // most recent one. Everything before it is not: four turns of history are now three messages
        // of it plus the summary.
        assert!(
            last.messages.len() <= 2 + crate::llm::compaction::KEEP_MESSAGES,
            "the middle of the conversation is still there: {} messages",
            last.messages.len(),
        );
        assert!(
            last.messages.len() < requests[3].messages.len(),
            "the turn after a compaction must be cheaper than the turn before it",
        );
        history_is_well_formed(last);
    }
}
