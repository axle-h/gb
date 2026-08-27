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

use std::collections::VecDeque;
use std::sync::atomic::Ordering;

use crate::joypad::JoypadButton;
use crate::llm::battle_report::{BattleReport, MAX_QUEUED as MAX_QUEUED_REPORTS};
use crate::llm::battle_script::{self, Outcome as ScriptOutcome};
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
    /// The `choose_action` call being carried out, if one is — see [`ActionQueue`]. `None` between
    /// decisions, which is the state in which a new turn may be asked for.
    queue: Option<ActionQueue>,
    /// What became of the overworld action handed over last, as the agent reported it. Written by
    /// [`Policy::on_event`] and read once, by [`Self::advance_queue`].
    ///
    /// ⚠️ **`None` is a real answer and not just "nothing yet".** Several endings leave
    /// `OverworldMovement` for a driver of their own without reporting anything — grass and cave
    /// pacing, and mounting Surf — and each of those hands the decision back on purpose. Reading a
    /// missing outcome as success would carry a chain on past a step that never happened.
    outcome: Option<ActionOutcome>,
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
    /// The battle being fought by the script, written up as it goes. `None` outside a battle and in
    /// any battle the script is not deciding — a battle the model answers turn by turn narrates
    /// itself through the ordinary event buffer and needs no report.
    battle_report: Option<BattleReport>,
    /// Finished reports, waiting for the next turn of any kind to carry them.
    ///
    /// ⚠️ **Not one slot.** `resume_after_battle` exists precisely so several battles can happen
    /// between two overworld decisions, and a single slot would silently drop all but the last.
    reports: Vec<String>,
    /// Kinds `buy_item`'s `then` queued for the current mart visit, drained by
    /// [`Policy::next_mart_purchase`]. Cleared whenever the model is asked a fresh mart turn.
    mart_queue: std::collections::VecDeque<BagItem>,
    /// A report whose battle has ended, waiting for the game to be observed once more so it can be
    /// closed against something.
    ///
    /// ⚠️ **`BattleEnded` is the wrong moment to finish, and finishing there was the bug.**
    /// `service_tools` runs only at decision points, so the last state this policy holds when a
    /// battle ends is the one the *last turn opened with* — closing against that reports every
    /// one-shot KO as the foe standing at full HP. Held here instead until the next observation,
    /// where the party carries our real HP.
    finishing: Option<BattleReport>,
    /// The most recent state that still had a battle in it, for closing the last turn of a report.
    ///
    /// ⚠️ **`self.state` is not good enough and the turn it gets wrong is the interesting one.**
    /// `BattleEnded` arrives on a tick whose `GameState` may already have `battle: None`, and a
    /// report closed against that has no HP to diff — so the move that actually won the fight is
    /// the one turn reported without a number. ⚠️ **It costs nothing**: the previous poll's box is
    /// *moved* here rather than cloned, so this is a pointer swap on a `GameState` that was already
    /// allocated.
    last_battle_state: Option<Box<GameState>>,
}

/// A note about the script, with whatever it printed before it stopped.
///
/// ⚠️ **The prints are the half that is actionable.** "It chose a move BULBASAUR does not know" says
/// what happened; the script's own `print` lines say which branch it was in when it did.
fn script_note(headline: &str, prints: &[String]) -> String {
    match prints.is_empty() {
        true => headline.to_string(),
        false => format!(
            "{headline}\n\nIt printed, before it stopped:\n{}",
            prints.iter().map(|line| format!("  {line}\n")).collect::<String>(),
        ),
    }
}

/// What every LLM-played run calls its trainer. See [`Policy::player_name`] below for why this is a
/// constant rather than something derived from `GB_MODEL`.
pub(crate) const PLAYER_NAME: &str = "AI";

/// How many battles one action may be resumed through before the decision is handed back anyway.
///
/// ⚠️ **A cap is needed even though a battle is itself a stream of decisions.** The model keeps
/// answering battle turns throughout, so it is never locked out of the run — but it is locked out of
/// the *overworld*, and the overworld is where the answer to "half the party has fainted, go and
/// heal instead" lives. Five is a long route's worth of trainers and wild encounters; past that, an
/// action the model chose several minutes ago is no longer obviously the action it would choose now.
const MAX_BATTLE_RESUMES: u8 = 5;

/// What became of the overworld action the policy last handed to the agent.
///
/// ⚠️ **Read off the agent's own events rather than by re-observing the world.** Whether the walk
/// arrived is not a thing a `GameState` says: the player standing on the destination tile is true
/// both of a warp that fired and of one that was refused, and an interaction's only success signal
/// is a text box the agent has already consumed. The agent knows, and says so once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionOutcome {
    /// `OverworldActionCompleted`, or `OverworldInteractionCompleted` for a person or a PC — the two
    /// are one thing here, and are two events only because a conversation has no arrival tile.
    Landed,
    /// `OverworldActionAborted`. The reason is carried because exactly one of them may be resumed
    /// from, and telling them apart is the whole of that feature.
    Stopped(OverworldActionAbortedReason),
}

/// One `choose_action` call, while the agent is working through it.
///
/// ⚠️ **This exists for every call, not only a chained one.** A single action needs the same
/// bookkeeping the moment `resume_after_battle` is on, and giving the two shapes one representation
/// is what stops "was this chained?" being asked at each of the four places that end a chain.
struct ActionQueue {
    /// The id the agent is carrying out now.
    current: String,
    /// The ids the model chained behind it, in the order it wrote them.
    rest: VecDeque<String>,
    /// What has already landed. Only ever read to say where a dropped chain got to.
    done: Vec<String>,
    /// From the tool call: a battle does not end `current`.
    resume_after_battle: bool,
    /// Battles `current` has already been resumed through. Reset by moving on to the next id, since
    /// the budget is per action rather than per call.
    resumes: u8,
}

/// Why what was left of a chain was thrown away. Each one reads differently to the model because
/// each one wants something different done about it.
enum Dropped {
    /// The id no longer matches anything on the live map. The sentence differs for a stale map
    /// prefix and for a world that moved under a decision, so it is composed by
    /// [`unresolved_note`] where the state is in hand.
    Unresolved(String),
    /// The agent aborted the action and said why.
    Stopped(OverworldActionAbortedReason),
    /// The action ended without the agent naming an outcome — see [`LlmPolicy::outcome`].
    Unreported,
    /// [`MAX_BATTLE_RESUMES`], spent.
    Resumes,
}

