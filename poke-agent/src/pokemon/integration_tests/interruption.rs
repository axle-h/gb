//! Turns abandoned because the game asked a different question while one was in flight.
//!
//! `LlmPolicy` keys a turn by the decision kind it answers, and a poll for a *different* kind
//! cancels it (see that module's header). A cancelled turn has already been paid for, so the shape
//! everyone expects — an overworld turn interrupted by a battle — is worth knowing the rate of
//! before a run goes on a metered endpoint.
//!
//! **Measured, it is zero**, and the reason is structural: **the agent presses nothing while a turn
//! is in flight**, so the game sits at a static menu or a stationary tile and cannot move on its
//! own. A wild encounter or a trainer's line of sight fires during a *walk*, and no policy poll
//! happens during a walk, so the battle is the next turn rather than an interruption of the one in
//! flight. What is kept here is the guard on that: leaving Oak's lab trips the rival's challenge,
//! which is the longest scripted freeze the early game has, and the agent must ask nothing inside
//! it.
//!
//! Nothing else in the suite can see this: every scripted policy answers within the tick it is
//! asked, so there is no window for the question to change in. [`SlowPolicy`] is that window — it
//! wraps any ordinary policy and holds each answer back for a number of agent ticks, which is the
//! one property of an LLM turn that matters here.
//!
//! ⚠️ **The window is the whole instrument, so `SlowPolicy` has to key turns exactly the way
//! `LlmPolicy` does** — `pick_field_move`'s exemption included. A decorator that merely delayed
//! answers without keying them would report a cancellation every time the agent asked anything at
//! all, which is not the thing being counted.
//!
//! ⚠️ **Both results here are negative, so the instrument is tested too**
//! (`the_detector_notices_a_question_being_replaced`). A detector that cannot fire proves nothing by
//! not firing, and this one is only ever asked to not fire.

use super::*;
use std::sync::{Arc, Mutex};

use crate::pokemon::actions::OverworldAction;
use crate::pokemon::bag::BagItem;
use crate::pokemon::battle::BattleAction;
use crate::pokemon::move_name::{PokemonMove, PokemonMoveName};
use crate::pokemon::policy::{FieldMove, Policy};
use crate::pokemon::species::PokemonSpecies;
use crate::pokemon::world_graph::WorldGraph;

const OAKS_LAB: &[u8] = include_bytes!("../data/oaks-lab-just-got-squirtle.bin");

/// The question the agent is asking, keyed the way `llm::tools::DecisionKind` keys it.
///
/// ⚠️ Deliberately a local enum rather than `DecisionKind` itself: this module is in the default
/// tier and must build with `--no-default-features`, and the point of the exercise is that the
/// cancellation is a property of the **agent's** polling, not of anything in the `llm` layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Question {
    Overworld,
    Battle,
    Nickname,
    Mart,
    ForgetMove,
}

/// One turn that was thrown away, and everything needed to say why.
#[derive(Debug, Clone)]
struct Abandoned {
    /// The agent tick the abandoned turn started on.
    started_on: u32,
    asked: Question,
    replaced_by: Question,
    /// How far into its latency the turn got before the question changed, in agent ticks. This is
    /// what a debounce would have to be longer than to have prevented it.
    after_ticks: u32,
    /// Where the player was standing when the abandoned turn started.
    at: String,
}

#[derive(Debug, Default)]
struct Log {
    /// Every turn started, in order: the agent tick it started on, and where the player was.
    started: Vec<(u32, Question, String)>,
    abandoned: Vec<Abandoned>,
    /// Advanced by the harness once per [`TestFixture::step`], so everything recorded here can be
    /// lined up against the agent's own events on the same clock.
    tick: u32,
}

impl Log {
    fn has_asked(&self, question: Question) -> bool {
        self.started.iter().any(|(_, asked, _)| *asked == question)
    }
}

struct Pending {
    question: Question,
    /// Ticks still owed before this turn answers.
    remaining: u32,
    at: String,
    started_on: u32,
}

