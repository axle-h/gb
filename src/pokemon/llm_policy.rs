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
//! kind the two would cancel each other fifty times a second and no turn would ever complete. W4
//! does not override it at all — the trait's `None` is exactly right, and W5's `use_field_move`
//! arrives as an *outcome* of an overworld turn, stashed and picked up on the next tick.

use std::sync::atomic::Ordering;

use crate::llm::prompt::{self, ApiSnapshot};
use crate::llm::tools::{self, DecisionKind, Terminal};
use crate::llm::worker::{ToolBatchResult, TurnHandles, TurnRequest};
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::agent::AgentEvent;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::policy::Policy;
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
    /// Prepended to the next turn: what went wrong with the last decision, in the model's own terms.
    note: Option<String>,
}

impl LlmPolicy {
    pub fn new(handles: TurnHandles) -> Self {
        Self {
            handles,
            pending: None,
            waiting: None,
            events: Vec::new(),
            snapshot: ApiSnapshot::default(),
            note: None,
        }
    }

    /// Which question *this* poll site is asking, inferred from the state.
    ///
    /// The plan has `service_tools` compare the pending kind against "the kind about to be asked",
    /// which the seam's signature does not carry. A battle in progress is the whole difference
    /// between the two kinds, so the state answers it — and it answers it the same way the `pick_*`
    /// that follows will. The consequence of being wrong either way is one wasted round trip, never
    /// a decision applied to the wrong state: the `pick_*` re-checks the kind before it accepts an
    /// outcome.
    fn observed_kind(state: &GameState) -> DecisionKind {
        match state.battle.is_some() {
            true => DecisionKind::Battle,
            false => DecisionKind::Overworld,
        }
    }

    /// The shared half of `pick_overworld_action` and `pick_battle_action`.
    ///
    /// Returns the decision to apply, or `None` for "not ready — ask again next tick", which is
    /// every one of: waiting out a `wait`, a turn still in flight, and a turn only just started.
    fn advance(&mut self, kind: DecisionKind, state: &GameState) -> Option<Terminal> {
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
            // A different question is being asked now. Cancelling costs the tokens already spent —
            // §17's risk 2b, which is why `TurnCancelled` is an event and not a silence.
            Some(_) => {
                self.start_turn(kind, state);
                None
            }
            None => {
                self.start_turn(kind, state);
                None
            }
        }
    }

    /// Bump the generation — which is what cancels anything in flight — and send a fresh turn.
    fn start_turn(&mut self, kind: DecisionKind, state: &GameState) {
        let id = self.handles.next_generation();
        let menu = match kind {
            DecisionKind::Overworld => tools::overworld_menu(state),
            DecisionKind::Battle => tools::battle_menu(state),
        };
        let mut situation = prompt::situation(kind, state, &self.snapshot, &self.events, &menu);
        if let Some(note) = self.note.take() {
            situation = format!("{note}\n\n{situation}");
        }
        let headline = format!(
            "{} — {} at ({}, {})",
            kind.label(),
            state.map.map,
            state.map.player_position.x,
            state.map.player_position.y,
        );
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
    /// ⚠️ Runs at every poll of every decision point — fifty times a second — so the common path
    /// here is an empty `try_recv` and nothing else.
    fn service_tools(&mut self, state: &GameState, api: &mut PokemonApi<'_>, graph: &WorldGraph) {
        // Only when a turn is about to be built: this is the one moment the policy is handed a
        // `PokemonApi`, and reading the screen costs a VRAM decode that is pure waste on the ticks
        // where nothing is going to ask for it.
        if self.pending.is_none() && self.waiting.is_none() {
            self.snapshot = ApiSnapshot::read(api);
        }

        let live = self.handles.current_generation();
        let asking = Self::observed_kind(state);
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
        match self.advance(DecisionKind::Overworld, state)? {
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
        match self.advance(DecisionKind::Battle, state)? {
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
    use crate::llm::protocol::{ChatRequest, Completion, FunctionCall, Role, ToolCall};
    use crate::llm::worker;
    use crate::pokemon::PokemonApiTrait;
    use crate::pokemon::actions::OverworldAction;
    use crate::web::published::{Published, UiEvent, UiEventBody};

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

            let config = LlmConfig {
                base_url: "http://scripted".into(),
                api_key: "none".into(),
                model: "scripted".into(),
                context_limit: 128_000,
                temperature: 1.0,
                max_tool_steps: 4,
            };
            let (worker, handles) =
                worker::channels(Box::new(Forwarding(Arc::clone(&endpoint))), config, Arc::clone(&published));
            let handle = worker.spawn().expect("the worker thread starts");

            let rig = Rig {
                gb,
                graph: WorldGraph::new(),
                endpoint,
                published,
                events,
                worker: Some(handle),
            };
            (rig, LlmPolicy::new(handles))
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
            .and_then(|m| m.content.as_deref())
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
            assert_eq!(policy.pick_field_move(&state), None, "W4 does not answer field moves");
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
            .filter_map(|m| m.content.as_deref())
            .collect();
        assert_eq!(results.len(), 3, "every call in the batch was answered, and in one go");

        let map = rig.state().map.map;
        let expected = format!("\"{map}\"");
        assert!(results[0].contains(&expected), "read_map: {}", &results[0][..results[0].len().min(200)]);
        assert!(results[1].contains("\"slot\":0"), "read_party: {}", results[1]);
        assert!(results[2].contains("\"badges\""), "read_trainer: {}", results[2]);
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
}