/// The two different mistakes an id that will not resolve can be, in the words that say which.
///
/// ⚠️ **Telling the model the wrong one is worse than saying nothing.** An id whose map prefix is
/// not the map the player is on was never on this turn's menu at all — the model quoted one from an
/// earlier turn — and "the game moved on" invites it to try the same thing again. An id for *this*
/// map that no longer resolves really is the world having changed while it decided.
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
            last_battle_state: None,
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
        let Some((mut situation, headline, menu)) = ({
            // No state has been observed yet, so there is nothing to describe. `service_tools` runs
            // immediately before every poll site, so this is only ever true before the first tick.
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
                    // Neither does W9's `Stuck`, and there the absence *is* the situation.
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
                // ⚠️ The ids the situation was rendered from, not a second list built beside it:
                // `tools::classify` refuses anything not in here, so the two disagreeing would
                // reject an action the model was told it could take.
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
        self.events.clear();
        // ⚠️ **Spent by the turn that carried them.** They are in `situation` now, and a report left
        // here would be re-rendered into every turn until the next battle overwrote it.
        self.reports.clear();
        // The events this report was going to replace have just gone, so it has nothing left to
        // take back — see `BattleReport::events_mark`.
        if let Some(report) = self.battle_report.as_mut() {
            report.events_mark = 0;
        }

        if self.handles.turns.send(TurnRequest { id, kind, situation, headline, menu }).is_ok() {
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

    /// Let the model's own script decide this battle turn, if it has one and it can.
    ///
    /// ⚠️ **Four guards, and each is a case that would otherwise be wrong rather than merely
    /// wasteful.** A Safari battle has a different action set entirely and `postgame::safari` has
    /// bespoke logic for it; a turn already in flight has been paid for and must be allowed to
    /// land — including the one the script asked for a moment ago; and with no script there is
    /// nothing to run. The `state.battle` check is what makes the report's `open` infallible.
    ///
    /// ⚠️ **A failure disarms the script for the whole run, not for this battle.** It is one strike
    /// because each failure costs a full request against the history to report — so disarming for a
    /// battle only moves that cost to the next one, and disarming for a turn pays it every turn.
    /// The reason goes back through [`battle_script::Live`], which the worker drains onto disk at
    /// the top of the very turn the failure caused.
    fn run_battle_script(&mut self, state: &GameState) -> Option<BattleAction> {
        if self.pending.is_some() || self.waiting.is_some() {
            return None;
        }
        let battle = state.battle.as_ref()?;
        if battle.battle_type == crate::pokemon::battle::BattleType::Safari {
            return None;
        }
        let source = self.handles.live_script.source()?;

        // ⚠️ **No `expect` here, even though the `?` above makes one unreachable.** This runs on
        // the thread that owns the `GameBoy`, so a panic takes the run's checkpoint with it — the
        // argument `web::audio` makes for wrapping the encoder. A `None` simply means this turn is
        // not scripted, which is always a safe answer.
        // A second battle can start before the game was ever observed out of the first — a trainer
        // straight after a wild encounter. Close the old one on what there is rather than letting it
        // collect a different battle's turns.
        if self.finishing.is_some() {
            self.close_battle_report(None);
        }
        let report = match self.battle_report.as_mut() {
            Some(report) => report,
            None => self.battle_report.insert(BattleReport::open(state, self.events.len())?),
        };
        let turn = report.decisions() as u32 + 1;
        let evaluation = battle_script::run(&source, state, turn);

        match evaluation.outcome {
            ScriptOutcome::Action(action) => {
                report.decided(state, &action, evaluation.prints);
                Some(action)
            }
            // The script wants this one answered properly. It stays armed; the turn falls through to
            // the ordinary path, and anything it printed rides on the situation as its argument.
            ScriptOutcome::Ask => {
                report.handed_back(state);
                self.note = Some(script_note("Your battle script handed this turn to you.", &evaluation.prints));
                None
            }
            ScriptOutcome::Failed(why) => {
                report.handed_back(state);
                self.handles.live_script.failed(&why);
                self.note = Some(script_note(
                    &format!(
                        "**Your battle script failed and is no longer deciding your battle turns.** \
                         {why}\n\nAnswer this turn yourself. When you are next in the overworld, \
                         `read_battle_script` to see it, fix it and `set_battle_script` again, or \
                         leave it off and keep answering battles as you always have.",
                    ),
                    &evaluation.prints,
                ));
                None
            }
        }
    }

    /// Close the report whose battle has ended and queue it for the next turn.
    ///
    /// `observed` is the game as it stands now, if it has been looked at since the battle finished.
    /// `Some` with no battle in it is the **ordinary** case and is what lets the closing line read
    /// our own HP out of the party; `None` falls back to the last state that still had a battle,
    /// which is all a second battle starting immediately leaves us.
    ///
    /// ⚠️ **`self.state` is deliberately not a fallback, and using it was the bug.** It is the state
    /// of the last *decision*, so closing against it reports every one-shot KO as the foe standing
    /// at the HP it started the turn on. No line at all beats a wrong one.
    ///
    /// ⚠️ **The events the report replaces are taken back.** Every message box in a scripted battle
    /// was folded into `self.events` on its way past and would appear under
    /// `### Since your last decision` as well as in the report, in two shapes, in the same request.
    /// `events_mark` is where the buffer stood when the battle opened; anything after it is the
    /// battle, and the report is the better account of it.
    fn close_battle_report(&mut self, observed: Option<&GameState>) {
        let Some(report) = self.finishing.take() else { return };
        let mark = report.events_mark.min(self.events.len());
        self.events.truncate(mark);
        let fallback = self.last_battle_state.take();
        let rendered = report.finish(observed.or(fallback.as_deref()));
        // Oldest first: a run that fought five battles between two overworld turns has more use for
        // the two most recent, and the count of what went is on the report itself.
        if self.reports.len() >= MAX_QUEUED_REPORTS {
            self.reports.remove(0);
        }
        self.reports.push(rendered);
    }

    /// The turn the model does not pay for: the next action of a `choose_action` that carried more
    /// than one, or the same action again after a battle interrupted it.
    ///
    /// ⚠️ **It runs before [`Self::advance`] and never touches `pending`, `waiting` or the
    /// generation.** This is the same shape `pick_field_move` has and for the same reason — a
    /// decision the model has already taken is being handed over, not asked for. Starting a turn
    /// here would cancel nothing and cost a completion for an answer that is already in hand.
    ///
    /// ⚠️ **Only a landed action advances the chain, and only a *battle* may be resumed through.**
    /// Every other ending stops it, which is the conservative half of the design and the load
    /// bearing one: a text box, a locked door and a guard turning the player back are the game
    /// saying something, and carrying on past that is exactly the loop the whole agent is built to
    /// avoid — the deployed run aborted on the same square 143 times without ever being asked to.
    /// A battle is the one interruption that means nothing about the action: it ends by itself, the
    /// world is where it was, and the walk was going to be re-issued verbatim.
    ///
    /// Returns the action to carry out, or `None` — which is either "no chain is running" or "the
    /// chain has just ended", the second having left a note for the turn [`Self::advance`] is about
    /// to start.
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
        // Spent either way: what happens next has been decided from it, and leaving it behind would
        // have the next action in the chain judged by the outcome of the one before it.
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
                    // The whole chain landed. Deliberately no note: every one of those actions is a
                    // `✓` line in this very turn's `### Since your last decision`, and saying it a
                    // second time in different words is a second thing to reconcile.
                    None => {
                        self.queue = None;
                        return None;
                    }
                }
            }
        }
        self.take_current(state)
    }

    /// Resolve the id at the head of the queue against the live game and hand it over.
    ///
    /// ⚠️ **Against a freshly recomputed action list, never the menu the chain was written from.**
    /// That is what makes a chain safe to offer at all: `actions()` is re-derived here, so an id
    /// that stopped being true — because the action before it took the player through a door — fails
    /// to match and the chain ends with a sentence, rather than matching something else that happens
    /// to sit at those coordinates on the new map.
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
    ///
    /// ⚠️ **A single action that was stopped gets no note at all, and that is the point of the two
    /// early returns.** The agent has already reported the abort, in the very turn this note would
    /// be prepended to; a second account of it in the policy's words is a second thing to reconcile
    /// and a way for the two to disagree. What is worth saying is only ever the part the agent
    /// cannot know: that there was more queued behind it, or that a resume budget ran out.
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
    ///
    /// ⚠️ **This used to be `GB_MODEL` shortened to fit, and the shortening was wrong more often
    /// than it was right.** Seven characters ([`MAX_PLAYER_NAME`](crate::pokemon::MAX_PLAYER_NAME))
    /// cannot hold a model id, so every name was a guess at which half of one mattered, and the
    /// guess routinely produced a model that does not exist: `openai/gpt-5.4-nano` came out `GPT54`,
    /// which is the same failure the whole-segments rule was written one level up to avoid
    /// (`gemma-3-12b` → `GEMMA31`). It is also a *lossy second copy* of something already recorded
    /// exactly — `meta.json` and `hall-of-fame/ledger.jsonl` both carry the full id — and the two
    /// could disagree, because the name is written once into the save and `GB_MODEL` can change
    /// under a restart.
    ///
    /// So the trainer card says who is playing in the only sense it can hold, and the model id stays
    /// where it is unambiguous. `RandomPolicy` still draws a name from its own list and
    /// `ConsolePolicy` is still `HUMAN`: those are not abbreviations of anything, so they lose
    /// nothing by being spelled out.
    fn player_name(&self) -> Option<String> {
        Some(PLAYER_NAME.to_string())
    }

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
        self.snapshot.arrival = graph.arrival();
        // The outgoing box is *moved* rather than cloned, so keeping the last in-battle state for
        // `close_battle_report` costs a pointer swap. See `last_battle_state`.
        if let Some(previous) = self.state.replace(Box::new(state.clone())) {
            if self.battle_report.is_some() && previous.battle.is_some() {
                self.last_battle_state = Some(previous);
            }
        }
        // ⚠️ **This is the observation the report was waiting for.** It runs immediately before
        // every poll site, so a report closed here is on `self.reports` before the very next
        // `start_turn` renders them.
        if self.finishing.is_some() && state.battle.is_none() {
            self.close_battle_report(Some(state));
        }

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
        // ⚠️ **Before `advance`, so a chain still running never starts a turn.** Saving the model a
        // request is the whole of what a chain buys; asking it anyway and throwing the answer away
        // would buy the opposite.
        if let Some(action) = self.advance_queue(state) {
            return Some(action);
        }
        match self.advance(DecisionKind::Overworld, TurnContext::None)? {
            Terminal::ChooseAction { id, then, resume_after_battle } => {
                // The queue is built even for a lone action with nothing chained and no resume: it
                // is the record of what is being carried out, and `take_current` is then the one
                // place an id is resolved — see [`Self::take_current`] for why that has to be
                // against a freshly recomputed list.
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
        // ⚠️ **Before `advance`, for the reason `advance_queue` is** — a decision already taken is
        // being handed over rather than asked for, and starting a turn here would buy a completion
        // for an answer that is in hand. The difference is that this one *takes* the decision, so it
        // is also the seam that makes a whole battle cost no requests at all.
        if let Some(action) = self.run_battle_script(state) {
            return Some(action);
        }
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
        // ⚠️ **Cleared here rather than when the shop closes.** This is the one call that means "the
        // model is being asked afresh", so draining it here is what makes a leftover order from an
        // abandoned visit impossible to spend at the *next* mart — there is no shop-closed callback
        // to hang it on, and a queue with no single point of truth is one that outlives its turn.
        self.mart_queue.clear();
        match self.advance(DecisionKind::MartPurchase, TurnContext::None)? {
            // ⚠️ The quantity is **not** trimmed to the wallet here — `assert_pokemart_state` does
            // that against the ROM's own price table, because Gen 1 hands over *nothing* for an
            // order it cannot afford and the agent has been trimming since long before this policy.
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

    /// The next kind queued by this visit's `buy_item`. See `Policy::next_mart_purchase`'s ⚠️ for
    /// why this is a method of its own rather than a longer answer to `pick_mart_purchase`.
    ///
    /// ⚠️ **It touches neither `pending`, `waiting` nor the generation** — the same shape
    /// `advance_queue` and `pick_field_move` have, and for the same reason: a decision already taken
    /// is being handed over, not asked for. Starting a turn here would cancel nothing and buy a
    /// completion for an answer already in hand.
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
        self.queue = None;
        self.outcome = None;
        self.manual.clear();
        self.note = None;
        // The game is a different game now, so a battle half-written up is about nothing.
        self.battle_report = None;
        self.finishing = None;
        self.reports.clear();
        // A queued order belongs to a mart in a game that no longer exists.
        self.mart_queue.clear();
        self.last_battle_state = None;
        // ⚠️ **Disarmed here as well as in `Worker::apply_restart`, because the two happen at
        // different moments.** The worker only learns about a restart at the top of its next turn,
        // and a battle can start before then — so without this the first battles of the new game
        // are fought by the previous game's script. The worker re-arms from the *new* run's file
        // when it catches up, which is empty for a fresh run and correct for a resumed one.
        self.handles.live_script.arm(None);
    }

    /// Collected by the agent at the top of its next tick, ahead of the state machine.
    fn take_manual_input(&mut self) -> Vec<JoypadButton> {
        std::mem::take(&mut self.manual)
    }

    /// The narrative between decisions: dialogue, a battle starting, and above all the abort reasons
    /// that tell a model to stop re-picking a route that cannot be walked.
    fn on_event(&mut self, event: &AgentEvent) {
        // ⚠️ **This is the only place the policy learns how an action ended**, and the three events
        // below are the whole of it. `event()` is the funnel every agent event goes through — the
        // ones collected into `update`'s local buffer included — so a class of ending cannot be
        // missed here, only mis-classified; and an ending nothing names is read as
        // [`ActionOutcome`]'s `None` rather than as success, which is the safe way round.
        match event {
            AgentEvent::OverworldActionCompleted { .. }
            | AgentEvent::OverworldInteractionCompleted { .. } => {
                self.outcome = Some(ActionOutcome::Landed);
            }
            AgentEvent::OverworldActionAborted { reason, .. } => {
                self.outcome = Some(ActionOutcome::Stopped(*reason));
            }
            // ⚠️ **The cartridge's own words are the only account of a battle turn there is.**
            // `BattleActionStarted` is the *intent*, published the moment the policy commits, and
            // the enemy's action is never an event at all — so "It's super effective!",
            // "SPARKY fainted!" and "gained 56 EXP" reach the model through here or not at all.
            AgentEvent::TextBox { message } => {
                if let Some(report) = self.battle_report.as_mut() {
                    report.said(message);
                }
            }
            AgentEvent::BattleEnded => self.finishing = self.battle_report.take(),
            _ => {}
        }

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
    use crate::llm::protocol::{ChatRequest, Completion, Fragment, FunctionCall, Message, Role, ToolCall};
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
            on_delta: &mut dyn FnMut(Fragment<'_>),
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
            // Both channels, in the order a real endpoint sends them: a reasoning model thinks
            // before it speaks, and the worker publishes the two as different events.
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

    /// ⚠️ **A `summary` is added to every well-formed call that does not have one**, because
    /// `tools::classify` rejects a terminal call without one and every fixture below predates that
    /// rule. Writing it into each of the sixty-odd argument strings by hand would be the same edit
    /// sixty times, and would leave each of them testing the rule rather than what it is about.
    /// Arguments that do not parse are left exactly as written — several tests hand this deliberate
    /// rubbish and expect a complaint about it.
    ///
    /// The enforcement itself is tested where it lives:
    /// `llm::tools::tests::a_terminal_call_must_say_what_it_is_doing`.
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

    /// See [`calls`]. A no-op on anything that is not a JSON object, or that already says something.
    fn with_summary(arguments: &str) -> String {
        let Ok(serde_json::Value::Object(mut object)) = serde_json::from_str(arguments) else {
            return arguments.to_string();
        };
        if !object.contains_key("summary") {
            object.insert("summary".into(), serde_json::json!("a test's decision"));
        }
        serde_json::Value::Object(object).to_string()
    }

    /// A reply that says something *and* calls a tool. The prose is what makes it possible to write a
    /// turn of a chosen size, which is how the compaction test reaches its threshold without a
    /// hundred thousand tokens of fixture.
    fn saying_calls(text: &str, pairs: &[(&str, &str)]) -> Reply {
        let mut reply = calls(pairs);
        reply.completion.content = text.to_string();
        reply
    }

    /// A reply that thinks before it speaks, which is what every local reasoning model does.
    fn thinking(thought: &str, mut reply: Reply) -> Reply {
        reply.completion.reasoning = thought.to_string();
        reply
    }

    /// A reply the endpoint cut off at `GB_MAX_TOKENS` — prose, no tool call, `finish_reason:
    /// "length"`.
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
            Self::with_config_in(script, None, tweak)
        }

        /// The same rig, pointed at a run directory, so a test can drop it and build a second one on
        /// the same files — which is the only way to exercise a restart from outside the process.
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
                // Without a run directory the note tools work, they simply keep nothing (W6b), and
                // the conversation is forgotten at the end of the test exactly as it used to be.
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
            self.action_ids(1).remove(0)
        }

        /// The first `count` ids the overworld menu offers, in the order the turn offers them —
        /// which is the order a chain has to be written in for `not_on_the_menu` to accept it.
        fn action_ids(&mut self, count: usize) -> Vec<String> {
            let state = self.state();
            let menu = tools::overworld_menu(&state, None);
            assert!(menu.len() >= count, "Oak's lab offers {} actions, not {count}", menu.len());
            menu.into_iter().take(count).map(|item| item.id).collect()
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

    /// **W6b / §10 — the plan is in the history exactly once, and the prefix in front of it never
    /// moves.**
    ///
    /// ⚠️ **Both halves are the point, and they are what this replaced.** The list used to be
    /// rendered into the *system* message on every request, so a `todo_add` changed message 0 — and
    /// a prompt cache is keyed on the prefix, so one edit to the model's own plan threw away the
    /// cached prefill of the entire conversation. Here the system message must be byte-identical
    /// across every request of the run, and the plan must appear once rather than accumulating a
    /// stale copy per turn.
    #[test]
    fn the_plan_is_appended_and_never_disturbs_the_cacheable_prefix() {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        let choose = format!(r#"{{"id":"{id}"}}"#);
        rig.push(vec![
            // Turn 1 decides without touching the plan at all.
            calls(&[("choose_action", &choose)]),
            // Turn 2 adds to it *and* decides in one message — the "remember this, and go north"
            // shape the worker has to service rather than discard.
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

        // The prefix. Nothing the model did may have touched it.
        for (n, request) in requests.iter().enumerate() {
            assert_eq!(request.messages[0].role, Role::System);
            assert_eq!(request.messages[0], requests[0].messages[0],
                       "request {n}'s system message differs — the whole prefix cache is gone");
        }

        // ⚠️ **The newest copy is the plan; the older ones are left where they are.** Removing the
        // stale one would mean rewriting the middle of the history, and a prompt cache is keyed on
        // the prefix — so that is a couple of thousand uncached tokens every time the model touches
        // its own list, against a few hundred *cached* ones for leaving it. `render()` says in the
        // message itself that the last one wins, and compaction is what bounds how many there are.
        assert!(!plans(&requests[0]).last().expect("a plan").contains("Poke Flute"),
                "nothing was planned yet");
        assert!(plans(&requests[2]).last().expect("a plan").contains("Poke Flute"),
                "the item added mid-turn is in the next turn: {:?}", plans(&requests[2]));
        assert_eq!(plans(&requests[2]).len(), 2, "the empty opening plan is still there, untouched");

        // ⚠️ **And therefore the history is append-only, with no exceptions.** Every request must
        // carry the whole of the one before it as a literal prefix — including turn 3, which is the
        // turn the plan changed and the one that used to pay for it.
        for n in 1..requests.len() {
            let sent = &requests[n - 1].messages;
            assert_eq!(&requests[n].messages[..sent.len()], &sent[..],
                       "request {n} rewrote history request {} had already sent — the cache is gone",
                       n - 1);
        }

        // The page is told too — a viewer reads the plan as what the run is trying to do.
        let published: Vec<Vec<String>> = rig
            .drained_events()
            .into_iter()
            .filter_map(|event| match event {
                UiEventBody::Plan { items } => Some(items.into_iter().map(|item| item.text).collect()),
                _ => None,
            })
            .collect();
        // Two publishes across four turns: the opening one and the edit. ⚠️ **The opening one is
        // not noise** — a resumed run loads a plan off disk with no event to announce it, so
        // without a publish from the first turn the panel would stay empty until the model next
        // happened to touch its own list, which can be an hour.
        assert_eq!(published, [vec![], vec!["come back to Route 12 with the Poke Flute".to_string()]],
                   "published on change, not on a timer");
    }

    /// **An unchanged plan still comes back to the tail eventually.**
    ///
    /// Emit-on-change is what keeps the prefix cache intact, and it has one failure mode: a model
    /// that sets a plan once and never touches it leaves that message wherever it first landed, so
    /// the list it is meant to be revising ends up the *least* recent thing in the request. Both
    /// deployed runs did exactly that — 258 turns with one `todo_set`, and 2430 turns with sixteen —
    /// and neither ever revised an item. `PLAN_REFRESH_TURNS` bounds how far back it can drift.
    ///
    /// ⚠️ **Both halves are asserted.** A refresh that happened every turn would pass a test that
    /// only checked the plan comes back, and would cost a turn's re-prefill on every request — which
    /// is the thing emit-on-change exists to avoid. So the turns in between must still append.
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
        // Request 1 is the first to carry the edit, so it is where the drift is measured from, and
        // the window runs from there.
        let planted = plan_at(&requests[1]);
        let due = REFRESH + 2;
        assert_eq!(plan_at(&requests[due - 1]), planted,
                   "the plan moved before it was due — every turn in between pays a re-prefill");
        for request in 2..due {
            let sent = &requests[request - 1].messages;
            assert_eq!(&requests[request].messages[..sent.len()], &sent[..],
                       "request {request} rewrote history the one before had already sent");
        }
        // …and then it comes back, once, to sit immediately before the situation it belongs to —
        // ⚠️ **as a second copy, with the buried one left exactly where it was.** Lifting it would
        // mean rewriting the middle of the history, which is the one thing the prefix cache cannot
        // survive; a stale copy a few hundred cached tokens back is the cheaper half of that trade by
        // about ten to one, and compaction is what stops them piling up.
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

        // ⚠️ **Every turn that does not carry the plan says so.** The copy is still in the history,
        // but tens of turns back in a conversation that is mostly menus — which is a message the
        // model can see and is not reading. The line costs nothing at the cache: the situation is
        // fresh tokens every turn either way.
        for (n, request) in requests.iter().enumerate() {
            let asked = last_user_message(request);
            let carried = plan_at(request) == request.messages.len() - 2;
            assert_eq!(!carried, asked.contains(crate::llm::prompt::PLAN_UNCHANGED),
                       "request {n} carried={carried} but the note says otherwise: {asked}");
        }
    }

    /// **The periodic refresh is an overworld thing, and an edit is not.**
    ///
    /// A refresh buys the model a fresh look at a list it has not touched, and there is nothing to
    /// be done about that list in the middle of a battle — one question, one answer, and the
    /// re-prefill it costs is bought for nothing. An *edit* has to land on any kind, because the todo
    /// tools are offered on all of them (`non_terminal_names` chains them unconditionally), so a plan
    /// changed during a battle that stayed uncorrected would have the next overworld turn read a
    /// stale one.
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
        // Request 1 is the first battle turn and *does* move the plan — the overworld turn before it
        // called `todo_set`, and an edit lands on any kind. That is the case this is distinguishing
        // itself from, so it is asserted rather than skipped past.
        let planted = plan_at(&requests[1]);
        assert_eq!(planted, requests[1].messages.len() - 2, "the edit is carried on the next turn");
        assert!(!last_user_message(&requests[1]).contains(crate::llm::prompt::PLAN_UNCHANGED));

        // Everything after it is a battle turn with nothing to say about the plan, so none of them
        // may move it however long the window has been up.
        for (n, request) in requests.iter().enumerate().skip(2) {
            assert_eq!(plan_at(request), planted,
                       "request {n} is a battle turn and repositioned the plan anyway");
            assert!(last_user_message(request).contains(crate::llm::prompt::PLAN_UNCHANGED),
                    "but it must still be told the plan is back there: request {n}");
        }
        assert!(requests.len() > REFRESH + 2, "the window has to have fallen due for this to mean anything");
    }

    /// **A compaction can take the plan, and the next turn puts it back.**
    ///
    /// `is_turn_start` refuses to cut *between* a plan and its turn, so the plan is only ever dropped
    /// along with the turn it belongs to — but it can be dropped, and after a summarising compaction
    /// the history is the system prompt, the summary and a short tail. What stops that clobbering
    /// the chain is that `sync_plan` runs at the top of the next turn, finds no copy, and appends
    /// one; the plan itself is re-rendered from `todo.json`, which is the authority and cannot go
    /// stale.
    ///
    /// ⚠️ **Which is also why the summary does not carry a copy of the plan.** A summary is written
    /// once and never rewritten, so a plan quoted inside one is frozen at the moment of the
    /// compaction and sits at message 1 contradicting the live copy for the rest of the run.
    #[test]
    fn a_compaction_that_drops_the_plan_does_not_break_the_chain() {
        use crate::llm::compaction;
        let mut todo = crate::llm::todo::TodoList::open(None);
        todo.apply(crate::llm::todo::TodoCall::Set { id: None, text: Some("deliver the parcel".into()) });
        let plan = crate::llm::prompt::plan_message(&todo);

        // A history shaped the way the worker leaves one: system, then turns, with the plan sitting
        // immediately in front of the situation of the turn it was last emitted for.
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
                "this history is long enough that the plan is inside the dropped middle — otherwise                  the test proves nothing");

        // The repair is `sync_plan`'s "there is no copy" arm, which is what the next turn runs.
        assert!(messages.iter().position(|m| crate::llm::prompt::is_plan(m)).is_none());
        messages.push(crate::llm::prompt::plan_message(&todo));
        let restored = messages.last().expect("a plan was appended");
        assert!(crate::llm::prompt::is_plan(restored));
        assert!(restored.text().expect("prose").contains("deliver the parcel"),
                "and it is re-rendered from the list on disk, so it cannot come back stale");
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

    /// A reasoning model's thinking reaches the page as its own kind of event, and **never reaches
    /// the endpoint again**.
    ///
    /// ⚠️ Both halves matter and they pull in opposite directions. Publishing it is the whole point —
    /// without it the local models stream three quarters of their output into a field nothing read,
    /// and the page showed a blank turn for however long the model thought. Sending it back is the
    /// mistake that would make the fix expensive: reasoning is billed once as completion tokens, and
    /// a copy in the history pays for it again on every turn for the rest of the run.
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

    /// **The one sentence about a turn that outlives it**, and the other half of the test above.
    ///
    /// ⚠️ Thinking is never sent back and most models write no `content` beside a tool call, so
    /// without this the assistant side of the history is a column of bare JSON — every turn saying
    /// what it did and none saying why, which is the state a model walks into the same building four
    /// times from. It rides on the terminal call's own arguments, so it lands in the history by
    /// itself: `Message::assistant` carries `tool_calls` verbatim.
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

        // The second turn's request carries the first turn's reason, because the assistant message
        // holding that tool call is still in the history.
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

    /// A reply cut off by `GB_MAX_TOKENS` is nudged differently from one that simply said nothing.
    ///
    /// ⚠️ **The distinction is not cosmetic.** Told only "that reply contained no tool call", a model
    /// that was cut off mid-thought concludes it forgot to call one and tries again at the same
    /// length — into the same ceiling, for as many attempts as it is given. What it has to be told is
    /// that the *thinking* ran out of room.
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

    /// §7.4's ⚠️: an id that does not resolve is a message back to the model, never a panic and
    /// never a silent no-op. What changed is *where* the message is made.
    ///
    /// ⚠️ **An id the turn never offered is refused inside the turn, so it costs one completion
    /// rather than the whole turn.** It used to be accepted here, published as a `Decision`, sent to
    /// the policy and refused by `resolve_overworld` — after which the complaint could only be
    /// carried on the *next* turn's situation, which is a second full prefill. The deployed run paid
    /// that 59 times in 934 `choose_action` decisions, every one of them an id whose map was a map
    /// the player had already left. `tools::not_on_the_menu` is the check and
    /// `an_id_the_turn_never_offered_is_refused_before_it_costs_the_turn` pins its wording.
    ///
    /// The rejection is a `tool` message, so the model answers it in the same conversation: this
    /// asserts the turn goes on to make a real decision rather than ending on the complaint.
    #[test]
    fn an_id_the_turn_never_offered_is_refused_without_ending_the_turn() {
        let (mut rig, mut policy) = Rig::new(vec![
            calls(&[("choose_action", r#"{"id":"PalletTown:99,99:Warp","summary":"out of the lab"}"#)]),
            calls(&[("wait", r#"{"ticks":1,"summary":"waiting"}"#)]),
        ]);

        rig.pump_overworld_for(&mut policy, Duration::from_secs(2));

        let requests = rig.requests();
        assert!(requests.len() >= 2, "the turn carries on after the id is refused");
        // ⚠️ The complaint is a `tool` result inside the same turn, not the next turn's situation.
        // That distinction is the whole saving, so it is what is asserted.
        let answer = requests[1]
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::Tool)
            .and_then(Message::text)
            .expect("the refused call is answered like any other tool call");
        assert!(answer.contains("PalletTown:99,99:Warp"), "the model is told which id failed: {answer}");
        assert!(answer.contains("not one of this turn's actions"), "{answer}");
        // The fixture is Oak's lab, so the id names a map the player is not on — the mistake the
        // deployed run made 59 times, and the one the complaint has to name rather than blaming the
        // world for having moved.
        assert!(answer.contains("OaksLab"), "it must say where the player actually is: {answer}");
        for request in &requests {
            history_is_well_formed(request);
        }
    }

    // ── Chained actions ──────────────────────────────────────────────────────────────────────────

    /// The whole point of `then`: the second action costs no request at all.
    ///
    /// ⚠️ **What is asserted is the request count, because that is the only thing the feature buys.**
    /// A chain that were re-decided by the model between its steps would pass every assertion about
    /// *which* actions came out and still be worth nothing.
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

        // …and once the chain is spent the model is asked again, as it would be for any decision.
        policy.on_event(&AgentEvent::OverworldActionCompleted { destination: second.tile });
        assert!(rig.pump_overworld_for(&mut policy, Duration::from_millis(300)).is_none());
        rig.wait_for_requests(2, Duration::from_secs(2));
        assert_eq!(rig.requests().len(), 2, "the end of a chain is an ordinary decision point");
    }

    /// A chain is a sequence of independent decisions, not a route the agent commits to: anything
    /// that stops one stops the rest, and the model is told where it got to.
    ///
    /// ⚠️ **The conservative rule is the load-bearing one.** Carrying on past a text box would walk
    /// a chain straight through the guards, locked doors and errands that are how this game says
    /// anything — the same loop that had one deployed run abort on the same square 143 times.
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

        // ⚠️ One tick, not a pump: the drop and the fresh turn both happen inside it, and polling on
        // would let the `wait` this turn answers with expire and buy a *third* turn.
        assert!(rig.tick_overworld(&mut policy).is_none(), "the chain is dropped rather than advanced");
        rig.wait_for_requests(2, Duration::from_secs(2));
        let requests = rig.requests();
        assert_eq!(requests.len(), 2, "a stopped chain hands the decision back");
        let situation = last_user_message(&requests[1]);
        assert!(situation.contains("were not tried"), "the model is told the rest was dropped: {situation}");
        assert!(situation.contains(&ids[0]), "and which action was stopped: {situation}");
    }

    /// **The other half of the same call.** A wild Pokémon interrupting a walk says nothing about
    /// the walk, so `resume_after_battle` takes it up again — and without the flag the decision comes
    /// back to the model, which is what every run before this did.
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

    /// ⚠️ **An ending nothing named is not an ending that went well.** Grass and cave pacing and the
    /// Surf mount all leave `OverworldMovement` for a driver of their own and report no outcome at
    /// all, on purpose — each is handing the decision back. Reading that silence as success would
    /// carry a chain on past a step that never happened.
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

    /// **The escape hatch is closed on a turn that has a menu**, end to end: the presses never reach
    /// the agent, the model is told where the answer actually is, and the turn carries on to a real
    /// decision rather than being thrown away.
    ///
    /// ⚠️ **This is the whole point of the change and it is worth stating plainly.** The deployed run
    /// spent 91 consecutive turns pressing buttons at a ledge on Route 3 while the connection into
    /// Pewter City sat in its action menu on every one of them; 738 of its 749 presses were on
    /// overworld turns with a perfectly good menu. Neither prose in the tool description nor a
    /// required `why` moved that number — 72% of the presses left `why` null — so the tool is no
    /// longer in the catalogue here at all.
    ///
    /// The press half of the contract still holds where it belongs; see
    /// [`a_stuck_turn_may_read_first_and_its_press_reaches_the_agent`].
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

        // ⚠️ The refusal has to name both halves — the menu, *and* the way to say the menu is wrong.
        // Told only "use the menu", a model that genuinely cannot find its action has nowhere to go.
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

    /// ⚠️ **One mart visit, several kinds, and the queue must not outlive the visit.** Balls *and*
    /// Potions was two mart turns and the two overworld turns that reach them, on the errand the
    /// prompt tells the model to run at every mart it passes. The tail is handed over by
    /// `next_mart_purchase`, a method of its own so the scripted policies keep quitting exactly
    /// where they always did — see its ⚠️ for why re-asking `pick_mart_purchase` would double-buy.
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

    /// ⚠️ **A queued order must never be spendable at the *next* mart.** There is no shop-closed
    /// callback to drain it on, so `pick_mart_purchase` — the one call that means "the model is
    /// being asked afresh" — clears it. Without this a chain abandoned halfway (a battle, a reset,
    /// the model walking out) would buy its tail the next time the player talked to any clerk.
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

    /// A slot the Pokémon does not have would send the menu cursor somewhere it can never arrive, so
    /// it is declined — and the model is told why rather than left watching a prompt that never
    /// closes.
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
        // ⚠️ **8 000 rather than the 6 000 this was written at, and the change is a fixture rather
        // than a finding.** What a turn costs before it says anything is the system prompt and the
        // tools array, neither of which a compaction touches, and both have grown — `read_guide`, a
        // second prose section, `choose_action`'s chain. Three ordinary turns had crept over 0.85 of
        // 6 000 on their own, so the compaction fired a turn early and **ate the prose reply meant
        // for turn 4 as its summary**: `after` (8 830) came back larger than `before` (5 198), which
        // is what this test's own assertion caught. The sizing rule is unchanged — three plain turns
        // under the threshold, the prose turn over it — and the number that expresses it moved.
        let (mut rig, mut policy) = Rig::with_config(vec![], |config| config.context_limit = 8_000);
        let id = rig.first_action_id();
        let choose = format!(r#"{{"id":"{id}"}}"#);
        rig.push(vec![
            calls(&[("choose_action", &choose)]),
            calls(&[("choose_action", &choose)]),
            calls(&[("choose_action", &choose)]),
            // ~4 900 tokens of prose in one turn, which is what puts it over `compact_above` (0.85)
            // of the window above. ⚠️ Sized against the *threshold*, so it moves when the default
            // does — a turn that lands just under it makes this test pass by never compacting at
            // all, and one that lands over it a turn early makes it compact the wrong thing.
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

    /// Half (A)'s headline, through the real worker: a second process opens its first request on the
    /// conversation the first one left behind, rather than on a bare system prompt.
    #[test]
    fn a_process_that_restarts_mid_run_opens_its_next_request_on_the_conversation_it_had() {
        let scratch = crate::run::tests::Scratch::new("llm-restart");
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

        // The precondition: the first process really did build a conversation worth restoring, and
        // really did write it down. Without this the assertions below pass on an empty file.
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
        // ⚠️ The invariant the endpoint enforces with a 400. A restored history that fails this is
        // the failure mode that lasts for the rest of the run rather than for one request.
        history_is_well_formed(last);

        drop(policy);
        drop(rig);
    }

    /// Half (B) through the real worker: after a compaction the conversation it replaced is gone from
    /// the request and still on disk.
    #[test]
    fn a_run_that_compacts_still_has_the_conversation_the_compaction_replaced_on_disk() {
        let scratch = crate::run::tests::Scratch::new("llm-compactlog");
        let (mut rig, mut policy) =
            Rig::with_config_in(vec![], Some(&scratch.0), |config| config.context_limit = 8_000);
        let id = rig.first_action_id();
        let choose = format!(r#"{{"id":"{id}"}}"#);
        // ⚠️ **The marker has to be in an *early* turn, not the one that fills the window.** What a
        // summary keeps is the tail, so the turn whose prose triggered the compaction is precisely
        // the one still in the request afterwards — asserting on that would fail for a reason that
        // has nothing to do with the log.
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
        // ⚠️ **The precondition is half the test.** A run that never compacted would pass the "it is
        // still in the log" assertion trivially, because it would still be in the live history too.
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

    /// ⚠️ **A turn in flight when `POST /api/new-run` lands belongs to the *old* game.** Its
    /// conversation has to stay with the run that had it: filed in the new run's directory instead,
    /// the new run would resume into a conversation about a game that no longer exists.
    #[test]
    fn the_conversation_a_new_run_leaves_behind_stays_with_the_run_that_had_it() {
        let old = crate::run::tests::Scratch::new("llm-oldrun");
        let new = crate::run::tests::Scratch::new("llm-newrun");
        let said = "I am about to be replaced by a brand new game.";

        let (mut rig, mut policy) = Rig::with_config_in(vec![], Some(&old.0), |_| {});
        let id = rig.first_action_id();
        let choose = format!(r#"{{"id":"{id}"}}"#);
        rig.push(vec![
            saying_calls(said, &[("choose_action", &choose)]),
            calls(&[("choose_action", &choose)]),
        ]);
        rig.pump_overworld(&mut policy).expect("the old game's turn lands");

        // The precondition: the old run really does have a conversation to misfile.
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

    /// A script that reaches an action on every one of `battle_script`'s validation scenarios *and*
    /// on the committed battle fixture. `best_move` is `()` only when nothing can damage the foe.
    const SCRIPT: &str = "if battle.best_move != () { battle.fight(battle.best_move); }\n\
                          if battle.can_run { battle.run(); }\n\
                          battle.ask();";

    /// A rig whose first overworld turn installs `source` and then walks somewhere, plus however
    /// many further replies the test needs. The id has to be read off the live menu, so the rig is
    /// built empty and the replies queued afterwards.
    fn armed_with(source: &str, then: usize) -> (Rig, LlmPolicy) {
        let (mut rig, mut policy) = Rig::new(vec![]);
        let id = rig.first_action_id();
        let walk = format!(r#"{{"id":"{id}"}}"#);
        {
            let mut replies = rig.endpoint.replies.lock().unwrap();
            replies.push_back(calls(&[
                ("set_battle_script", &serde_json::json!({ "script": source }).to_string()),
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
            .events
            .try_iter()
            .filter_map(|event| match event.body {
                UiEventBody::ToolResult { name, content, .. } if name == "set_battle_script" => Some(content),
                _ => None,
            })
            .collect();
        assert_eq!(results.len(), 1, "one script was installed");
        assert!(results[0].starts_with("ok"), "and it armed: {}", results[0]);
    }

    /// **The whole feature, in one number.** A battle is fought from beginning to end and the
    /// endpoint is never called: the request count after the battle is the request count before it.
    ///
    /// ⚠️ **This is the assertion that matters, not that an action came back.** A script that
    /// worked but still started a turn would pass every other test here and buy nothing at all —
    /// the saving is the request, and a battle is five to thirty of them against a history that by
    /// then is tens of thousands of tokens.
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

    /// The other half: the model is told what happened, once, on its next turn.
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

        // ⚠️ And exactly once. A report left queued would be re-rendered into every turn after it.
        assert_eq!(situation.matches("### Battle report").count(), 1, "{situation}");
    }

    /// ⚠️ **The report replaces the raw event stream rather than sitting beside it.** Every message
    /// box in the battle also went into `events`, and without `events_mark` the same prose would be
    /// in the same request twice, in two shapes.
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

    /// One strike. The failing turn comes straight back to the model with the reason, and every
    /// battle turn after it does too.
    #[test]
    fn a_script_that_fails_disarms_and_hands_the_turn_back() {
        // ⚠️ **Broken on the second turn, and validated clean — which is the honest shape of this
        // failure.** Every validation scenario is turn 1, so a script whose behaviour depends on
        // the turn number is exactly what validation cannot catch, and exactly what the disarm is
        // for. See `battle_script::SCENARIOS`.
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

        // ⚠️ And it stays disarmed: the next battle turn is the model's too, not a second failure.
        assert!(policy.handles.live_script.source().is_none(), "one strike disarms for the run");
    }

    /// ⚠️ **A reset disarms immediately, not at the worker's next turn.** `POST /api/new-run`
    /// checkpoints the old run and starts a fresh game on the emulator thread, and a battle can
    /// begin before the worker has looked at the restart cell — so the cell is cleared here too, or
    /// the new game's first battles are fought by the previous game's script.
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

    /// `battle.ask()` is the granular half of the feature: the script keeps deciding, and hands
    /// back only the turns it says are worth paying for.
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
        assert!(policy.handles.live_script.source().is_some(), "and the script is still armed");
    }
}