/// An ordinary policy with an LLM's latency bolted on.
struct SlowPolicy {
    inner: Box<dyn Policy>,
    /// Agent ticks of 20 ms one turn takes to answer.
    latency: u32,
    pending: Option<Pending>,
    log: Arc<Mutex<Log>>,
}

impl SlowPolicy {
    fn new(inner: Box<dyn Policy>, latency: u32) -> (Self, Arc<Mutex<Log>>) {
        let log = Arc::new(Mutex::new(Log::default()));
        (Self { inner, latency, pending: None, log: Arc::clone(&log) }, log)
    }

    /// `true` when the wrapped policy should be asked *now*; `false` is "still thinking".
    ///
    /// This is `LlmPolicy::advance` with the round trip replaced by a countdown, and the two rules
    /// it exists to enforce are the same ones:
    ///
    /// - a turn in flight is only advanced by a poll for **the same question**, and
    /// - a poll for a different question **cancels** it, because a battle decision must never be
    ///   applied to an overworld state.
    fn advance(&mut self, question: Question, at: String) -> bool {
        match self.pending.take() {
            Some(pending) if pending.question == question => {
                if pending.remaining > 0 {
                    self.pending = Some(Pending { remaining: pending.remaining - 1, ..pending });
                    return false;
                }
                true
            }
            // The agent is asking something else, so whatever was in flight is now answering a dead
            // question and is dropped. This is the event the whole module exists to count.
            Some(pending) => {
                self.log.lock().expect("the log is never poisoned").abandoned.push(Abandoned {
                    started_on: pending.started_on,
                    asked: pending.question,
                    replaced_by: question,
                    after_ticks: self.latency - pending.remaining,
                    at: pending.at,
                });
                self.open(question, at)
            }
            None => self.open(question, at),
        }
    }

    /// Put a turn in flight. Always `false`: the poll that opens a turn is the one poll that spends
    /// none of its budget, exactly as `LlmPolicy::advance` sends the request and answers `None` on
    /// the tick it is first asked.
    fn open(&mut self, question: Question, at: String) -> bool {
        let mut log = self.log.lock().expect("the log is never poisoned");
        let started_on = log.tick;
        log.started.push((started_on, question, at.clone()));
        drop(log);
        self.pending = Some(Pending { question, remaining: self.latency, at, started_on });
        false
    }
}

/// Where the player is, in the words a turn headline uses.
fn describe(state: &GameState) -> String {
    format!("{} ({}, {})", state.map.map, state.map.player_position.x, state.map.player_position.y)
}

impl Policy for SlowPolicy {
    fn name(&self) -> &'static str { "scripted" }

    fn pick_overworld_action(&mut self, state: &GameState, graph: &WorldGraph) -> Option<OverworldAction> {
        match self.advance(Question::Overworld, describe(state)) {
            true => self.inner.pick_overworld_action(state, graph),
            false => None,
        }
    }

    fn pick_battle_action(&mut self, state: &GameState) -> Option<BattleAction> {
        match self.advance(Question::Battle, describe(state)) {
            true => self.inner.pick_battle_action(state),
            false => None,
        }
    }

    fn pick_nickname(&mut self, species: PokemonSpecies) -> Option<Option<String>> {
        // ⚠️ The three menu prompts are handed no `GameState`, exactly as `LlmPolicy` is not, so the
        // square recorded is the one the last state-carrying poll saw.
        match self.advance(Question::Nickname, "a naming screen".to_string()) {
            true => self.inner.pick_nickname(species),
            false => None,
        }
    }

    fn pick_mart_purchase(&mut self, state: &GameState) -> Option<Option<BagItem>> {
        match self.advance(Question::Mart, describe(state)) {
            true => self.inner.pick_mart_purchase(state),
            false => None,
        }
    }

    fn pick_move_to_forget(&mut self, slot: usize, current: &[PokemonMove], new_move: PokemonMoveName)
        -> Option<Option<usize>>
    {
        match self.advance(Question::ForgetMove, "a forget-move prompt".to_string()) {
            true => self.inner.pick_move_to_forget(slot, current, new_move),
            false => None,
        }
    }

