//! The `Policy` the LLM drives.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;

use gb::joypad::JoypadButton;
use crate::llm::battle_report::{BattleReport, MAX_QUEUED as MAX_QUEUED_REPORTS};
use crate::llm::battle_script::{self, Outcome as ScriptOutcome, ScriptState};
use crate::llm::prompt::{self, ApiSnapshot, TurnContext};
use crate::llm::tools::{self, DecisionKind, Terminal};
use crate::llm::worker::{ToolBatchResult, TurnHandles, TurnRequest};
use crate::pokemon::actions::OverworldAction;
use crate::pokemon::agent::{AgentEvent, OverworldActionAbortedReason};
use crate::pokemon::bag::BagItem;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::policy::{FieldMove, Policy};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::world_graph::WorldGraph;
use crate::pokemon::{GameState, PokemonApi};

pub struct LlmPolicy {
    handles: TurnHandles,
    /// The kind the in-flight turn answers, and its generation.
    pending: Option<(DecisionKind, u64)>,
    waiting: Option<(DecisionKind, u16)>,
    /// Everything the agent has said since the last turn was built, folded into the next one.
    events: Vec<String>,
    /// The events the turn in flight carried: put back in front of the next turn's if that one
    /// is abandoned unanswered, or nothing would ever say them.
    in_flight_events: Vec<String>,
    /// The half of the situation that needs a `PokemonApi`.
    snapshot: ApiSnapshot,
    /// The last `GameState` seen at a poll where a turn could start.
    state: Option<Box<GameState>>,
    /// Which of the five poll sites was asked last.
    site: Option<DecisionKind>,
    /// The `choose_action` call being carried out, if one is — see [`ActionQueue`].
    queue: Option<ActionQueue>,
    /// What became of the overworld action handed over last, as the agent reported it.
    outcome: Option<ActionOutcome>,
    /// A decided [`FieldMove`], waiting for the `pick_field_move` that will collect it.
    field_move: Option<FieldMove>,
    /// Raw presses the agent collects at its next tick ([`Policy::take_manual_input`]).
    manual: Vec<JoypadButton>,
    /// Prepended to the next turn: what went wrong with the last decision.
    note: Option<String>,
    /// `GB_STUCK_TIMEOUT_SECS`, handed to the agent once ([`Policy::stuck_timeout`]).
    stuck_timeout: Option<std::time::Duration>,
    /// The battle being fought by the script, written up as it goes.
    battle_report: Option<BattleReport>,
    /// Finished reports, waiting for the next turn of any kind to carry them.
    reports: Vec<String>,
    /// Kinds `buy_item`'s `then` queued for this visit; see [`Policy::next_mart_purchase`].
    mart_queue: std::collections::VecDeque<BagItem>,
    /// The [`crate::llm::guide::chapter_index`] the last `read_guide` answered from, to spot a badge moving it.
    guide_chapter_read: Option<usize>,
    /// A report whose battle has ended, waiting for one more observation to close against.
    finishing: Option<BattleReport>,
    /// The model took this battle from its script with `choose_battle_action`'s `take_over`.
    taken_over: bool,
    /// The latest state with a battle in it, for closing a report's last turn.
    last_battle_state: Option<Box<GameState>>,
}

/// A note about the script, with whatever it printed before it stopped.
fn script_note(headline: &str, prints: &[String]) -> String {
    match prints.is_empty() {
        true => headline.to_string(),
        false => format!(
            "{headline}\n\nIt printed, before it stopped:\n{}",
            prints.iter().map(|line| format!("  {line}\n")).collect::<String>(),
        ),
    }
}

/// The note a turn of a battle the model has taken over carries.
fn taken_over_note(account: Option<String>) -> String {
    let mut note = String::from(
        "You took this battle over, so your battle script is deciding none of its turns. It is \
         still armed and goes back to deciding them as soon as this battle ends.",
    );
    if let Some(account) = account {
        note.push_str(&format!("\n\n{account}"));
    }
    note
}

/// What every LLM-played run calls its trainer, whatever the model.
pub(crate) const PLAYER_NAME: &str = "AI";

/// How many battles one action may be resumed through before the decision is handed back anyway.
pub(crate) const MAX_BATTLE_RESUMES: u8 = 5;

/// What became of the overworld action the policy last handed to the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionOutcome {
    /// `OverworldActionCompleted`, or `OverworldInteractionCompleted` for a person or a PC.
    Landed,
    /// `OverworldActionAborted`.
    Stopped(OverworldActionAbortedReason),
}

/// One `choose_action` call, while the agent is working through it.
struct ActionQueue {
    /// The id the agent is carrying out now.
    current: String,
    /// The ids the model chained behind it, in the order it wrote them.
    rest: VecDeque<String>,
    /// What has already landed.
    done: Vec<String>,
    /// From the tool call: a battle does not end `current`.
    resume_after_battle: bool,
    /// Battles `current` has already been resumed through.
    resumes: u8,
}

/// Why what was left of a chain was thrown away.
enum Dropped {
    /// The id matches nothing on the live map.
    Unresolved(String),
    /// The agent aborted the action and said why.
    Stopped(OverworldActionAbortedReason),
    /// The action ended without the agent naming an outcome — see [`LlmPolicy::outcome`].
    Unreported,
    /// [`MAX_BATTLE_RESUMES`], spent.
    Resumes,
}

/// The two different mistakes an id that will not resolve can be, in the words that say which.
fn unresolved_note(id: &str, state: &GameState) -> String {
    match id.split(':').next() {
        Some(named) if named != state.map.map.to_string() => format!(
            "`{id}` is an id for `{named}` and you are in `{}`. Ids are minted for the map you are \
             standing on, so one from an earlier turn never resolves. Nothing happened; pick from \
             the list in this turn.",
            state.map.map,
        ),
        _ => format!(
            "`{id}` is no longer available — the game moved on while you were deciding. Here is the \
             current situation; pick again."
        ),
    }
}

impl LlmPolicy {
    pub fn new(handles: TurnHandles, stuck_timeout: Option<std::time::Duration>) -> Self {
        Self {
            handles,
            stuck_timeout,
            pending: None,
            waiting: None,
            events: Vec::new(),
            in_flight_events: Vec::new(),
            snapshot: ApiSnapshot::default(),
            state: None,
            site: None,
            field_move: None,
            queue: None,
            outcome: None,
            manual: Vec::new(),
            note: None,
            battle_report: None,
            finishing: None,
            reports: Vec::new(),
            mart_queue: std::collections::VecDeque::new(),
            guide_chapter_read: None,
            taken_over: false,
            last_battle_state: None,
        }
    }

    /// Which question this poll site is asking.
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
    fn advance(&mut self, kind: DecisionKind, context: TurnContext<'_>) -> Option<Terminal> {
        // First: this tells the next tick's `service_tools` which question a batch belongs to.
        self.site = Some(kind);

        match self.waiting {
            Some((waiting_on, ticks)) if waiting_on == kind => {
                self.waiting = (ticks > 1).then_some((kind, ticks - 1));
                return None;
            }
            // The wait was answering the other question.
            Some(_) => self.waiting = None,
            None => {}
        }

        match self.pending {
            // The turn in flight is answering this very question.
            Some((pending, id)) if pending == kind => match self.handles.outcomes.try_recv() {
                Ok(outcome) if outcome.id == id => {
                    self.pending = None;
                    self.in_flight_events.clear();
                    Some(outcome.decision)
                }
                // An outcome from a turn already abandoned.
                Ok(_) => None,
                Err(_) => None,
            },
            // A different question is being asked now, or none was.
            _ => {
                self.start_turn(kind, context);
                None
            }
        }
    }

    /// Bump the generation — which is what cancels anything in flight — and send a fresh turn.
    fn start_turn(&mut self, kind: DecisionKind, context: TurnContext<'_>) {
        if self.pending.is_some() && !self.in_flight_events.is_empty() {
            let mut carried = std::mem::take(&mut self.in_flight_events);
            carried.append(&mut self.events);
            self.events = carried;
        }
        // Immutable reads of `self` stay inside this block, freeing the mutations below.
        let Some((mut situation, headline, menu)) = ({
            // No state has been observed yet, so there is nothing to describe.
            self.state.as_deref().map(|state| {
                let menu = match kind {
                    DecisionKind::Overworld => tools::overworld_menu(state, self.snapshot.arrival),
                    DecisionKind::Battle => tools::battle_menu(state),
                    DecisionKind::MartPurchase => tools::mart_menu(&self.snapshot, state),
                    DecisionKind::ForgetMove => match context {
                        TurnContext::ForgetMove { current, .. } => tools::forget_menu(current),
                        _ => Vec::new(),
                    },
                    // The naming screen offers no choices; the tool's own arguments are the menu.
                    DecisionKind::Nickname | DecisionKind::Stuck => Vec::new(),
                };
                let situation =
                    prompt::situation(kind, state, &self.snapshot, &self.events, &menu, context, &self.reports);
                let headline = format!(
                    "{} · {} at ({}, {})",
                    kind.label(),
                    state.map.map,
                    state.map.player_position.x,
                    state.map.player_position.y,
                );
                // The ids the situation was rendered from: `tools::classify` refuses anything
                // else, so a second list could reject an offered action.
                let ids: Vec<String> = menu.iter().map(|item| item.id.clone()).collect();
                (situation, headline, ids)
            })
        }) else {
            return;
        };

        let id = self.handles.next_generation();
        if let Some(note) = self.note.take() {
            situation = format!("{note}\n\n{situation}");
        }
        self.in_flight_events = std::mem::take(&mut self.events);
        // Spent by the turn that carried them.
        self.reports.clear();
        // Those events are gone, so the report has none to take back (`BattleReport::events_mark`).
        for report in self.battle_report.iter_mut().chain(self.finishing.iter_mut()) {
            report.events_mark = 0;
            report.events_end = report.events_end.map(|_| 0);
        }

        if self.handles.turns.send(TurnRequest { id, kind, situation, headline, menu }).is_ok() {
            self.pending = Some((kind, id));
        }
        // If the send failed the worker has gone.
    }

    /// The decision could not be carried out.
    fn reject(&mut self, note: String) {
        self.note = Some(note);
    }

    /// Let the model's own script decide this battle turn, if it has one and no turn is in flight.
    fn run_battle_script(&mut self, state: &GameState) -> Option<BattleAction> {
        if self.pending.is_some() || self.waiting.is_some() {
            return None;
        }
        let battle = state.battle.as_ref()?;
        if battle.battle_type == crate::pokemon::battle::BattleType::Safari {
            return None;
        }
        let source = self.handles.live_script.source()?;

        if self.finishing.is_some() {
            self.close_battle_report(None);
        }
        let report = match self.battle_report.as_mut() {
            Some(report) => report,
            None => self.battle_report.insert(BattleReport::open(state, self.events.len())?),
        };

        // After the report is opened and through `handed_back`, not as another early return.
        if self.taken_over {
            let account = report.handed_back(state);
            self.note = Some(taken_over_note(account));
            return None;
        }

        let turn = report.decisions() as u32 + 1;
        let evaluation = battle_script::run(&source, state, turn);

        match evaluation.outcome {
            ScriptOutcome::Action(action) => {
                // The script decided this turn, so nobody was asked and nothing was published.
                self.handles.live_script.decided_one();
                report.decided(state, &action, evaluation.prints);
                Some(action)
            }
            // The script wants this one answered properly.
            ScriptOutcome::Ask => {
                let account = report.handed_back(state);
                let mut note =
                    script_note("Your battle script handed this turn to you.", &evaluation.prints);
                // The `take_over` offer rides on an account: only a script that has been deciding
                // turns will replace what is chosen here.
                if let Some(account) = account {
                    note.push_str(&format!(
                        "\n\n{account}\nIt is still armed, so it decides the turns after this one \
                         too, including any it does not hand back. If the rest of this battle should \
                         be yours, pass `take_over` to `choose_battle_action`; it goes back to \
                         deciding at the next battle.",
                    ));
                }
                self.note = Some(note);
                None
            }
            ScriptOutcome::Failed(why) => {
                // What the script did before it broke; no `take_over`, as it has already stopped.
                let account = report.handed_back(state);
                self.handles.live_script.failed(&why);
                let mut note = script_note(
                    &format!(
                        "**Your battle script failed and is no longer deciding your battle turns.** \
                         {why}\n\nAnswer this turn yourself. When you are next in the overworld, \
                         `read_battle_script` to see it, fix it and `set_battle_script` again, or \
                         leave it off and keep answering battles as you always have.",
                    ),
                    &evaluation.prints,
                );
                if let Some(account) = account {
                    note.push_str(&format!("\n\n{account}"));
                }
                self.note = Some(note);
                None
            }
        }
    }

