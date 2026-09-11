//! Turns abandoned because the game asked a different question while one was in flight.

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
    /// How far into its latency the turn got before the question changed, in ticks.
    after_ticks: u32,
    /// Where the player was standing when the abandoned turn started.
    at: String,
}

#[derive(Debug, Default)]
struct Log {
    /// Every turn started, in order: the agent tick it started on, and where the player was.
    started: Vec<(u32, Question, String)>,
    abandoned: Vec<Abandoned>,
    /// Advanced by the harness once per [`TestFixture::step`], on the agent's own clock.
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
    /// Agent ticks one turn takes to answer.
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
    fn advance(&mut self, question: Question, at: String) -> bool {
        match self.pending.take() {
            Some(pending) if pending.question == question => {
                if pending.remaining > 0 {
                    self.pending = Some(Pending { remaining: pending.remaining - 1, ..pending });
                    return false;
                }
                true
            }
            // The agent is asking something else, so the turn in flight is dropped.
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

    /// Put a turn in flight.
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
        // The menu prompts get no `GameState`, as `LlmPolicy` does not, so the square is the last
        // poll's.
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

    /// Not a decision point, as in `LlmPolicy`: it runs before every idle `pick_overworld_action`,
    /// so keying it would cancel every overworld turn.
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
    /// The events the agent emitted, as prose.
    story: Vec<String>,
    reached_battle: bool,
    ticks: u32,
}

/// Walk out of Oak's lab with every answer held back `latency` ticks, until the rival's battle asks
/// its first question.
fn walk_out_of_the_lab(latency: u32) -> Run {
    let scripted = DeterministicPolicy::new(42, vec![PolicyStep::goto(Map::PalletTown)]);
    let (policy, log) = SlowPolicy::new(Box::new(scripted), latency);
    // The walk plus a handful of turns at this latency.
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

/// Leaving Oak's lab strands no turn in the rival's script, at any latency.
#[test]
fn leaving_oaks_lab_does_not_strand_a_turn_in_the_rivals_script() {
    // An instant answer, five seconds, and a minute.
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

/// The detector fires, or every negative result above means nothing.
#[test]
fn the_detector_notices_a_question_being_replaced() {
    struct Silent;
    impl Policy for Silent {
        fn name(&self) -> &'static str { "scripted" }
        fn pick_overworld_action(&mut self, _: &GameState, _: &WorldGraph) -> Option<OverworldAction> { None }
        fn pick_battle_action(&mut self, _: &GameState) -> Option<BattleAction> { None }
    }

    let (mut policy, log) = SlowPolicy::new(Box::new(Silent), 10);

    for tick in 1..=3 {
        log.lock().expect("the log is never poisoned").tick = tick;
        assert!(!policy.advance(Question::Overworld, "OaksLab (5, 6)".to_string()),
                "a turn with 10 ticks of latency cannot answer on tick {tick}");
    }
    assert!(log.lock().expect("the log is never poisoned").abandoned.is_empty(),
            "polls of the same question advance a turn, they do not replace it");

    // Tick 4 asks something else.
    log.lock().expect("the log is never poisoned").tick = 4;
    policy.advance(Question::Battle, "OaksLab (5, 6)".to_string());

    let log = log.lock().expect("the log is never poisoned");
    assert_eq!(log.abandoned.len(), 1, "the replaced overworld turn is one abandoned turn");
    let abandoned = &log.abandoned[0];
    assert_eq!(abandoned.asked, Question::Overworld);
    assert_eq!(abandoned.replaced_by, Question::Battle);
    assert_eq!(abandoned.started_on, 1, "it started on the tick that opened it");
    // Two, not three: the first poll opens the turn, as `LlmPolicy::advance` returns `None` on it.
    assert_eq!(abandoned.after_ticks, 2, "two polls advanced it before the question changed");
    assert_eq!(log.started.len(), 2, "the replacement is a turn of its own");
}