    /// ⚠️ **Not a decision point**, for the reason `LlmPolicy::pick_field_move` is not one: it runs
    /// on every idle overworld tick immediately before `pick_overworld_action`, so keying it would
    /// cancel the overworld turn fifty times a second and nothing would ever be answered.
    fn pick_field_move(&mut self, state: &GameState) -> Option<FieldMove> {
        self.inner.pick_field_move(state)
    }

    fn on_event(&mut self, event: &AgentEvent) { self.inner.on_event(event) }
    fn steps_remaining(&self) -> Option<usize> { self.inner.steps_remaining() }
    fn current_step_is_long_running(&self) -> bool { self.inner.current_step_is_long_running() }
    fn is_exhausted(&self) -> bool { self.inner.is_exhausted() }
}

/// What one latency setting did on the way out of Oak's lab.
struct Run {
    latency: u32,
    log: Arc<Mutex<Log>>,
    /// The events the agent emitted, as prose, so a failure says what the run actually did.
    story: Vec<String>,
    reached_battle: bool,
    ticks: u32,
}

/// Walk out of Oak's lab with the starter, holding every answer back by `latency` agent ticks, and
/// stop once the rival's battle has put its first question to the policy.
///
/// ⚠️ **It stops at the first battle turn rather than fighting.** The interruption under test is the
/// transition; what follows is a fight the RNG decides, and a test that had to win one would be
/// measuring that instead. ⚠️ **And it must not stop on `BattleStarted`**, which is several seconds
/// of intro text before the menu: the cancellation, if there is one, is recorded by the poll that
/// *replaces* the overworld turn, and that is the first battle poll.
fn walk_out_of_the_lab(latency: u32) -> Run {
    // ⚠️ The policy's own seed is deliberately fixed. It picks between *choices*, and this route
    // offers none — one queued step, one warp — so sweeping it re-runs an identical trace and buys
    // nothing. What varies the race here is the latency, which is what moves the answer relative to
    // the script.
    let scripted = DeterministicPolicy::new(42, vec![PolicyStep::goto(Map::PalletTown)]);
    let (policy, log) = SlowPolicy::new(Box::new(scripted), latency);
    // Enough game time for the walk plus a handful of turns at this latency: a turn costs
    // `latency` ticks of 20 ms, and getting out of the lab takes a few of them.
    let budget = Duration::from_secs(90 + (latency as u64 * 20 * 8) / 1000);
    let mut fixture = TestFixture::with_policy(OAKS_LAB, budget, Box::new(policy));

    let mut story = Vec::new();
    let mut reached_battle = false;
    let mut tick = 0u32;
    while fixture.total_cycles < fixture.max_cycles {
        tick += 1;
        log.lock().expect("the log is never poisoned").tick = tick;
        fixture.step();
        for event in fixture.agent.drain_events() {
            if matches!(event, AgentEvent::BattleStarted) {
                reached_battle = true;
            }
            story.push(format!("{tick}: {event}"));
        }
        if log.lock().expect("the log is never poisoned").has_asked(Question::Battle) {
            break;
        }
    }
    Run { latency, log, story, reached_battle, ticks: tick }
}