    /// Close the report whose battle has ended and queue it for the next turn.
    fn close_battle_report(&mut self, observed: Option<&GameState>) {
        let Some(report) = self.finishing.take() else { return };
        let end = report.events_end.unwrap_or(self.events.len()).min(self.events.len());
        self.events.drain(report.events_mark.min(end)..end);
        let fallback = self.last_battle_state.take();
        let rendered = report.finish(observed.or(fallback.as_deref()));
        // Oldest dropped first: the recent battles are the useful ones.
        if self.reports.len() >= MAX_QUEUED_REPORTS {
            self.reports.remove(0);
        }
        self.reports.push(rendered);
    }

    /// The turn the model does not pay for: the next chained action, or the same after a battle.
    fn advance_queue(&mut self, state: &GameState) -> Option<OverworldAction> {
        enum Step {
            /// `current` landed: move on to whatever was chained behind it.
            Next,
            /// A battle took `current` and the model asked for it back.
            Resume,
            Drop(Dropped),
        }

        let step = {
            let queue = self.queue.as_ref()?;
            match self.outcome {
                Some(ActionOutcome::Landed) => Step::Next,
                Some(ActionOutcome::Stopped(OverworldActionAbortedReason::Battle))
                    if queue.resume_after_battle =>
                {
                    match queue.resumes < MAX_BATTLE_RESUMES {
                        true => Step::Resume,
                        false => Step::Drop(Dropped::Resumes),
                    }
                }
                Some(ActionOutcome::Stopped(reason)) => Step::Drop(Dropped::Stopped(reason)),
                None => Step::Drop(Dropped::Unreported),
            }
        };
        // Spent either way, or the next action would be judged by the last one's outcome.
        self.outcome = None;

        match step {
            Step::Drop(dropped) => {
                self.drop_queue(dropped);
                return None;
            }
            Step::Resume => self.queue.as_mut()?.resumes += 1,
            Step::Next => {
                let queue = self.queue.as_mut()?;
                let finished = std::mem::take(&mut queue.current);
                queue.done.push(finished);
                // Per action, not per call: a chain of three each get the full budget.
                queue.resumes = 0;
                match queue.rest.pop_front() {
                    Some(next) => queue.current = next,
                    // The whole chain landed.
                    None => {
                        self.queue = None;
                        return None;
                    }
                }
            }
        }
        self.take_current(state)
    }

    /// Resolve the id at the head of the queue against a fresh action list and hand it over.
    fn take_current(&mut self, state: &GameState) -> Option<OverworldAction> {
        let id = self.queue.as_ref()?.current.clone();
        match tools::resolve_overworld(state, &id) {
            Some(action) => Some(action),
            None => {
                self.drop_queue(Dropped::Unresolved(unresolved_note(&id, state)));
                None
            }
        }
    }

    /// Throw away what is left of the chain and leave the model a note saying where it got to.
    fn drop_queue(&mut self, dropped: Dropped) {
        let Some(queue) = self.queue.take() else { return };
        let waiting = queue.rest.len();

        if waiting == 0 && matches!(dropped, Dropped::Stopped(_) | Dropped::Unreported) {
            return;
        }

        let why = match dropped {
            Dropped::Unresolved(sentence) => sentence,
            Dropped::Stopped(reason) => format!("`{}` was stopped: {reason}.", queue.current),
            Dropped::Unreported => format!(
                "The agent handed the decision back before `{}` finished.",
                queue.current,
            ),
            Dropped::Resumes => format!(
                "`{}` has been interrupted by a battle {MAX_BATTLE_RESUMES} times now, so it was \
                 not taken up again. Decide for yourself whether it is still the right thing to do.",
                queue.current,
            ),
        };

        let mut note = String::new();
        if !queue.done.is_empty() {
            note.push_str(&format!(
                "{} carried out. ",
                queue.done.iter().map(|id| format!("`{id}`")).collect::<Vec<_>>().join(", "),
            ));
        }
        note.push_str(&why);
        if waiting > 0 {
            note.push_str(match waiting {
                1 => " The one action you had chained behind it was not tried.".to_string(),
                more => format!(" The {more} actions you had chained behind it were not tried."),
            }
            .as_str());
            note.push_str(" The menu below is the current one; pick again from it.");
        }
        self.note = Some(note);
    }
}

impl Policy for LlmPolicy {
    fn name(&self) -> &'static str { crate::pokemon::policy::LLM_POLICY_NAME }

    /// Every LLM run is played by `AI`, whatever the model.
    fn player_name(&self) -> Option<String> {
        Some(PLAYER_NAME.to_string())
    }

    /// Runs fifty times a second, so the common path is a snapshot and an empty `try_recv`.
    fn service_tools(&mut self, state: &GameState, api: &mut PokemonApi<'_>, graph: &WorldGraph) {
        let live = self.handles.current_generation();
        let asking = self.observed_kind(state);

        // The one moment the policy has a `PokemonApi`, and the source of every turn's snapshot.
        self.snapshot = ApiSnapshot::read(api);
        self.snapshot.arrival = graph.arrival();
        // Moved, not cloned, so keeping the last battle state costs a pointer swap.
        if let Some(previous) = self.state.replace(Box::new(state.clone())) {
            if self.battle_report.is_some() && previous.battle.is_some() {
                self.last_battle_state = Some(previous);
            }
        }
        // This is the observation the report was waiting for.
        if self.finishing.is_some() && state.battle.is_none() {
            self.close_battle_report(Some(state));
        }

        while let Ok(batch) = self.handles.tool_calls.try_recv() {
            let current = batch.turn == live
                && self.pending.is_some_and(|(kind, id)| kind == asking && id == batch.turn);
            let result = match current {
                // All-or-nothing, from one observed state.
                true => {
                    // From the state the answer was rendered from: the chapter it was handed.
                    if batch.calls.iter().any(|call| call.function.name == tools::READ_GUIDE) {
                        self.guide_chapter_read = Some(crate::llm::guide::chapter_index(state.badges));
                    }
                    ToolBatchResult::Answered(
                        batch.calls.iter().map(|call| tools::service_read(call, state, api, graph)).collect(),
                    )
                }
                // The tool is never executed.
                false => ToolBatchResult::Cancelled,
            };
            let _ = self.handles.tool_results.send(result);
        }
    }