/// **The question this module was written to answer.** Leaving Oak's lab with a starter trips the
/// rival's challenge: the script freezes the player, walks GARY over and starts a battle. If the
/// agent reached a decision point anywhere inside that window it would ask an *overworld* question
/// that the game is about to make unanswerable, and the turn would be thrown away.
///
/// It does not, and the trace says why: the walk to the door is aborted into `RunningScript` and the
/// agent asks nothing at all for the ~900 ticks (18 s of game time) between the abort and the battle
/// menu. Three latencies rather than one because the window is a race — the script fires on a *tile*
/// and the answer lands on a *clock*, so an instant answer, a typical one and a slow one put the
/// walk in three different places relative to it.
///
/// ⚠️ **The precondition is half the test.** A run that never reaches the battle question stopped on
/// its cycle budget and proves nothing; the first version of this passed at every latency below 600
/// ticks by stopping before the rival ever got to ask anything.
#[test]
fn leaving_oaks_lab_does_not_strand_a_turn_in_the_rivals_script() {
    // 20 ms per tick: an instant answer, five seconds, and a minute. The deployed run's turn latency
    // spanned that range (median 12 s, p90 59 s).
    const LATENCIES: [u32; 3] = [1, 250, 3000];

    let mut findings: Vec<String> = Vec::new();
    for latency in LATENCIES {
        let run = walk_out_of_the_lab(latency);
        let log = run.log.lock().expect("the log is never poisoned");
        println!(
            "[latency {:>4} ticks ({:>5} ms)] {} ticks, {} turns, {} abandoned, battle {}",
            run.latency, run.latency * 20, run.ticks, log.started.len(), log.abandoned.len(),
            if run.reached_battle { "started" } else { "NOT started" },
        );
        for (tick, question, at) in &log.started {
            println!("    tick {tick:>5}: {question:?} turn, at {at}");
        }
        for line in &run.story {
            println!("      {line}");
        }
        for abandoned in &log.abandoned {
            findings.push(format!(
                "latency {} ticks: the {:?} turn started on tick {} at {} was abandoned after {} \
                 ticks ({} ms) for a {:?} turn",
                run.latency, abandoned.asked, abandoned.started_on, abandoned.at,
                abandoned.after_ticks, abandoned.after_ticks * 20, abandoned.replaced_by,
            ));
        }
        assert!(log.has_asked(Question::Battle),
                "latency {} ticks: the rival never put a battle question, so this run proves \
                 nothing. Story: {:?}", run.latency, run.story);
    }

    assert!(findings.is_empty(),
            "a turn was stranded by the rival's script:\n  {}", findings.join("\n  "));
}

/// ⚠️ **The detector has to be shown firing, or every negative result above means nothing.**
///
/// `SlowPolicy` is the whole instrument, and its one job is to notice a question being replaced
/// before it is answered. Nothing in the game produces that (which is the finding), so it is
/// produced by hand here: an overworld poll opens a turn, and a battle poll arrives before the
/// latency has run out.
#[test]
fn the_detector_notices_a_question_being_replaced() {
    struct Silent;
    impl Policy for Silent {
        fn name(&self) -> &'static str { "scripted" }
        fn pick_overworld_action(&mut self, _: &GameState, _: &WorldGraph) -> Option<OverworldAction> { None }
        fn pick_battle_action(&mut self, _: &GameState) -> Option<BattleAction> { None }
    }

    let (mut policy, log) = SlowPolicy::new(Box::new(Silent), 10);

    // Tick 1 opens an overworld turn; ticks 2 and 3 advance it without answering it.
    for tick in 1..=3 {
        log.lock().expect("the log is never poisoned").tick = tick;
        assert!(!policy.advance(Question::Overworld, "OaksLab (5, 6)".to_string()),
                "a turn with 10 ticks of latency cannot answer on tick {tick}");
    }
    assert!(log.lock().expect("the log is never poisoned").abandoned.is_empty(),
            "polls of the same question advance a turn, they do not replace it");

    // Tick 4 asks something else, which is the event under test.
    log.lock().expect("the log is never poisoned").tick = 4;
    policy.advance(Question::Battle, "OaksLab (5, 6)".to_string());

    let log = log.lock().expect("the log is never poisoned");
    assert_eq!(log.abandoned.len(), 1, "the replaced overworld turn is one abandoned turn");
    let abandoned = &log.abandoned[0];
    assert_eq!(abandoned.asked, Question::Overworld);
    assert_eq!(abandoned.replaced_by, Question::Battle);
    assert_eq!(abandoned.started_on, 1, "it started on the tick that opened it");
    // Two, not three: the first poll *opens* the turn and spends none of its budget, exactly as
    // `LlmPolicy::advance` sends the request and returns `None` on the tick it is first asked.
    assert_eq!(abandoned.after_ticks, 2, "two polls advanced it before the question changed");
    assert_eq!(log.started.len(), 2, "the replacement is a turn of its own");
}