    fn pick_overworld_action(&mut self, state: &GameState, _graph: &WorldGraph) -> Option<OverworldAction> {
        // Before `advance`, so a chain still running never starts a turn.
        if let Some(action) = self.advance_queue(state) {
            return Some(action);
        }
        // Bound here rather than inline: `TurnContext` is `Copy` and borrows it.
        let standing = self.handles.live_script.standing();
        let context = TurnContext::Overworld {
            script: self.handles.live_script.state(),
            standing: &standing,
            guide: crate::llm::guide::status(state.badges, self.guide_chapter_read),
        };
        match self.advance(DecisionKind::Overworld, context)? {
            Terminal::ChooseAction { id, then, resume_after_battle } => {
                // Even a lone action gets a queue, so [`Self::take_current`] is the one place an
                // id is resolved, against a fresh list.
                self.queue = Some(ActionQueue {
                    current: id,
                    rest: then.into(),
                    done: Vec::new(),
                    resume_after_battle,
                    resumes: 0,
                });
                self.outcome = None;
                self.take_current(state)
            }
            // Stashed: this method returns a walk, and a field move is not one.
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
            other => {
                self.reject(format!("`{other:?}` cannot be used in the overworld."));
                None
            }
        }
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        // Before `advance`: a decision in hand is handed over rather than bought with a completion.
        if let Some(action) = self.run_battle_script(state) {
            return Some(action);
        }
        // Read after `run_battle_script`, never before.
        let context = match self.note.is_some() {
            true => TurnContext::None,
            false => TurnContext::Battle { script: self.handles.live_script.state() },
        };
        match self.advance(DecisionKind::Battle, context)? {
            Terminal::ChooseBattleAction { id, take_over } => match tools::resolve_battle(state, &id) {
                Some(action) => {
                    // Set only where the action resolved.
                    self.taken_over |= take_over;
                    Some(action)
                }
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

    /// Not a decision point, and must never become one.
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
        // Cleared here rather than when the shop closes.
        self.mart_queue.clear();
        match self.advance(DecisionKind::MartPurchase, TurnContext::None)? {
            // Not trimmed to the wallet here: `assert_pokemart_state` does that, because Gen 1
            // hands over nothing for an order it cannot afford.
            Terminal::BuyItem { item, then } => {
                self.mart_queue = then.into();
                Some(item)
            }
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

    /// The next kind queued by this visit's `buy_item`.
    fn next_mart_purchase(&mut self) -> Option<BagItem> {
        self.mart_queue.pop_front()
    }

    fn pick_move_to_forget(
        &mut self,
        party_slot: usize,
        current_moves: &[PokemonMove],
        new_move: PokemonMoveName,
    ) -> Option<Option<usize>> {
        let context =
            TurnContext::ForgetMove { slot: party_slot, current: current_moves, new: new_move };
        match self.advance(DecisionKind::ForgetMove, context)? {
            Terminal::ForgetMove { slot } => match slot {
                // A slot the mon lacks is never reached, and the cursor would loop until timeout.
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

    /// The sixth kind, asked by the watchdog rather than by a poll site.
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

    /// `POST /api/new-run` — the emulator has reloaded the game from the start under us.
    fn restart(&mut self, run_dir: Option<&std::path::Path>) {
        self.handles.next_generation();
        if let Ok(mut cell) = self.handles.reset.lock() {
            *cell = Some(crate::llm::worker::Reset {
                run_dir: run_dir.map(|path| path.to_path_buf()),
                kind: crate::llm::worker::ResetKind::NewGame,
            });
        }
        self.pending = None;
        self.waiting = None;
        self.events.clear();
        self.in_flight_events.clear();
        self.snapshot = ApiSnapshot::default();
        self.state = None;
        self.site = None;
        self.field_move = None;
        self.queue = None;
        self.outcome = None;
        self.manual.clear();
        self.note = None;
        // The game is a different game now, so a battle half-written up is about nothing.
        self.battle_report = None;
        self.finishing = None;
        self.taken_over = false;
        self.reports.clear();
        // A queued order belongs to a mart in the old game.
        self.mart_queue.clear();
        self.guide_chapter_read = None;
        self.last_battle_state = None;
        // Disarmed here as well as in `Worker::apply_reset`, which happens at another moment.
        self.handles.live_script.arm(None, ScriptState::Unedited, Default::default());
    }

    /// `POST /api/clear` — the model forgets the run; the run carries on.
    fn clear_conversation(&mut self, run_dir: Option<&std::path::Path>) -> Result<(), String> {
        let mut cell = self
            .handles
            .reset
            .lock()
            .map_err(|_| "the reset channel is poisoned, so the conversation cannot be cleared".to_string())?;
        if matches!(cell.as_ref(), Some(pending) if pending.kind == crate::llm::worker::ResetKind::NewGame) {
            return Ok(());
        }
        *cell = Some(crate::llm::worker::Reset {
            run_dir: run_dir.map(|path| path.to_path_buf()),
            kind: crate::llm::worker::ResetKind::Cleared,
        });
        drop(cell);

        self.handles.next_generation();
        // The in-flight turn, and any wait it asked for, answer a question the model forgot.
        self.pending = None;
        self.in_flight_events.clear();
        self.waiting = None;
        Ok(())
    }

    /// Collected by the agent at the top of its next tick, ahead of the state machine.
    fn take_manual_input(&mut self) -> Vec<JoypadButton> {
        std::mem::take(&mut self.manual)
    }

    /// The narrative between decisions, above all the abort reasons that stop a re-picked route.
    fn on_event(&mut self, event: &AgentEvent) {
        // The only place the policy learns how an action ended.
        match event {
            AgentEvent::OverworldActionCompleted { .. }
            | AgentEvent::OverworldInteractionCompleted { .. } => {
                self.outcome = Some(ActionOutcome::Landed);
            }
            AgentEvent::OverworldActionAborted { reason, .. } => {
                self.outcome = Some(ActionOutcome::Stopped(*reason));
            }
            // The cartridge's own words are the only account of a battle turn there is.
            AgentEvent::TextBox { message } => {
                if let Some(report) = self.battle_report.as_mut() {
                    report.said(message);
                }
            }
            // The one place `taken_over` is cleared, which scopes it to one fight.
            AgentEvent::BattleEnded => {
                self.finishing = self.battle_report.take();
                // This event is pushed below, and is the report's last.
                if let Some(report) = self.finishing.as_mut() {
                    report.events_end = Some(self.events.len() + 1);
                }
                self.taken_over = false;
            }
            _ => {}
        }

        // A conversation can run for hundreds of boxes while no decision is asked for.
        const MAX_BUFFERED: usize = 64;
        if self.events.len() >= MAX_BUFFERED {
            self.events.remove(0);
        }
        self.events.push(prompt::describe_event(event));
    }
}

impl Drop for LlmPolicy {
    /// The emulator thread is ending.
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

    use gb::game_boy::GameBoy;
    use crate::llm::LlmError;
    use crate::llm::client::ChatEndpoint;
    use crate::llm::config::LlmConfig;
    use crate::llm::protocol::{ChatRequest, Completion, Fragment, FunctionCall, Message, Role, ToolCall};
    use crate::llm::worker;
    use crate::pokemon::PokemonApiTrait;
    use crate::pokemon::actions::OverworldAction;
    use crate::published::{Published, RunStatus, UiEvent, UiEventBody};

    // ── A scripted endpoint ──────────────────────────────────────────────────────────────────────

    /// One reply, and whether it waits for permission first, so a turn can be held in flight.
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
            on_delta: &mut dyn FnMut(Fragment<'_>),
            cancelled: &dyn Fn() -> bool,
        ) -> Result<Completion, LlmError> {
            self.seen.lock().unwrap().push(request.clone());
            let Some(reply) = self.replies.lock().unwrap().pop_front() else {
                // Out of script.
                return Err(LlmError::Cancelled);
            };
            if let Some(release) = reply.release {
                // Checks `cancelled` as a real stream does, so cancelling takes production's path.
                while !release.load(Ordering::SeqCst) {
                    if cancelled() {
                        return Err(LlmError::Cancelled);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            // Reasoning before content, as a real endpoint sends them.
            if !reply.completion.reasoning.is_empty() {
                on_delta(Fragment::Reasoning(&reply.completion.reasoning));
            }
            if !reply.completion.content.is_empty() {
                on_delta(Fragment::Content(&reply.completion.content));
            }
            Ok(reply.completion)
        }
    }

    fn says(text: &str) -> Reply {
        Reply { completion: Completion { content: text.into(), ..Completion::default() }, release: None }
    }

    /// Adds a `summary` to every well-formed call without one, as `tools::classify` requires.
    fn calls(pairs: &[(&str, &str)]) -> Reply {
        let tool_calls = pairs
            .iter()
            .enumerate()
            .map(|(i, (name, arguments))| ToolCall {
                id: format!("call_{i}"),
                kind: "function".into(),
                function: FunctionCall { name: (*name).into(), arguments: with_summary(arguments) },
            })
            .collect();
        Reply { completion: Completion { tool_calls, ..Completion::default() }, release: None }
    }

    /// See [`calls`].
    fn with_summary(arguments: &str) -> String {
        let Ok(serde_json::Value::Object(mut object)) = serde_json::from_str(arguments) else {
            return arguments.to_string();
        };
        if !object.contains_key("summary") {
            object.insert("summary".into(), serde_json::json!("a test's decision"));
        }
        serde_json::Value::Object(object).to_string()
    }

    /// A reply that says something and calls a tool.
    fn saying_calls(text: &str, pairs: &[(&str, &str)]) -> Reply {
        let mut reply = calls(pairs);
        reply.completion.content = text.to_string();
        reply
    }

    /// A reply that thinks before it speaks.
    fn thinking(thought: &str, mut reply: Reply) -> Reply {
        reply.completion.reasoning = thought.to_string();
        reply
    }

    /// A reply cut off at `GB_MAX_TOKENS`: prose, no tool call, `finish_reason: "length"`.
    fn truncated(text: &str) -> Reply {
        let mut reply = says(text);
        reply.completion.finish_reason = Some("length".to_string());
        reply
    }

    fn held(mut reply: Reply, release: &Arc<AtomicBool>) -> Reply {
        reply.release = Some(Arc::clone(release));
        reply
    }

    // ── The rig ──────────────────────────────────────────────────────────────────────────────────

    /// A real `GameState` without a running emulator: the fixture is loaded and read once.
    struct Rig {
        gb: GameBoy,
        graph: WorldGraph,
        endpoint: Arc<Scripted>,
        published: Arc<Published>,
        events: std::sync::mpsc::Receiver<UiEvent>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    /// Oak's lab after the starter: a party of one, and several reachable actions.
    const FIXTURE: &[u8] = include_bytes!("data/oaks-lab-just-got-squirtle.bin");

    /// Mid-battle, the other decision kind, which the cancellation path needs.
    const IN_BATTLE: &[u8] = include_bytes!("data/battle-state.bin");

    impl Rig {
        fn new(script: Vec<Reply>) -> (Self, LlmPolicy) {
            Self::with_config(script, |_| {})
        }

        /// [`Self::new`] with the config tweaked, usually a small `context_limit` for compaction.
        fn with_config(script: Vec<Reply>, tweak: impl FnOnce(&mut LlmConfig)) -> (Self, LlmPolicy) {
            Self::with_config_in(script, None, tweak)
        }

        /// The same rig on a run directory, so a second rig on the same files is a restart.
        fn with_config_in(
            script: Vec<Reply>,
            run_dir: Option<&std::path::Path>,
            tweak: impl FnOnce(&mut LlmConfig),
        ) -> (Self, LlmPolicy) {
            let mut gb = GameBoy::dmg(crate::pokemon::roms::POKERED);
            gb.load_state(FIXTURE).expect("the committed fixture loads");

            let endpoint = Arc::new(Scripted {
                replies: Mutex::new(script.into_iter().collect()),
                seen: Mutex::new(Vec::new()),
            });
            let published = Published::new();

            // Drained onto an mpsc so a test reads every event without racing the ring buffer.
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
                compact_above: crate::llm::config::DEFAULT_COMPACT_ABOVE,
                temperature: 1.0,
                max_tool_steps: 4,
                request_timeout: std::time::Duration::from_secs(crate::llm::config::DEFAULT_REQUEST_TIMEOUT_SECS),
                max_tokens: Some(crate::llm::config::DEFAULT_MAX_TOKENS),
                reasoning_effort: None,
                stuck_timeout: Some(Duration::from_secs(300)),
            };
            tweak(&mut config);
            // Read off before the worker takes the config, exactly as `web/mod.rs` does.
            let config_stuck_timeout = config.stuck_timeout;
            let (worker, handles) = worker::channels(
                Box::new(Forwarding(Arc::clone(&endpoint))),
                config,
                Arc::clone(&published),
                crate::llm::todo::TodoList::open(run_dir),
                crate::llm::battle_script::BattleScript::open(run_dir),
                crate::llm::history::History::open(run_dir),
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

        /// A trainer just spotted the player.
        fn enter_battle(&mut self) {
            self.gb.load_state(IN_BATTLE).expect("the committed battle fixture loads");
            assert!(self.state().battle.is_some(), "battle-state.bin should be mid-battle");
        }

        fn state(&mut self) -> GameState {
            PokemonApi::new(&mut self.gb).game_state().expect("the fixture has a readable state")
        }

        /// One agent tick: the tool poll, then the decision poll, in `agent.rs`'s order.
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

        /// A menu prompt tick in `agent.rs`'s order: `service_tools`, then that site's `pick_*`.
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

        /// A jammed tick in `run_watchdog`'s order: `service_tools`, then `pick_unstick`.
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

        /// Poll fifty times a second until a decision lands or time runs out.
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

        /// Events up to the first `wanted` accepts, or all within `budget` if it never comes.
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
            self.action_ids(1).remove(0)
        }

        /// The first `count` menu ids, in the menu's order.
        fn action_ids(&mut self, count: usize) -> Vec<String> {
            let state = self.state();
            let menu = tools::overworld_menu(&state, None);
            assert!(menu.len() >= count, "Oak's lab offers {} actions, not {count}", menu.len());
            menu.into_iter().take(count).map(|item| item.id).collect()
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            // Dropping the policy bumped the generation and closed the channels, ending the worker.
            let _ = self.published.publish_event(UiEventBody::Notice { level: "info", message: "done".into() });
            if let Some(handle) = self.worker.take() {
                let _ = handle.join();
            }
        }
    }

    /// Forwards to an `Arc<Scripted>`, so the test keeps a handle on what the worker saw.
    struct Forwarding(Arc<Scripted>);

    impl ChatEndpoint for Forwarding {
        fn stream_completion(
            &self,
            request: &ChatRequest,
            on_delta: &mut dyn FnMut(Fragment<'_>),
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

    /// Every `tool_call` in the history has a `tool` message answering it.
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

    /// Fifty polls a second must make exactly one turn.
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

    /// `POST /api/new-run`, from the policy's side.
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

    /// `POST /api/clear`, end to end through the real worker.
    #[test]
    fn a_clear_throws_the_conversation_and_the_plan_away_and_plays_on() {
        let scratch = crate::run::Scratch::new("llm-clear");
        let (mut rig, mut policy) = Rig::with_config_in(vec![], Some(&scratch.0), |_| {});
        let id = rig.first_action_id();
        let choose = format!(r#"{{"id":"{id}"}}"#);
        let said = "I am heading north out of Pallet Town to look for Oak.";
        rig.push(vec![
            saying_calls(said, &[("todo_set", r#"{"text":"deliver the parcel to Oak"}"#), ("choose_action", &choose)]),
            calls(&[("choose_action", &choose)]),
            calls(&[("choose_action", &choose)]),
        ]);
        assert!(rig.pump_overworld(&mut policy).is_some(), "turn 1 decides");
        let first_turn_messages = rig.requests()[0].messages.len();
        assert!(rig.pump_overworld(&mut policy).is_some(), "turn 2 decides");
        let plan = scratch.0.join(crate::run::files::TODO);
        assert!(plan.is_file(), "the precondition: the model wrote a plan down");
        assert!(
            rig.requests()[1].messages.len() > first_turn_messages,
            "the precondition: turn 2 really is carrying turn 1's conversation",
        );

        // Exactly what the emulator thread does on the clear tick, through `PokemonAgent`.
        policy.clear_conversation(Some(scratch.0.as_path())).expect("a model is playing");

        assert!(rig.pump_overworld(&mut policy).is_some(), "the run plays on after the clear");
        rig.wait_for_requests(3, Duration::from_secs(5));
        let requests = rig.requests();
        let last = requests.last().expect("a third request");
        assert!(
            !last.messages.iter().any(|m| m.text().is_some_and(|t| t.contains(said))),
            "the cleared turn is still carrying what the model said before it",
        );
        assert_eq!(
            last.messages.len(), first_turn_messages + 1,
            "a cleared turn is a first turn plus the note: {:?}",
            last.messages.iter().map(|m| m.role).collect::<Vec<_>>(),
        );
        assert!(
            last.messages.iter().any(|m| m.text() == Some(crate::llm::prompt::CLEARED_NOTE)),
            "and the model is told the erasure was deliberate",
        );
        history_is_well_formed(last);
        assert!(!plan.is_file(), "todo.json outlived the clear, which is the whole thing it must not do");

        drop(policy);
        drop(rig);
    }

    /// A clear must not downgrade a `POST /api/new-run` that has not been picked up yet.
    #[test]
    fn a_clear_leaves_a_restart_that_has_not_landed_yet_alone() {
        // `_rig` held to the end: its join must come after the policy's `Sender` has gone.
        let (_rig, mut policy) = Rig::new(vec![]);
        policy.restart(None);
        policy.clear_conversation(None).expect("a model is playing");

        let pending = policy.handles.reset.lock().expect("the cell").clone();
        assert_eq!(
            pending.expect("a reset is still pending").kind,
            crate::llm::worker::ResetKind::NewGame,
            "the clear overwrote the restart",
        );
    }

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

        // Released into a turn that has been abandoned, the reply must not become an action.
        release.store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        rig.endpoint.replies.lock().unwrap().push_back(
            calls(&[("choose_action", &format!(r#"{{"id":"{id}"}}"#))]),
        );
        assert!(rig.pump_overworld(&mut policy).is_some(), "the run carries on after the restart");
    }

    /// `pick_field_move` shares the `Overworld` kind.
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

        // A trainer spots the player.
        rig.enter_battle();
        assert!(rig.tick_battle(&mut policy).is_none(), "the battle turn has only just been asked");
        assert!(policy.handles.current_generation() > generation, "the generation must move to cancel");

        // The endpoint saw the cancellation, so releasing the held reply changes nothing.
        rig.wait_for_requests(2, Duration::from_secs(2));
        release.store(true, Ordering::SeqCst);

        let requests = rig.requests();
        assert_eq!(requests.len(), 2);
        let offered: Vec<&str> = requests[1].tools.iter().map(|t| t.function.name).collect();
        assert!(offered.contains(&"choose_battle_action") && !offered.contains(&"choose_action"),
                "the replacement turn is a battle turn");
        // …built from the state the battle is in, not from the overworld state it replaced.
        let asked = last_user_message(&requests[1]);
        assert!(asked.contains("### Battle menu") && asked.contains("`run`"), "{asked}");

        // …and it is the battle decision that lands, from a fresh `battle_options`.
        let action = rig.pump_battle(&mut policy, Duration::from_secs(2)).expect("the battle turn decides");
        assert_eq!(tools::battle_id(&action), "run");
    }

    /// What the cancelled turn was told is told again by the turn that replaced it.
    #[test]
    fn a_cancelled_turn_hands_its_events_to_the_next() {
        let release = Arc::new(AtomicBool::new(false));
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            replies.push_back(held(calls(&[("choose_action", &format!(r#"{{"id":"{id}"}}"#))]), &release));
            replies.push_back(calls(&[("choose_battle_action", r#"{"id":"run"}"#)]));
        }
        policy.on_event(&AgentEvent::TextBox { message: "the walk was given up".into() });
        rig.tick_overworld(&mut policy);
        rig.wait_for_requests(1, Duration::from_secs(2));

        // A trainer spots the player before the model has answered.
        rig.enter_battle();
        rig.tick_battle(&mut policy);
        rig.wait_for_requests(2, Duration::from_secs(2));
        release.store(true, Ordering::SeqCst);

        let requests = rig.requests();
        let asked = last_user_message(&requests[1]);
        assert!(asked.contains("the walk was given up"), "the cancelled turn's news went with it:\n{asked}");
    }

    /// The plan is in the history exactly once, and the prefix in front of it never moves.
    #[test]
    fn the_plan_is_appended_and_never_disturbs_the_cacheable_prefix() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        let choose = format!(r#"{{"id":"{id}"}}"#);
        rig.push(vec![
            // Turn 1 decides without touching the plan at all.
            calls(&[("choose_action", &choose)]),
            // Turn 2 edits the plan and decides in one message.
            calls(&[
                ("todo_set", r#"{"text":"come back to Route 12 with the Poke Flute"}"#),
                ("choose_action", &choose),
            ]),
            // Turns 3 and 4 change nothing, so neither may move the plan.
            calls(&[("choose_action", &choose)]),
            calls(&[("choose_action", &choose)]),
        ]);
        for turn in 1..=4 {
            assert!(rig.pump_overworld(&mut policy).is_some(), "turn {turn} decides");
        }

        let requests = rig.requests();
        assert_eq!(requests.len(), 4);
        let plans = |request: &ChatRequest| -> Vec<String> {
            request.messages.iter().filter(|m| crate::llm::prompt::is_plan(m))
                .filter_map(Message::text).map(str::to_string).collect()
        };

        // The prefix.
        for (n, request) in requests.iter().enumerate() {
            assert_eq!(request.messages[0].role, Role::System);
            assert_eq!(request.messages[0], requests[0].messages[0],
                       "request {n}'s system message differs — the whole prefix cache is gone");
        }

        // The newest copy is the plan; the older ones are left where they are.
        assert!(!plans(&requests[0]).last().expect("a plan").contains("Poke Flute"),
                "nothing was planned yet");
        assert!(plans(&requests[2]).last().expect("a plan").contains("Poke Flute"),
                "the item added mid-turn is in the next turn: {:?}", plans(&requests[2]));
        assert_eq!(plans(&requests[2]).len(), 2, "the empty opening plan is still there, untouched");

        // And therefore the history is append-only, with no exceptions.
        for n in 1..requests.len() {
            let sent = &requests[n - 1].messages;
            assert_eq!(&requests[n].messages[..sent.len()], &sent[..],
                       "request {n} rewrote history request {} had already sent — the cache is gone",
                       n - 1);
        }

        // The page is told too.
        let published: Vec<Vec<String>> = rig
            .drained_events()
            .into_iter()
            .filter_map(|event| match event {
                UiEventBody::Plan { items } => Some(items.into_iter().map(|item| item.text).collect()),
                _ => None,
            })
            .collect();
        // Two publishes across four turns: the opening one and the edit.
        assert_eq!(published, [vec![], vec!["come back to Route 12 with the Poke Flute".to_string()]],
                   "published on change, not on a timer");
    }

    /// An unchanged plan still comes back to the tail eventually.
    #[test]
    fn a_plan_nobody_edits_is_brought_back_to_the_tail_of_the_history() {
        const REFRESH: usize = crate::llm::worker::PLAN_REFRESH_TURNS as usize;
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        let choose = format!(r#"{{"id":"{id}"}}"#);
        // One edit on turn 1, then nothing at all for well past the refresh window.
        let mut replies = vec![calls(&[
            ("todo_set", r#"{"text":"deliver the parcel to Oak"}"#),
            ("choose_action", &choose),
        ])];
        replies.extend((0..REFRESH + 3).map(|_| calls(&[("choose_action", &choose)])));
        let turns = replies.len();
        rig.push(replies);
        for turn in 1..=turns {
            assert!(rig.pump_overworld(&mut policy).is_some(), "turn {turn} decides");
        }

        let requests = rig.requests();
        let plan_at = |request: &ChatRequest| -> usize {
            request.messages.iter().rposition(|m| crate::llm::prompt::is_plan(m)).expect("a plan is carried")
        };
        // Request 1 first carries the edit, so the window runs from there.
        let planted = plan_at(&requests[1]);
        let due = REFRESH + 2;
        assert_eq!(plan_at(&requests[due - 1]), planted,
                   "the plan moved before it was due — every turn in between pays a re-prefill");
        for request in 2..due {
            let sent = &requests[request - 1].messages;
            assert_eq!(&requests[request].messages[..sent.len()], &sent[..],
                       "request {request} rewrote history the one before had already sent");
        }
        let refreshed = plan_at(&requests[due]);
        assert!(refreshed > planted, "the plan is still buried at {refreshed} after {REFRESH} quiet turns");
        assert_eq!(requests[due].messages[planted], requests[due - 1].messages[planted],
                   "the older copy was disturbed — everything after it is a re-prefill");
        assert_eq!(requests[due].messages[planted], requests[due].messages[refreshed],
                   "and the refresh says the same thing, since nothing edited it");
        assert_eq!(refreshed, requests[due].messages.len() - 2,
                   "a refreshed plan belongs directly in front of the turn that reads it");
        assert_eq!(plan_at(&requests[due + 1]), refreshed,
                   "and the window starts again rather than moving it every turn from here on");

        // Every turn that does not carry the plan says so.
        for (n, request) in requests.iter().enumerate() {
            let asked = last_user_message(request);
            let carried = plan_at(request) == request.messages.len() - 2;
            assert_eq!(!carried, asked.contains(crate::llm::prompt::PLAN_UNCHANGED),
                       "request {n} carried={carried} but the note says otherwise: {asked}");
        }
    }

    /// A refused plan call repeated inside the turn is answered, not run again.
    #[test]
    fn a_refused_plan_call_repeated_in_one_turn_is_not_serviced_twice() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        let choose = format!(r#"{{"id":"{id}"}}"#);
        rig.push(vec![
            // One real item, so the refusal has ids to name.
            calls(&[("todo_set", r#"{"text":"deliver the parcel to Oak"}"#), ("choose_action", &choose)]),
            calls(&[
                ("todo_set", r#"{"id":5,"text":""}"#),
                ("todo_set", r#"{"id":5,"text":""}"#),
                ("todo_complete", r#"{"id":5}"#),
                ("todo_complete", r#"{"id":5}"#),
            ]),
            calls(&[("choose_action", &choose)]),
        ]);
        assert!(rig.pump_overworld(&mut policy).is_some(), "turn 1 decides");
        assert!(rig.pump_overworld(&mut policy).is_some(), "turn 2 decides after its read step");

        // The tool results of the read step are in the request that followed it.
        let requests = rig.requests();
        let answers: Vec<String> = requests.last().expect("a third request")
            .messages.iter()
            .filter(|m| m.role == Role::Tool)
            .filter_map(Message::text)
            .filter(|text| text.contains("TODO 5") || text.contains("that exact call"))
            .map(str::to_string)
            .collect();
        assert_eq!(answers.len(), 4, "every call still gets a result: {answers:?}");
        for first in [&answers[0], &answers[2]] {
            assert!(first.contains("There is no TODO 5"), "{first}");
            assert!(first.contains("holds 1"), "the first refusal names the ids: {first}");
        }
        for repeat in [&answers[1], &answers[3]] {
            assert!(repeat.contains("already made that exact call this turn"), "{repeat}");
        }

        // And none of it may have reached the list.
        let plan = requests.last().expect("a third request").messages.iter()
            .rposition(|m| crate::llm::prompt::is_plan(m))
            .map(|at| requests.last().unwrap().messages[at].text().unwrap_or_default().to_string())
            .expect("a plan is carried");
        assert!(plan.contains("deliver the parcel to Oak"), "{plan}");
        assert_eq!(plan.matches("- [").count(), 1, "the plan grew under a turn that only failed: {plan}");
    }

    /// The periodic refresh is an overworld thing, and an edit is not.
    #[test]
    fn a_battle_turn_never_pays_to_reposition_a_plan_it_cannot_act_on() {
        const REFRESH: usize = crate::llm::worker::PLAN_REFRESH_TURNS as usize;
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        let choose = format!(r#"{{"id":"{id}"}}"#);
        let mut replies = vec![calls(&[
            ("todo_set", r#"{"text":"deliver the parcel to Oak"}"#),
            ("choose_action", &choose),
        ])];
        // Well past the window, all of it in battle.
        replies.extend((0..REFRESH + 4).map(|_| calls(&[("choose_battle_action", r#"{"id":"run"}"#)])));
        rig.push(replies);
        assert!(rig.pump_overworld(&mut policy).is_some(), "the plan is planted by an overworld turn");
        rig.enter_battle();
        for turn in 0..REFRESH + 4 {
            assert!(rig.pump_battle(&mut policy, Duration::from_secs(2)).is_some(), "battle turn {turn} decides");
        }

        let requests = rig.requests();
        let plan_at = |request: &ChatRequest| -> usize {
            request.messages.iter().rposition(|m| crate::llm::prompt::is_plan(m)).expect("a plan is carried")
        };
        // Request 1, a battle turn, moves the plan: the overworld turn before it edited it.
        let planted = plan_at(&requests[1]);
        assert_eq!(planted, requests[1].messages.len() - 2, "the edit is carried on the next turn");
        assert!(!last_user_message(&requests[1]).contains(crate::llm::prompt::PLAN_UNCHANGED));

        // Later battle turns leave the plan alone however long the window has been up.
        for (n, request) in requests.iter().enumerate().skip(2) {
            assert_eq!(plan_at(request), planted,
                       "request {n} is a battle turn and repositioned the plan anyway");
            assert!(last_user_message(request).contains(crate::llm::prompt::PLAN_UNCHANGED),
                    "but it must still be told the plan is back there: request {n}");
        }
        assert!(requests.len() > REFRESH + 2, "the window has to have fallen due for this to mean anything");
    }

    /// A compaction can take the plan, and the next turn puts it back.
    #[test]
    fn a_compaction_that_drops_the_plan_does_not_break_the_chain() {
        use crate::llm::compaction;
        let mut todo = crate::llm::todo::TodoList::open(None);
        todo.apply(crate::llm::todo::TodoCall::Set { id: None, text: Some("deliver the parcel".into()) });
        let plan = crate::llm::prompt::plan_message(&todo);

        let mut messages = vec![Message::system(crate::llm::prompt::SYSTEM_PROMPT)];
        for turn in 0..6 {
            if turn == 1 { messages.push(plan.clone()); }
            messages.push(Message::user(format!("## Decision: turn {turn}")));
            messages.push(Message::assistant(format!("did turn {turn}"), vec![]));
        }
        assert!(messages.iter().any(crate::llm::prompt::is_plan), "the plan starts in the history");

        compaction::apply_summary(&mut messages, "I am in Pallet Town.", compaction::KEEP_MESSAGES);
        assert_eq!(messages[0].role, Role::System, "and the system prompt is never compacted");
        assert!(!messages.iter().any(crate::llm::prompt::is_plan),
                "this history is long enough that the plan is inside the dropped middle — otherwise \
                 the test proves nothing");

        // The repair is `sync_plan`'s "there is no copy" arm, which is what the next turn runs.
        assert!(messages.iter().position(|m| crate::llm::prompt::is_plan(m)).is_none());
        messages.push(crate::llm::prompt::plan_message(&todo));
        let restored = messages.last().expect("a plan was appended");
        assert!(crate::llm::prompt::is_plan(restored));
        assert!(restored.text().expect("prose").contains("deliver the parcel"),
                "and it is re-rendered from the list on disk, so it cannot come back stale");
    }

    #[test]
    fn a_parallel_read_batch_is_answered_from_one_observation() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            replies.push_back(calls(&[("read_map", "{}"), ("read_party", "{}"), ("read_bag", "{}")]));
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
        assert!(results[2].contains("\"slots_total\":20"), "read_bag: {}", results[2]);
    }

    /// A stuck turn may read first, and its press goes out through the escape hatch.
    #[test]
    fn a_stuck_turn_may_read_first_and_its_press_reaches_the_agent() {
        let (mut rig, mut policy) = Rig::new(vec![
            calls(&[("read_map", "{}")]),
            calls(&[("press_buttons", r#"{"buttons":["a"],"why":"a text box that will not close"}"#)]),
        ]);

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut pressed = Vec::new();
        while Instant::now() < deadline && pressed.is_empty() {
            rig.tick_stuck(&mut policy, "script");
            pressed = policy.take_manual_input();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(pressed, vec![JoypadButton::A], "the nudge never came back out of the policy");
        assert!(policy.take_manual_input().is_empty(), "a collected press is not queued again");

        let requests = rig.requests();
        assert_eq!(requests.len(), 2, "a read, then the decision");
        history_is_well_formed(&requests[1]);
        let answered: Vec<&str> =
            requests[1].messages.iter().filter(|m| m.role == Role::Tool).filter_map(Message::text).collect();
        assert_eq!(answered.len(), 1, "the read batch of a stuck turn has to be answered, not cancelled");
        assert!(answered[0].contains(&format!("\"{}\"", rig.state().map.map)), "{}", answered[0]);

        let offered: Vec<&str> =
            requests[0].tools.iter().map(|tool| tool.function.name).collect();
        assert!(offered.contains(&"press_buttons") && offered.contains(&"wait"));
        assert!(!offered.contains(&"choose_action"), "a wedged agent cannot walk anywhere: {offered:?}");
        let situation = requests[0].messages.last().and_then(Message::text).unwrap_or_default();
        assert!(situation.contains("`script`"), "the situation must name the state it is stuck in");
        assert!(situation.contains("300 seconds"), "…and how long it has been stuck: {situation:.400}");
    }

    #[test]
    fn a_stuck_turn_is_cancelled_the_moment_the_agent_asks_a_real_question() {
        let release = Arc::new(AtomicBool::new(false));
        let (mut rig, mut policy) = Rig::new(vec![
            held(calls(&[("press_buttons", r#"{"buttons":["a"]}"#)]), &release),
            calls(&[("wait", r#"{"ticks":1}"#)]),
        ]);

        rig.tick_stuck(&mut policy, "script");
        rig.wait_for_requests(1, Duration::from_secs(5));

        // The jam clears while the stuck turn is still streaming.
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
                "a cancelled turn is an event, never a silence");
    }

    /// Reasoning reaches the page as its own event, and never reaches the endpoint again.
    #[test]
    fn thinking_is_published_but_never_sent_back() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        rig.push(vec![
            thinking("Oak's lab is north.", calls(&[("read_map", "{}")])),
            thinking("Yes, north.", saying_calls("Heading north.", &[("choose_action", &format!(r#"{{"id":"{id}"}}"#))])),
        ]);

        rig.pump_overworld(&mut policy).expect("the turn lands");
        let events = rig.events_until(Duration::from_secs(5), |event| matches!(event, UiEventBody::Decision { .. }));

        let thoughts: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                UiEventBody::AssistantReasoning { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(thoughts, ["Oak's lab is north.", "Yes, north."], "one block per completion, not per turn");

        let said: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                UiEventBody::AssistantDelta { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(said, ["Heading north."], "the reply is still its own channel");

        // Nothing the model thought may appear in any message of any request that follows.
        for request in rig.requests() {
            for message in &request.messages {
                let text = message.text().unwrap_or_default();
                assert!(!text.contains("north."), "the thinking was sent back to the endpoint: {text}");
            }
        }
    }

    /// The summary is the one sentence about a turn that outlives it.
    #[test]
    fn the_reason_for_a_decision_is_carried_into_the_next_turn() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        let why = "Oak wants to see me before I leave town.";
        rig.push(vec![
            calls(&[("choose_action", &format!(r#"{{"id":"{id}","summary":"{why}"}}"#))]),
            calls(&[("choose_action", &format!(r#"{{"id":"{id}","summary":"and now inside"}}"#))]),
        ]);

        rig.pump_overworld(&mut policy).expect("the first turn lands");
        let events = rig.events_until(Duration::from_secs(5), |event| matches!(event, UiEventBody::Decision { .. }));
        let narration = events.iter().find_map(|event| match event {
            UiEventBody::Decision { narration, .. } => narration.clone(),
            _ => None,
        });
        assert_eq!(narration.as_deref(), Some(why), "the page is told the model's own reason");

        // The next request carries it, in the assistant message still in the history.
        rig.pump_overworld(&mut policy).expect("the second turn lands");
        let requests = rig.requests();
        let latest = requests.last().expect("a second request");
        assert!(
            latest.messages.iter().any(|message| {
                message.tool_calls.iter().any(|call| call.function.arguments.contains(why))
            }),
            "the reason for the last decision is not in the history the next turn was built on",
        );
    }

    /// A reply cut off by `GB_MAX_TOKENS` gets a different nudge from one that said nothing.
    #[test]
    fn a_reply_cut_off_by_the_token_cap_is_told_that_rather_than_that_it_said_nothing() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        rig.push(vec![
            truncated(&"I should think about this very carefully. ".repeat(20)),
            calls(&[("choose_action", &format!(r#"{{"id":"{id}"}}"#))]),
        ]);

        rig.pump_overworld(&mut policy).expect("the turn lands on the second attempt");

        let requests = rig.requests();
        assert!(requests.len() >= 2, "the truncated reply was nudged rather than accepted");
        let nudge = last_user_message(&requests[1]);
        assert!(nudge.contains("cut off"), "{nudge}");
        assert!(nudge.contains("briefly"), "the correction asked for is a shorter thought: {nudge}");
        assert!(nudge.contains("choose_action"), "and it still quotes the contract: {nudge}");
    }

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
        // …then change the question before the poll that would have answered it.
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

    #[test]
    fn a_reply_with_no_tool_call_is_nudged_once_then_forced_to_wait() {
        let (mut rig, mut policy) = Rig::new(vec![
            says("I think I will head north and see what happens."),
            says("Yes, north is definitely the way."),
        ]);

        // The next turn runs out of script, so this pumps for a bounded time.
        rig.pump_overworld_for(&mut policy, Duration::from_secs(2));

        let requests = rig.requests();
        assert!(requests.len() >= 2, "the model got exactly one nudge before being overruled");
        assert!(last_user_message(&requests[1]).contains("no tool call"), "{}", last_user_message(&requests[1]));
        assert!(last_user_message(&requests[1]).contains("choose_action"), "the nudge quotes the contract");

        // And it is visible.
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

    /// A `wait` answers the question that was asked.
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

    #[test]
    fn an_id_the_turn_never_offered_is_refused_without_ending_the_turn() {
        let (mut rig, mut policy) = Rig::new(vec![
            calls(&[("choose_action", r#"{"id":"PalletTown:99,99:Warp","summary":"out of the lab"}"#)]),
            calls(&[("wait", r#"{"ticks":1,"summary":"waiting"}"#)]),
        ]);

        rig.pump_overworld_for(&mut policy, Duration::from_secs(2));

        let requests = rig.requests();
        assert!(requests.len() >= 2, "the turn carries on after the id is refused");
        // The complaint is a `tool` result inside the same turn, not the next turn's situation.
        let answer = requests[1]
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::Tool)
            .and_then(Message::text)
            .expect("the refused call is answered like any other tool call");
        assert!(answer.contains("PalletTown:99,99:Warp"), "the model is told which id failed: {answer}");
        assert!(answer.contains("not one of this turn's actions"), "{answer}");
        assert!(answer.contains("OaksLab"), "it must say where the player actually is: {answer}");
        for request in &requests {
            history_is_well_formed(request);
        }
    }

    // ── Chained actions ──────────────────────────────────────────────────────────────────────────

    /// The second chained action costs no request.
    #[test]
    fn a_chained_action_is_taken_without_asking_the_model_again() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let ids = rig.action_ids(2);
        rig.endpoint.replies.lock().unwrap().push_back(calls(&[(
            "choose_action",
            &format!(r#"{{"id":"{}","then":["{}"],"summary":"heal, then leave"}}"#, ids[0], ids[1]),
        )]));

        let first = rig.pump_overworld(&mut policy).expect("the first action lands");
        assert_eq!(tools::overworld_id(&rig.state(), &first), ids[0]);
        assert_eq!(rig.requests().len(), 1);

        // The agent reports that it arrived, which is the only signal a chain advances on.
        policy.on_event(&AgentEvent::OverworldActionCompleted { destination: first.tile });
        let second = rig.tick_overworld(&mut policy).expect("the chained action follows immediately");
        assert_eq!(tools::overworld_id(&rig.state(), &second), ids[1]);
        assert_eq!(rig.requests().len(), 1, "the chained action must cost no second request");

        // Once the chain is spent the model is asked again.
        policy.on_event(&AgentEvent::OverworldActionCompleted { destination: second.tile });
        assert!(rig.pump_overworld_for(&mut policy, Duration::from_millis(300)).is_none());
        rig.wait_for_requests(2, Duration::from_secs(2));
        assert_eq!(rig.requests().len(), 2, "the end of a chain is an ordinary decision point");
    }

    /// Anything that stops one chained action stops the rest, and the model is told where it got.
    #[test]
    fn a_chain_stops_where_the_agent_was_stopped_and_says_where_it_got_to() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let ids = rig.action_ids(3);
        rig.endpoint.replies.lock().unwrap().push_back(calls(&[(
            "choose_action",
            &format!(
                r#"{{"id":"{}","then":["{}","{}"],"summary":"three in a row"}}"#,
                ids[0], ids[1], ids[2],
            ),
        )]));
        rig.endpoint.replies.lock().unwrap().push_back(calls(&[("wait", r#"{"ticks":1,"summary":"think"}"#)]));

        let first = rig.pump_overworld(&mut policy).expect("the first action lands");
        policy.on_event(&AgentEvent::OverworldActionAborted {
            destination: first.tile,
            reason: OverworldActionAbortedReason::Textbox,
            at: None,
        });

        // One tick: polling on would let this turn's `wait` expire and buy a third turn.
        assert!(rig.tick_overworld(&mut policy).is_none(), "the chain is dropped rather than advanced");
        rig.wait_for_requests(2, Duration::from_secs(2));
        let requests = rig.requests();
        assert_eq!(requests.len(), 2, "a stopped chain hands the decision back");
        let situation = last_user_message(&requests[1]);
        assert!(situation.contains("were not tried"), "the model is told the rest was dropped: {situation}");
        assert!(situation.contains(&ids[0]), "and which action was stopped: {situation}");
    }

    /// The other half of the same call.
    #[test]
    fn a_battle_takes_the_action_up_again_only_when_it_was_asked_to() {
        for (resume, expected_requests) in [(true, 1), (false, 2)] {
            let (mut rig, mut policy) = Rig::new(vec![]);
            let id = rig.first_action_id();
            rig.endpoint.replies.lock().unwrap().push_back(calls(&[(
                "choose_action",
                &format!(r#"{{"id":"{id}","resume_after_battle":{resume},"summary":"to the centre"}}"#),
            )]));
            rig.endpoint.replies.lock().unwrap().push_back(calls(&[("wait", r#"{"ticks":1,"summary":"think"}"#)]));

            let action = rig.pump_overworld(&mut policy).expect("the action lands");
            policy.on_event(&AgentEvent::OverworldActionAborted {
                destination: action.tile,
                reason: OverworldActionAbortedReason::Battle,
                at: None,
            });

            match resume {
                true => {
                    let again = rig.tick_overworld(&mut policy).expect("the same action is taken up again");
                    assert_eq!(tools::overworld_id(&rig.state(), &again), id);
                }
                false => {
                    assert!(rig.tick_overworld(&mut policy).is_none(), "the decision comes back");
                    rig.wait_for_requests(2, Duration::from_secs(2));
                }
            }
            assert_eq!(
                rig.requests().len(),
                expected_requests,
                "resume_after_battle={resume} should cost {expected_requests} request(s)",
            );
        }
    }

    /// An ending nothing named is not an ending that went well.
    #[test]
    fn a_chain_does_not_advance_on_an_ending_the_agent_never_reported() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let ids = rig.action_ids(2);
        rig.endpoint.replies.lock().unwrap().push_back(calls(&[(
            "choose_action",
            &format!(r#"{{"id":"{}","then":["{}"],"summary":"grass, then out"}}"#, ids[0], ids[1]),
        )]));
        rig.endpoint.replies.lock().unwrap().push_back(calls(&[("wait", r#"{"ticks":1,"summary":"think"}"#)]));

        assert!(rig.pump_overworld(&mut policy).is_some(), "the first action lands");
        // No event of any kind, which is exactly what a pace or a surf mount produces.
        assert!(rig.tick_overworld(&mut policy).is_none(), "the chain is dropped rather than advanced");
        rig.wait_for_requests(2, Duration::from_secs(2));
        assert_eq!(rig.requests().len(), 2, "the chain must not advance on silence");
    }

    /// A field move is decided by an overworld turn and collected by a different method.
    #[test]
    fn a_field_move_decision_is_collected_by_the_next_field_move_poll() {
        let (mut rig, mut policy) = Rig::new(vec![calls(&[(
            "use_field_move",
            r#"{"move":"reorder_party","slot":0}"#,
        )])]);

        // The overworld poll never yields an action for this…
        assert!(rig.pump_overworld_for(&mut policy, Duration::from_secs(2)).is_none());
        let state = rig.state();
        assert_eq!(policy.pick_field_move(&state), Some(FieldMove::ReorderParty { slot: 0 }));
        assert_eq!(policy.pick_field_move(&state), None, "it is taken, not repeated every tick");
    }

    /// A field move that cannot be carried out is a sentence to the model, never a silent no-op.
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

    /// On a turn with a menu, presses never reach the agent and the turn carries on to a decision.
    #[test]
    fn a_press_on_a_turn_with_a_menu_is_refused_and_the_turn_carries_on() {
        let (mut rig, mut policy) = Rig::new(vec![
            calls(&[("press_buttons", r#"{"buttons":["b","start","a"]}"#)]),
            calls(&[("wait", r#"{"ticks":1}"#)]),
        ]);

        rig.pump_overworld_for(&mut policy, Duration::from_secs(2));
        assert!(policy.take_manual_input().is_empty(), "no press may reach the agent from here");

        let requests = rig.requests();
        assert!(requests.len() >= 2, "the refusal is a tool result, so the turn recovers");
        let offered: Vec<&str> = requests[0].tools.iter().map(|tool| tool.function.name).collect();
        assert!(!offered.contains(&"press_buttons"), "not even offered: {offered:?}");
        assert!(offered.contains(&"report_issue"), "what replaced it: {offered:?}");

        // The refusal names the menu, and `report_issue` for a menu that is wrong.
        let refusal = requests[1]
            .messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .filter_map(Message::text)
            .next_back()
            .expect("the refusal is answered as a tool result")
            .to_string();
        assert!(refusal.contains("choose_action"), "{refusal}");
        assert!(refusal.contains("report_issue"), "{refusal}");
    }

    /// Each menu prompt is its own turn with scoped tools, answered in its `pick_*`'s shape.
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

    /// The wiring half of the guide nudge, which the prompt's own test cannot see.
    #[test]
    fn reading_the_guide_records_the_chapter_and_a_later_badge_says_so() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        let decide = || calls(&[("choose_action", &format!(r#"{{"id":"{id}","summary":"on we go"}}"#))]);
        for reply in [calls(&[("read_guide", "{}")]), decide()] {
            rig.endpoint.replies.lock().unwrap().push_back(reply);
        }

        assert_eq!(policy.guide_chapter_read, None, "nothing read yet");
        assert!(rig.pump_overworld(&mut policy).is_some(), "the turn lands");

        // The fixture is Oak's lab: no badges, so chapter 0, the Boulder Badge.
        assert_eq!(policy.guide_chapter_read, Some(0), "the read is recorded from the state it answered from");
        let requests = rig.requests();
        let chapter: Vec<&str> =
            requests[1].messages.iter().filter(|m| m.role == Role::Tool).filter_map(Message::text).collect();
        assert!(chapter[0].contains("Brock"), "chapter 0 really was served: {}", &chapter[0][..chapter[0].len().min(200)]);
        // Nothing is said while the chapter it read is the chapter it is in.
        assert!(!last_user_message(&requests[0]).contains("walkthrough"), "{}", last_user_message(&requests[0]));

        // Now the cell and the game disagree, which is what winning a badge does to them.
        policy.guide_chapter_read = Some(4);
        let before = rig.requests().len();
        rig.endpoint.replies.lock().unwrap().push_back(decide());
        assert!(rig.pump_overworld(&mut policy).is_some(), "and so does the next one");
        let after = rig.requests();
        assert!(after.len() > before, "a fresh turn went out");
        let situation = last_user_message(&after[before]);
        assert!(situation.contains("won a badge since you last read"), "{situation}");
        assert!(situation.contains(&crate::llm::guide::chapter_goal(0)), "and names the chapter: {situation}");
    }

    /// The mart's stock comes from the ROM through `ApiSnapshot`; `GameState` lacks it.
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

    /// One mart visit, several kinds, and the queue must not outlive the visit.
    #[test]
    fn a_mart_turn_can_buy_several_kinds_in_one_visit() {
        use crate::pokemon::item::ItemId;
        let (mut rig, mut policy) = Rig::new(vec![calls(&[(
            "buy_item",
            r#"{"item":"Potion","quantity":3,"then":[{"item":"PokeBall","quantity":10},{"item":"Antidote"}]}"#,
        )])]);

        let head = rig
            .pump_prompt(&mut policy, |policy, state| policy.pick_mart_purchase(state))
            .expect("the mart menu is answered");
        assert_eq!(head, Some(BagItem::new(ItemId::Potion, 3)));
        assert_eq!(policy.next_mart_purchase(), Some(BagItem::new(ItemId::PokeBall, 10)));
        // An omitted quantity is one here exactly as it is on the head order.
        assert_eq!(policy.next_mart_purchase(), Some(BagItem::new(ItemId::Antidote, 1)));
        assert_eq!(policy.next_mart_purchase(), None, "and then the shop closes");
    }

    /// A queued order must never be spendable at the next mart.
    #[test]
    fn an_abandoned_chain_is_not_spent_at_the_next_mart() {
        use crate::pokemon::item::ItemId;
        let (mut rig, mut policy) = Rig::new(vec![
            calls(&[("buy_item", r#"{"item":"Potion","then":[{"item":"PokeBall","quantity":10}]}"#)]),
            calls(&[("buy_item", r#"{"item":"Antidote"}"#)]),
        ]);

        rig.pump_prompt(&mut policy, |policy, state| policy.pick_mart_purchase(state))
            .expect("the first mart is answered");
        // The visit is abandoned with the PokeBall still queued, and a fresh turn is asked.
        let second = rig
            .pump_prompt(&mut policy, |policy, state| policy.pick_mart_purchase(state))
            .expect("the second mart is answered");
        assert_eq!(second, Some(BagItem::new(ItemId::Antidote, 1)));
        assert_eq!(policy.next_mart_purchase(), None, "the abandoned tail went with the old turn");
    }

    /// Answering the mid-battle forget prompt cancels the battle turn in flight.
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
            .pump_prompt(&mut policy, |policy, _| policy.pick_move_to_forget(0, &moves, PokemonMoveName::Bite))
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

    /// A slot the Pokémon lacks is declined, with the reason, rather than wedging the prompt.
    #[test]
    fn a_forget_slot_the_pokemon_does_not_have_declines_instead_of_hanging() {
        let (mut rig, mut policy) = Rig::new(vec![calls(&[("forget_move", r#"{"slot":3}"#)])]);
        let moves: Vec<PokemonMove> =
            [PokemonMoveName::Tackle, PokemonMoveName::Growl].into_iter().map(PokemonMove::with_max_pp).collect();

        let answer = rig
            .pump_prompt(&mut policy, |policy, _| policy.pick_move_to_forget(0, &moves, PokemonMoveName::Bite))
            .expect("it is answered rather than left hanging");
        assert_eq!(answer, None, "declining keeps all the moves it has");
    }

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

    #[test]
    fn a_full_context_is_summarised_and_the_next_turn_carries_the_summary() {
        let (mut rig, mut policy) = Rig::with_config(vec![], |config| config.context_limit = 8_000);
        let id = rig.first_action_id();
        let choose = format!(r#"{{"id":"{id}"}}"#);
        rig.push(vec![
            calls(&[("choose_action", &choose)]),
            calls(&[("choose_action", &choose)]),
            calls(&[("choose_action", &choose)]),
            // Enough prose in one turn to put it over `compact_above` of the window.
            saying_calls(&"I am thinking very hard about this. ".repeat(500), &[("choose_action", &choose)]),
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

        // The fifth turn opens on the system prompt and the summary.
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
            "the one-terminal-call contract has to survive the compaction",
        );
        // The tail is kept, so the turn that filled the window is still there.
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

    /// A second process's first request opens on the conversation the first one left.
    #[test]
    fn a_process_that_restarts_mid_run_opens_its_next_request_on_the_conversation_it_had() {
        let scratch = crate::run::Scratch::new("llm-restart");
        let said = "I am heading north out of Pallet Town to look for Oak.";

        let first_turns = {
            let (mut rig, mut policy) = Rig::with_config_in(vec![], Some(&scratch.0), |_| {});
            let id = rig.first_action_id();
            let choose = format!(r#"{{"id":"{id}"}}"#);
            rig.push(vec![
                saying_calls(said, &[("choose_action", &choose)]),
                calls(&[("choose_action", &choose)]),
            ]);
            rig.pump_overworld(&mut policy).expect("turn 1 lands");
            rig.pump_overworld(&mut policy).expect("turn 2 lands");
            let turns = rig.requests().last().expect("requests were sent").messages.len();
            drop(policy);
            drop(rig);
            turns
        };

        // The first process built a conversation worth restoring.
        assert!(first_turns > 3, "the first process only sent {first_turns} messages");
        let saved = std::fs::read_to_string(scratch.0.join(crate::run::files::HISTORY)).expect("a history");
        assert!(saved.contains(said), "the first process wrote its conversation down");

        let (mut rig, mut policy) = Rig::with_config_in(vec![], Some(&scratch.0), |_| {});
        let id = rig.first_action_id();
        rig.push(vec![calls(&[("choose_action", &format!(r#"{{"id":"{id}"}}"#))])]);
        rig.pump_overworld(&mut policy).expect("the resumed process plays on");

        let requests = rig.requests();
        let last = requests.last().expect("a request");
        assert_eq!(last.messages[0].role, Role::System, "index 0 is still the system prompt");
        assert_eq!(
            last.messages.iter().filter(|m| m.role == Role::System).count(),
            1,
            "and there is exactly one of it, not the stored copy behind a fresh one",
        );
        assert!(
            last.messages.iter().any(|m| m.text().is_some_and(|t| t.contains(said))),
            "the first process's own words came back",
        );
        assert!(
            last.messages.iter().any(|m| m.text() == Some(crate::llm::prompt::RESUMED_NOTE)),
            "and the model is told why the game may be behind them",
        );
        // The invariant the endpoint enforces with a 400.
        history_is_well_formed(last);

        drop(policy);
        drop(rig);
    }

    /// After a compaction the replaced conversation is gone from the request and still on disk.
    #[test]
    fn a_run_that_compacts_still_has_the_conversation_the_compaction_replaced_on_disk() {
        let scratch = crate::run::Scratch::new("llm-compactlog");
        let (mut rig, mut policy) =
            Rig::with_config_in(vec![], Some(&scratch.0), |config| config.context_limit = 8_000);
        let id = rig.first_action_id();
        let choose = format!(r#"{{"id":"{id}"}}"#);
        // The marker has to be in an early turn, not the one that fills the window.
        let doomed = "I remember standing outside the lab on the very first turn.";
        let filler = "I am thinking very hard about this. ".repeat(500);
        rig.push(vec![
            saying_calls(doomed, &[("choose_action", &choose)]),
            calls(&[("choose_action", &choose)]),
            calls(&[("choose_action", &choose)]),
            saying_calls(&filler, &[("choose_action", &choose)]),
            says("I am in Oak's lab with a Squirtle, about to leave for Route 1."),
            calls(&[("choose_action", &choose)]),
        ]);
        for turn in 1..=4 {
            rig.pump_overworld(&mut policy).unwrap_or_else(|| panic!("turn {turn} did not land"));
        }
        rig.events_until(Duration::from_secs(5), |event| matches!(event, UiEventBody::Compacted { .. }));
        rig.pump_overworld(&mut policy).expect("the run continues after a compaction");

        let requests = rig.requests();
        let last = requests.last().expect("a request");
        assert!(
            !last.messages.iter().any(|m| m.text().is_some_and(|t| t.contains(doomed))),
            "the compaction really did take it out of the conversation",
        );

        drop(policy);
        drop(rig);

        assert!(
            !std::fs::read_to_string(scratch.0.join(crate::run::files::HISTORY)).unwrap().contains(doomed),
            "and out of what the next process would resume on",
        );
        let logged = std::fs::read_to_string(scratch.0.join(crate::run::files::CONVERSATION)).expect("a log");
        assert!(logged.contains(doomed), "but the log kept what the summary replaced");
        assert!(
            logged.lines().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .any(|l| l["kind"] == "compaction"),
            "and says where it went",
        );
    }

    /// A turn in flight when `POST /api/new-run` lands belongs to the old game.
    #[test]
    fn the_conversation_a_new_run_leaves_behind_stays_with_the_run_that_had_it() {
        let old = crate::run::Scratch::new("llm-oldrun");
        let new = crate::run::Scratch::new("llm-newrun");
        let said = "I am about to be replaced by a brand new game.";

        let (mut rig, mut policy) = Rig::with_config_in(vec![], Some(&old.0), |_| {});
        let id = rig.first_action_id();
        let choose = format!(r#"{{"id":"{id}"}}"#);
        rig.push(vec![
            saying_calls(said, &[("choose_action", &choose)]),
            calls(&[("choose_action", &choose)]),
        ]);
        rig.pump_overworld(&mut policy).expect("the old game's turn lands");

        assert!(
            std::fs::read_to_string(old.0.join(crate::run::files::HISTORY)).unwrap().contains(said),
            "the old run wrote its conversation down before the restart",
        );

        policy.restart(Some(new.0.as_path()));
        rig.pump_overworld(&mut policy).expect("the new game's first turn lands");
        drop(policy);
        drop(rig);

        assert!(
            std::fs::read_to_string(old.0.join(crate::run::files::HISTORY)).unwrap().contains(said),
            "the old run keeps its own conversation",
        );
        let started = std::fs::read_to_string(new.0.join(crate::run::files::HISTORY))
            .expect("the new run has a history of its own from the moment it starts");
        assert!(!started.contains(said), "and the new run inherits none of it: {started}");
    }

    // ── The battle script ────────────────────────────────────────────────────────────────────────

    /// A script that decides every validation scenario and the committed battle fixture.
    const SCRIPT: &str = "if battle.best_move != () { battle.fight(battle.best_move); }\n\
                          if battle.can_run { battle.run(); }\n\
                          battle.ask();";

    /// A rig whose first overworld turn installs `source` and walks, plus `then` more replies.
    fn armed_with(source: &str, then: usize) -> (Rig, LlmPolicy) {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        let walk = format!(r#"{{"id":"{id}"}}"#);
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            replies.push_back(calls(&[
                ("set_battle_script", &serde_json::json!({ "script": source, "purpose": "a test" }).to_string()),
                ("choose_action", &walk),
            ]));
            for _ in 0..then {
                replies.push_back(calls(&[("choose_action", &walk)]));
            }
        }
        arm(&mut rig, &mut policy);
        (rig, policy)
    }

    /// Install a script on an overworld turn, the way the model would.
    fn arm(rig: &mut Rig, policy: &mut LlmPolicy) {
        let armed = rig.pump_overworld(policy);
        assert!(armed.is_some(), "the overworld turn still has to decide something");
        let results: Vec<String> = rig
            .events_until(Duration::from_secs(5), |event| {
                matches!(event, UiEventBody::ToolResult { name, .. } if name == "set_battle_script")
            })
            .into_iter()
            .filter_map(|event| match event {
                UiEventBody::ToolResult { name, content, .. } if name == "set_battle_script" => Some(content),
                _ => None,
            })
            .collect();
        assert_eq!(results.len(), 1, "one script was installed");
        assert!(results[0].starts_with("ok"), "and it armed: {}", results[0]);
    }

    /// The page is told what is fighting for the run, once, and then left alone.
    #[test]
    fn the_page_is_told_about_the_script_once_and_not_once_a_turn() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        let walk = format!(r#"{{"id":"{id}"}}"#);
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            replies.push_back(calls(&[
                ("set_battle_script", &serde_json::json!({ "script": SCRIPT, "purpose": "a test" }).to_string()),
                ("choose_action", &walk),
            ]));
            for _ in 0..3 {
                replies.push_back(calls(&[("choose_action", &walk)]));
            }
        }
        // The turn that arms it, then three that do not touch it.
        for turn in 0..4 {
            assert!(rig.pump_overworld(&mut policy).is_some(), "overworld turn {turn} decided nothing");
        }

        let published: Vec<(Option<String>, bool, bool)> = rig
            .events
            .try_iter()
            .filter_map(|event| match event.body {
                UiEventBody::BattleScript { source, armed, is_default, .. } => Some((source, armed, is_default)),
                _ => None,
            })
            .collect();
        // Two, and the first one is the default.
        assert_eq!(published.len(), 2, "one event per change, not one per turn: {published:#?}");
        assert!(published[0].2, "the first is the default: {published:#?}");
        assert!(!published[0].1, "which is never armed, or the page would call the battles free");
        assert_eq!(published[1].0.as_deref(), Some(SCRIPT), "and it carries the source, not a flag");
        assert!(published[1].1, "armed, which is the fact the panel is drawn from");
        assert!(!published[1].2, "and no longer the default");
    }

    #[test]
    fn a_scripted_battle_is_fought_without_a_single_request() {
        let (mut rig, mut policy) = armed_with(SCRIPT, 0);
        let before = rig.requests().len();

        rig.enter_battle();
        // Ten turns of a battle, each answered by the script alone.
        for turn in 0..10 {
            let action = rig
                .pump_battle(&mut policy, Duration::from_millis(200))
                .unwrap_or_else(|| panic!("the script did not decide battle turn {turn}"));
            assert!(
                crate::pokemon::policy::battle_options(&rig.state()).unwrap().contains(&action),
                "turn {turn} chose something the game never offered: {action}",
            );
        }

        assert_eq!(rig.requests().len(), before, "a scripted battle costs no requests at all");
    }

    /// The model is told what happened, once, on its next turn.
    #[test]
    fn what_the_script_did_reaches_the_model_on_the_next_turn() {
        let (mut rig, mut policy) = armed_with(SCRIPT, 1);

        rig.enter_battle();
        rig.pump_battle(&mut policy, Duration::from_millis(200)).expect("the script decides");
        policy.on_event(&AgentEvent::TextBox { message: "It's super effective!".into() });
        policy.on_event(&AgentEvent::BattleEnded);

        // Back outside, and the next turn carries the account of a battle nobody was asked about.
        rig.gb.load_state(FIXTURE).expect("back to the overworld fixture");
        rig.pump_overworld(&mut policy).expect("the next overworld turn lands");
        rig.wait_for_requests(2, Duration::from_secs(5));

        let situation = rig.requests().last().expect("a second request").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        assert!(situation.contains("### Battle report"), "no report in:\n{situation}");
        assert!(situation.contains("battle."), "{situation}");
        assert!(situation.contains("It's super effective!"), "the cartridge's own words: {situation}");

        // And exactly once.
        assert_eq!(situation.matches("### Battle report").count(), 1, "{situation}");
    }

    /// Enforced in the parser, not merely required in the schema.
    #[test]
    fn a_script_with_nothing_said_about_what_it_is_for_is_not_armed() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            replies.push_back(calls(&[
                ("set_battle_script", &serde_json::json!({ "script": SCRIPT }).to_string()),
                ("choose_action", &format!(r#"{{"id":"{id}"}}"#)),
            ]));
        }
        // Not `arm`, which asserts that the install succeeded.
        assert!(rig.pump_overworld(&mut policy).is_some(), "the turn still decides an action");

        // `events_until`, never `try_iter`: the worker thread publishes the tool result.
        let seen = rig.events_until(Duration::from_secs(5), |event| {
            matches!(event, UiEventBody::ToolResult { name, .. } if name == "set_battle_script")
        });
        let answer = seen.iter().find_map(|event| match event {
            UiEventBody::ToolResult { name, content, .. } if name == "set_battle_script" => Some(content.clone()),
            _ => None,
        }).expect("the tool answered");
        assert!(answer.contains("needs a `purpose`"), "and says what is missing: {answer}");
        assert!(answer.contains("nothing was changed"), "and that the script was not taken: {answer}");

        // The refusal has to leave the run on the default rather than half-armed.
        assert!(!policy.handles.live_script.state().eq(&ScriptState::Armed),
                "a refused script is not deciding battles");
    }

    /// The standing line is a constant unless it carries these two, and a constant is not read.
    #[test]
    fn an_armed_script_says_what_it_was_for_and_how_much_it_has_decided() {
        let (mut rig, mut policy) = armed_with(SCRIPT, 1);

        rig.enter_battle();
        rig.pump_battle(&mut policy, Duration::from_millis(200)).expect("the script decides");
        policy.on_event(&AgentEvent::BattleEnded);

        rig.gb.load_state(FIXTURE).expect("back to the overworld fixture");
        rig.pump_overworld(&mut policy).expect("the next overworld turn lands");
        rig.wait_for_requests(2, Duration::from_secs(5));

        let situation = rig.requests().last().expect("a second request").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        // `armed_with` installs it with this purpose.
        assert!(situation.contains("You installed it for: \"a test\""),
                "the standing line quotes the model's own words:\n{situation}");
        assert!(situation.contains("1 battle turn since you installed it"),
                "and counts what it has decided:\n{situation}");
        // Not what a broken counter renders.
        assert!(!situation.contains("not decided a battle turn yet"), "{situation}");
    }

    /// The report replaces the raw event stream rather than sitting beside it.
    #[test]
    fn a_scripted_battle_is_not_narrated_twice_in_the_same_request() {
        let (mut rig, mut policy) = armed_with(SCRIPT, 1);

        rig.enter_battle();
        policy.on_event(&AgentEvent::BattleStarted);
        rig.pump_battle(&mut policy, Duration::from_millis(200)).expect("the script decides");
        policy.on_event(&AgentEvent::TextBox { message: "WILD RATTATA appeared!".into() });
        policy.on_event(&AgentEvent::BattleEnded);

        rig.gb.load_state(FIXTURE).expect("back to the overworld fixture");
        rig.pump_overworld(&mut policy).expect("the next overworld turn lands");
        rig.wait_for_requests(2, Duration::from_secs(5));

        let situation = rig.requests().last().expect("a second request").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        assert_eq!(
            situation.matches("WILD RATTATA appeared!").count(), 1,
            "the battle is accounted for once, not once per mechanism:\n{situation}",
        );
    }

    /// What happens after the battle, before its report is closed, is the overworld's and stays.
    #[test]
    fn what_follows_a_scripted_battle_is_not_folded_into_its_report() {
        use crate::pokemon::tile::MetaTile;
        let (mut rig, mut policy) = armed_with(SCRIPT, 1);

        rig.enter_battle();
        policy.on_event(&AgentEvent::BattleStarted);
        rig.pump_battle(&mut policy, Duration::from_millis(200)).expect("the script decides");
        policy.on_event(&AgentEvent::BattleEnded);
        // The verdict on a pickup a trainer's battle interrupted lands after the battle.
        policy.on_event(&AgentEvent::OverworldPickupFailed { target: MetaTile::Sprite("Max Ether".into()) });

        rig.gb.load_state(FIXTURE).expect("back to the overworld fixture");
        rig.pump_overworld(&mut policy).expect("the next overworld turn lands");
        rig.wait_for_requests(2, Duration::from_secs(5));

        let situation = rig.requests().last().expect("a second request").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        assert!(situation.contains("nothing was picked up: the Max Ether"),
                "the failed pickup was dropped with the battle's events:\n{situation}");
    }

    /// One strike.
    #[test]
    fn a_script_that_fails_disarms_and_hands_the_turn_back() {
        // Broken on the second turn, yet validated clean.
        let broken = "if battle.turn > 1 { battle.fight(\"Hydro Cannon\"); }\n\
                      if battle.best_move != () { battle.fight(battle.best_move); }\n\
                      if battle.can_run { battle.run(); }\n\
                      battle.ask();";
        let (mut rig, mut policy) = armed_with(broken, 0);
        rig.endpoint.replies.lock().unwrap()
            .push_back(calls(&[("choose_battle_action", r#"{"id":"run"}"#)]));
        let before = rig.requests().len();

        rig.enter_battle();
        rig.pump_battle(&mut policy, Duration::from_millis(200)).expect("turn 1 is fine");
        assert_eq!(rig.requests().len(), before, "and cost nothing");

        rig.pump_battle(&mut policy, Duration::from_secs(5)).expect("the model answers turn 2 instead");
        rig.wait_for_requests(before + 1, Duration::from_secs(5));

        let situation = rig.requests().last().expect("a battle request").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        assert!(situation.contains("no longer deciding your battle turns"), "{situation}");
        assert!(situation.contains("Hydro Cannon"), "the reason names what it asked for: {situation}");
        assert!(situation.contains("set_battle_script"), "and how to fix it: {situation}");

        // And it stays disarmed: the next battle turn is the model's too, not a second failure.
        assert!(policy.handles.live_script.source().is_none(), "one strike disarms for the run");
    }

    #[test]
    fn a_battle_the_model_takes_over_is_not_decided_by_the_script_again() {
        // `SCRIPT` behind one line, so turn 2 deciding is proven by another test in this file.
        let asks_first = "if battle.turn == 1 { battle.ask(); }\n\
                          if battle.best_move != () { battle.fight(battle.best_move); }\n\
                          if battle.can_run { battle.run(); }\n\
                          battle.ask();";
        let (mut rig, mut policy) = armed_with(asks_first, 0);
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            replies.push_back(calls(&[("choose_battle_action", r#"{"id":"run","take_over":true}"#)]));
            replies.push_back(calls(&[("choose_battle_action", r#"{"id":"run"}"#)]));
        }
        let before = rig.requests().len();

        rig.enter_battle();
        rig.pump_battle(&mut policy, Duration::from_secs(5)).expect("the model answers turn 1");
        rig.wait_for_requests(before + 1, Duration::from_secs(5));

        // The assertion is the request, not the action.
        rig.pump_battle(&mut policy, Duration::from_secs(5)).expect("the model answers turn 2 too");
        rig.wait_for_requests(before + 2, Duration::from_secs(5));

        let situation = rig.requests().last().expect("a second battle request").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        assert!(situation.contains("You took this battle over"), "{situation}");
        assert!(situation.contains("as soon as this battle ends"), "and that it expires: {situation}");

        // Still armed, which is what makes this not a disarm under another name.
        assert!(policy.handles.live_script.source().is_some(), "the script survives the takeover");
        assert_eq!(policy.handles.live_script.state(), ScriptState::Armed, "and stays armed");

        // The takeover ends with the battle, and the next one is scripted again.
        policy.on_event(&AgentEvent::BattleEnded);
        rig.endpoint.replies.lock().unwrap()
            .push_back(calls(&[("choose_battle_action", r#"{"id":"run"}"#)]));
        rig.enter_battle();
        rig.pump_battle(&mut policy, Duration::from_secs(5)).expect("turn 1 of the next battle asks");
        rig.wait_for_requests(before + 3, Duration::from_secs(5));
        let spent = rig.requests().len();
        rig.pump_battle(&mut policy, Duration::from_millis(200)).expect("and turn 2 is the script's");
        assert_eq!(rig.requests().len(), spent, "which cost no request at all");
    }

    #[test]
    fn an_ask_says_what_the_script_did_since_the_model_last_chose() {
        let asks_on_odd_turns = "if battle.turn % 2 == 1 { battle.ask(); }\n\
                                 if battle.best_move != () { battle.fight(battle.best_move); }\n\
                                 battle.ask();";
        let (mut rig, mut policy) = armed_with(asks_on_odd_turns, 0);
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            for _ in 0..2 {
                replies.push_back(calls(&[("choose_battle_action", r#"{"id":"run"}"#)]));
            }
        }
        let before = rig.requests().len();

        rig.enter_battle();
        // Turn 1 asks.
        rig.pump_battle(&mut policy, Duration::from_secs(5)).expect("the model answers turn 1");
        rig.wait_for_requests(before + 1, Duration::from_secs(5));
        let first = rig.requests().last().expect("the first battle request").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        assert!(!first.contains("while you were not being asked"), "nothing to report yet: {first}");

        // Turn 2 is the script's, and turn 3 comes back to the model — carrying turn 2.
        rig.pump_battle(&mut policy, Duration::from_millis(200)).expect("turn 2 is the script's");
        rig.pump_battle(&mut policy, Duration::from_secs(5)).expect("the model answers turn 3");
        rig.wait_for_requests(before + 2, Duration::from_secs(5));

        let situation = rig.requests().last().expect("the second battle request").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        assert!(situation.contains("while you were not being asked"), "{situation}");
        assert!(situation.contains("2. "), "with the turn numbered as the report numbers it: {situation}");
        // And the way out is named on the turn that can take it.
        assert!(situation.contains("take_over"), "and how to stop it: {situation}");
    }

    /// A reset disarms immediately, not at the worker's next turn.
    #[test]
    fn a_reset_stops_the_old_games_script_deciding_the_new_games_battles() {
        let (mut rig, mut policy) = armed_with(SCRIPT, 0);
        assert!(policy.handles.live_script.source().is_some(), "armed to begin with");

        policy.restart(None);
        assert!(policy.handles.live_script.source().is_none(), "and disarmed the moment the game changed");

        // Which means the very next battle turn is the model's, not a script's.
        rig.endpoint.replies.lock().unwrap()
            .push_back(calls(&[("choose_battle_action", r#"{"id":"run"}"#)]));
        rig.enter_battle();
        rig.pump_battle(&mut policy, Duration::from_secs(5)).expect("the model answers it");
    }

    #[test]
    fn a_battle_turn_on_the_default_script_says_so_and_names_the_tools_in_order() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let walk = format!(r#"{{"id":"{}"}}"#, rig.first_action_id());
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            replies.push_back(calls(&[("choose_battle_action", r#"{"id":"run"}"#)]));
            replies.push_back(calls(&[("choose_action", &walk)]));
        }

        rig.enter_battle();
        rig.pump_battle(&mut policy, Duration::from_secs(5)).expect("the model answers it");

        let situation = rig.requests().last().expect("a battle request").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        assert!(situation.contains("still the default one"), "{situation}");
        // The battle turn states the cost and names no tool, because it carries none of them.
        for tool in ["read_battle_script", "get_battle_script_docs", "set_battle_script"] {
            assert!(!situation.contains(tool), "the battle turn does not name {tool}: {situation}");
        }

        // All three, in the order they have to be called, on the turn that has them.
        rig.gb.load_state(FIXTURE).expect("back to the overworld fixture");
        rig.pump_overworld(&mut policy).expect("the overworld turn lands");
        rig.wait_for_requests(2, Duration::from_secs(5));
        let overworld = rig.requests().last().expect("an overworld request").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        assert!(overworld.contains("still the default one"), "{overworld}");
        let read = overworld.find("read_battle_script").unwrap_or_else(|| panic!("{overworld}"));
        let docs = overworld.find("get_battle_script_docs").unwrap_or_else(|| panic!("{overworld}"));
        let set = overworld.find("set_battle_script").unwrap_or_else(|| panic!("{overworld}"));
        assert!(read < docs && docs < set, "named in the order they are called: {overworld}");
    }

    #[test]
    fn every_battle_turn_after_a_failure_still_says_the_script_is_broken() {
        let broken = "if battle.turn > 1 { battle.fight(\"Hydro Cannon\"); }\n\
                      if battle.best_move != () { battle.fight(battle.best_move); }\n\
                      battle.ask();";
        let (mut rig, mut policy) = armed_with(broken, 0);
        // Queued here, not through `armed_with`'s `then`, as the endpoint answers in order.
        let walk = format!(r#"{{"id":"{}"}}"#, rig.first_action_id());
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            for _ in 0..2 {
                replies.push_back(calls(&[("choose_battle_action", r#"{"id":"run"}"#)]));
            }
            replies.push_back(calls(&[("choose_action", &walk)]));
        }
        let before = rig.requests().len();

        rig.enter_battle();
        rig.pump_battle(&mut policy, Duration::from_millis(200)).expect("turn 1 is scripted");
        rig.pump_battle(&mut policy, Duration::from_secs(5)).expect("turn 2 fails and comes back");
        rig.wait_for_requests(before + 1, Duration::from_secs(5));

        let failing = rig.requests().last().expect("the failing turn").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        assert!(failing.contains("Hydro Cannon"), "the note carries the reason: {failing}");
        // The note is the account of that turn, so the state line does not repeat it.
        assert!(!failing.contains("failed and is no longer deciding your battle turns, so they"),
                "the note and the state line are alternatives: {failing}");

        rig.pump_battle(&mut policy, Duration::from_secs(5)).expect("and the next turn too");
        rig.wait_for_requests(before + 2, Duration::from_secs(5));
        let next = rig.requests().last().expect("the turn after").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        assert!(next.contains("no longer deciding your battle turns"),
                "a run whose script broke has to keep being told: {next}");
        assert!(!next.contains("read_battle_script"),
                "the battle turn does not name a tool it is not offered: {next}");
        assert!(!next.contains("Hydro Cannon"),
                "nor repeat the reason it cannot act on: {next}");

        // The overworld turn, which holds the tools, is where it all lands.
        policy.on_event(&AgentEvent::BattleEnded);
        rig.gb.load_state(FIXTURE).expect("back to the overworld fixture");
        rig.pump_overworld(&mut policy).expect("the overworld turn after the failure lands");
        rig.wait_for_requests(before + 3, Duration::from_secs(5));
        let overworld = rig.requests().last().expect("the overworld turn").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        assert!(overworld.contains("no longer deciding your battle turns"),
                "the state is said where it can be acted on: {overworld}");
        assert!(overworld.contains("Hydro Cannon"),
                "with the reason, rather than a round trip away in `read_battle_script`: {overworld}");
        assert!(overworld.contains("set_battle_script"),
                "and the tool that arms a corrected one: {overworld}");
        // The claim that the tools are here.
        assert!(overworld.contains("this is a turn that can fix it"), "{overworld}");
    }

    /// `battle.ask()` hands back only the turns the script says are worth paying for.
    #[test]
    fn a_script_can_hand_one_turn_back_and_stay_armed() {
        let asking = "if battle.me.level > 3 { battle.ask(); }\nbattle.ask();";
        let (mut rig, mut policy) = armed_with(asking, 0);
        rig.endpoint.replies.lock().unwrap()
            .push_back(calls(&[("choose_battle_action", r#"{"id":"run"}"#)]));

        rig.enter_battle();
        rig.pump_battle(&mut policy, Duration::from_secs(5)).expect("the model answers the asked turn");

        let situation = rig.requests().last().expect("a battle request").messages.last()
            .expect("a situation").text().unwrap_or_default().to_string();
        assert!(situation.contains("handed this turn to you"), "{situation}");
        assert!(!situation.contains("no longer deciding"), "asking is not failing: {situation}");
        // Not told twice: the note already accounts for this turn, with its prints.
        assert!(!situation.contains("but it did not decide this one"),
                "the note stands alone where there is one: {situation}");
        assert!(!situation.contains("No battle script is set"), "there very much is one: {situation}");
        assert!(policy.handles.live_script.source().is_some(), "and the script is still armed");
    }
}
