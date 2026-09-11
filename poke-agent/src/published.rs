
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use tokio::sync::broadcast;

use crate::pokemon::observe::StatusView;
use crate::frame::{Encoded, Frame, PIXELS};
use gb::lcd_palette::LcdColor;

/// ~2 s of video at 30 fps.
const VIDEO_CAPACITY: usize = 64;
/// Events are small and a viewer catching up on a burst of dialogue is normal, so this is
/// generous.
const EVENT_CAPACITY: usize = 1024;
/// ~1.3 s of audio at 50 packets a second.
const AUDIO_CAPACITY: usize = 64;

// ── Video
// ────────────────────────────────────────────────────────────────────────────────────────

/// One video message, encoded once and shared with every subscriber.
#[derive(Debug, Clone)]
pub struct VideoMessage {
    /// Unwrapped, unlike the `u16` on the wire — a late joiner compares these to decide what to
    /// discard and that comparison is wrong across a wrap (~36 minutes at 30 fps).
    pub seq: u64,
    pub keyframe: bool,
    pub bytes: Arc<[u8]>,
}

impl From<Encoded> for VideoMessage {
    fn from(encoded: Encoded) -> Self {
        Self { seq: encoded.seq, keyframe: encoded.keyframe, bytes: encoded.bytes.into() }
    }
}

pub struct FrameSnapshot {
    pub seq: u64,
    pub pixels: Box<Frame>,
}

// ── Events
// ───────────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct UiEvent {
    /// Monotonic from process start.
    pub seq: u64,
    /// Unix milliseconds — when this was published, on the *wall* clock.
    pub at: u64,
    #[serde(flatten)]
    pub body: UiEventBody,
}

/// Now, in Unix milliseconds. Saturating rather than `expect`: a host whose clock is set before
/// 1970 should lose its timestamps, not its run.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiEventBody {
    /// The 10 Hz heartbeat.
    Status(Box<StatusSnapshot>),
    /// A [`crate::pokemon::agent::AgentEvent`], flattened.
    Agent { kind: &'static str, text: String },
    /// Something the operator should see — a failed agent tick, a policy that ran out.
    Notice { level: &'static str, message: String },

    TurnStarted { turn: u64, kind: &'static str, headline: String },
    /// One fragment of the assistant's prose, as it arrives.
    AssistantDelta { turn: u64, text: String },
    /// One fragment of the model's *thinking*, as it arrives, for the endpoints that stream it
    /// separately from the reply (`reasoning_content`).
    AssistantReasoning { turn: u64, text: String },
    /// A tool the model called.
    ToolCall { turn: u64, id: String, kind: &'static str, name: String, arguments: String },
    /// What one tool call answered, paired with its [`Self::ToolCall`] by `id`.
    ToolResult { turn: u64, id: String, name: String, ok: bool, content: String, image: bool },
    /// The terminal call that ended the turn.
    Decision { turn: u64, summary: String, narration: Option<String>, usage: Option<UsageView> },
    /// The turn was abandoned — the game moved on to a different question, or the model would not
    /// produce a decision.
    TurnCancelled { turn: u64, reason: String },

    #[serde(rename = "run_status")]
    Run { status: RunStatus },
    /// The model's plan, in full, whenever it changes.
    Plan { items: Vec<TodoView> },
    /// The model's battle script, whenever it changes — the program deciding its battle turns.
    BattleScript { source: Option<String>, armed: bool, is_default: bool, last_failure: Option<String> },
    Compacted {
        before: u64,
        after: u64,
        /// How many screenshots stage 1 turned into a line of text.
        images_evicted: usize,
        /// Whether stage 2 ran: eviction alone was not enough and the model wrote a summary.
        summarised: bool,
    },
}

/// What the run is doing right now.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RunStatus {
    /// Before the emulator has run a single cycle.
    Booting,
    /// The agent is driving and no decision is pending.
    Playing,
    /// A request is out and nothing has come back yet.
    AwaitingLlm { kind: &'static str },
    /// Tokens are arriving.
    Streaming,
    /// A tool batch is with the emulator thread, or a screenshot is being encoded.
    RunningTool { name: String },
    Compacting,
    /// A retry is being waited out.
    RateLimited { retry_in_ms: u64 },
    /// The endpoint's quota is exhausted and it said when it reopens, or it has refused a streak of
    /// requests outright, so the whole run is paused — no requests, and the emulator stopped with it.
    Throttled { until_ms: u64, message: String },
    /// The last turn could not be completed.
    Error { message: String },
}

/// One item on the model's plan, as the page draws it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TodoView {
    pub id: u32,
    pub text: String,
    pub done: bool,
}

/// Context occupancy and the run's bill so far. Published with every decision, so a viewer sees
/// the context fill up in real time rather than discovering it in a 400.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct UsageView {
    /// Prompt + completion of the most recent response: how full the window was, last time we
    /// knew.
    pub context_tokens: u64,
    pub context_limit: u64,
    /// Cumulative for the whole run — this is the bill, not the gauge.
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Completions billed this run. More than the number of turns: a turn that reads before it
    /// decides costs several.
    pub completions: u64,
    /// Whether these came from `Usage::estimate` rather than from the endpoint. A guess presented
    /// as a measurement is worse than no number.
    pub estimated: bool,
}

/// What the status panel renders, and the cheapest thing the host can read.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusSnapshot {
    /// Wall-clock milliseconds since the current run started being played by this process, minus
    /// whatever it has spent parked on a spent quota.
    pub wall_ms: u64,
    /// Emulated milliseconds over that same span.
    pub emulated_ms: u64,
    /// Emulated milliseconds the catch-up clamp has discarded on this run. Zero on a host that
    /// has kept up throughout; a one-off number is a stall that has already been recovered from.
    pub dropped_ms: u64,
    /// Emulated milliseconds over the run's whole life, across every process that has played it:
    /// `meta.json`'s total as it stood when this process opened the directory, plus `emulated_ms`
    /// above.
    pub run_emulated_ms: u64,
    pub target_speed: f64,
    /// `"random"` or `"llm"`.
    pub policy: &'static str,
    /// `GB_MODEL` — who is actually playing. `None` under any policy that is not an LLM.
    pub model: Option<String>,
    /// [`crate::pokemon::agent::PokemonAgent::state_debug`] — which arm of the state machine is
    /// driving. The single most useful field when a run looks stuck.
    pub agent_state: String,
    pub frame_seq: u64,
    pub game: Option<StatusView>,
    /// The same value the last [`UiEventBody::Run`] carried, repeated on the heartbeat so a
    /// viewer that joined between two transitions still knows what the run is doing.
    pub run: RunStatus,
}

impl StatusSnapshot {
    /// Whether this says anything the previous one did not — the test the host suppresses a
    /// heartbeat on.
    pub fn says_the_same_as(&self, previous: &Self) -> bool {
        let Self {
            wall_ms: _,
            emulated_ms: _,
            run_emulated_ms: _,
            frame_seq: _,
            dropped_ms,
            target_speed,
            policy,
            model,
            agent_state,
            game,
            run,
        } = self;
        dropped_ms == &previous.dropped_ms
            && target_speed == &previous.target_speed
            && policy == &previous.policy
            && model == &previous.model
            && agent_state == &previous.agent_state
            && game == &previous.game
            && run == &previous.run
    }
}

// ── The buffers
// ──────────────────────────────────────────────────────────────────────────────────

pub struct Published {
    video: broadcast::Sender<VideoMessage>,
    /// One Opus packet, encoded once on the emulator thread for every listener.
    audio: broadcast::Sender<Arc<[u8]>>,
    /// The keyframe a late joiner starts from.
    keyframe: RwLock<Option<VideoMessage>>,
    frame: RwLock<Arc<FrameSnapshot>>,
    events: broadcast::Sender<UiEvent>,
    next_event_seq: AtomicU64,
    /// The current [`RunStatus`].
    status: RwLock<RunStatus>,
    /// The last [`UsageView`] a decision carried, and how many decisions have landed.
    usage: RwLock<Option<UsageView>>,
    /// Decisions that landed, not `max(turn)`.
    turns: AtomicU64,
    save_state: RwLock<Option<(Arc<Vec<u8>>, u64)>>,
    /// The most recent heartbeat, for a client that has just connected.
    latest_status: RwLock<Option<UiEvent>>,
    /// The model's plan, as last published, for a client that has just connected.
    latest_plan: RwLock<Option<UiEvent>>,
    /// The battle script, as last published, for a client that has just connected — the same cell
    /// as [`Self::latest_plan`] and needed more badly.
    latest_battle_script: RwLock<Option<UiEvent>>,
    /// The last few pictures a tool answered with, keyed by the seq of the `ToolResult` naming
    /// them.
    tool_images: RwLock<VecDeque<(u64, Arc<Vec<u8>>)>>,
    /// The Unix millisecond the endpoint's quota reopens, or `0` for "the run is not parked".
    throttled_until: AtomicU64,
}

/// How many tool pictures [`Published::tool_images`] holds.
const TOOL_IMAGE_CACHE: usize = 16;

impl Published {
    pub fn new() -> Arc<Self> {
        Self::resuming(0)
    }

    /// The same, but with the event counter continued from a previous process.
    pub fn resuming(next_seq: u64) -> Arc<Self> {
        Arc::new(Self {
            video: broadcast::channel(VIDEO_CAPACITY).0,
            audio: broadcast::channel(AUDIO_CAPACITY).0,
            keyframe: RwLock::new(None),
            frame: RwLock::new(Arc::new(FrameSnapshot {
                seq: 0,
                pixels: Box::new([LcdColor::WHITE; PIXELS]),
            })),
            events: broadcast::channel(EVENT_CAPACITY).0,
            next_event_seq: AtomicU64::new(next_seq),
            status: RwLock::new(RunStatus::Booting),
            save_state: RwLock::new(None),
            latest_status: RwLock::new(None),
            latest_plan: RwLock::new(None),
            latest_battle_script: RwLock::new(None),
            tool_images: RwLock::new(VecDeque::new()),
            throttled_until: AtomicU64::new(0),
            usage: RwLock::new(None),
            turns: AtomicU64::new(0),
        })
    }

    /// Publish one encoded frame: the standalone keyframe describing the new state, and the delta
    /// that gets an already-connected client there.
    pub fn publish_video(&self, keyframe: Encoded, delta: Encoded) {
        *self.keyframe.write().expect("video keyframe lock poisoned") = Some(keyframe.into());
        let _ = self.video.send(delta.into());
    }

    /// Video messages from here on. Only [`Self::join_video`] and the ordering test want this;
    /// everything else wants the keyframe with it.
    pub fn subscribe_video(&self) -> broadcast::Receiver<VideoMessage> {
        self.video.subscribe()
    }

    /// Subscribe, then take the keyframe to start from — never the other way round.
    pub fn join_video(&self) -> (broadcast::Receiver<VideoMessage>, Option<VideoMessage>) {
        let receiver = self.subscribe_video();
        let keyframe = self.keyframe.read().expect("video keyframe lock poisoned").clone();
        (receiver, keyframe)
    }

    /// The keyframe on its own, for a subscriber that has already lagged out of the ring buffer
    /// and needs to re-sync without dropping its connection. Hand one Opus packet to every
    /// listener.
    pub fn publish_audio(&self, packet: Arc<[u8]>) {
        let _ = self.audio.send(packet);
    }

    /// Subscribe, and that is the whole of it.
    pub fn join_audio(&self) -> broadcast::Receiver<Arc<[u8]>> {
        self.audio.subscribe()
    }

    /// Whether anyone is listening.
    pub fn audio_listeners(&self) -> usize {
        self.audio.receiver_count()
    }

    pub fn latest_keyframe(&self) -> Option<VideoMessage> {
        self.keyframe.read().expect("video keyframe lock poisoned").clone()
    }

    pub fn publish_frame(&self, snapshot: FrameSnapshot) {
        *self.frame.write().expect("frame lock poisoned") = Arc::new(snapshot);
    }

    /// The latest frame as pixels. `Arc`, so a worker encoding a PNG holds the read lock for the
    /// length of one clone rather than the length of the encode.
    pub fn latest_frame(&self) -> Arc<FrameSnapshot> {
        Arc::clone(&self.frame.read().expect("frame lock poisoned"))
    }

    /// Keep the machine as it is now, for a report filed during the turn that is starting.
    pub fn publish_save_state(&self, state: Vec<u8>) {
        *self.save_state.write().expect("save state lock poisoned") =
            Some((Arc::new(state), now_ms()));
    }

    pub fn latest_save_state(&self) -> Option<(Arc<Vec<u8>>, u64)> {
        self.save_state.read().expect("save state lock poisoned").clone()
    }

    /// Stamp a sequence number on an event body and broadcast it.
    pub fn publish_event(&self, body: UiEventBody) -> u64 {
        // The fold behind `usage()`/`turns()`.
        if let UiEventBody::Decision { usage, .. } = &body {
            self.turns.fetch_add(1, Ordering::Relaxed);
            if let Some(view) = usage {
                *self.usage.write().expect("usage lock poisoned") = Some(*view);
            }
        }
        let seq = self.next_event_seq.fetch_add(1, Ordering::Relaxed);
        let event = UiEvent { seq, at: now_ms(), body };
        // Kept before the send, for the reason `publish_status` gives: a client that joins in the
        // gap should see a stale plan rather than none.
        if matches!(event.body, UiEventBody::Plan { .. }) {
            *self.latest_plan.write().expect("plan lock poisoned") = Some(event.clone());
        }
        if matches!(event.body, UiEventBody::BattleScript { .. }) {
            *self.latest_battle_script.write().expect("battle script lock poisoned") = Some(event.clone());
        }
        let _ = self.events.send(event);
        seq
    }

    /// Keep a picture a tool answered with, under the seq of the `ToolResult` that announced it.
    pub fn put_tool_image(&self, seq: u64, png: Vec<u8>) {
        let mut images = self.tool_images.write().expect("tool image lock poisoned");
        images.push_back((seq, Arc::new(png)));
        while images.len() > TOOL_IMAGE_CACHE {
            images.pop_front();
        }
    }

    /// A picture by the seq of the event that named it. `None` once it has fallen off the ring,
    /// which is an ordinary 404 rather than a fault.
    pub fn tool_image(&self, seq: u64) -> Option<Arc<Vec<u8>>> {
        let images = self.tool_images.read().expect("tool image lock poisoned");
        images.iter().find(|(at, _)| *at == seq).map(|(_, png)| Arc::clone(png))
    }

    /// What the run has spent, as of the last decision that reported figures. `None` under any
    /// policy that is not an LLM, and until the first decision under one.
    pub fn usage(&self) -> Option<UsageView> {
        *self.usage.read().expect("usage lock poisoned")
    }

    /// Decisions that have landed in this process. See the field for why this is not a turn id.
    pub fn turns(&self) -> u64 {
        self.turns.load(Ordering::Relaxed)
    }

    /// Forget what the *previous* run spent, when a new one becomes current.
    pub fn forget_usage(&self) {
        *self.usage.write().expect("usage lock poisoned") = None;
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<UiEvent> {
        self.events.subscribe()
    }

    /// Publish a heartbeat and keep it as the one a new client is handed.
    pub fn publish_status(&self, snapshot: StatusSnapshot) -> u64 {
        let seq = self.next_event_seq.fetch_add(1, Ordering::Relaxed);
        let event = UiEvent { seq, at: now_ms(), body: UiEventBody::Status(Box::new(snapshot)) };
        *self.latest_status.write().expect("status lock poisoned") = Some(event.clone());
        let _ = self.events.send(event);
        seq
    }

    /// Subscribe, then take the events to open with — never the other way round.
    pub fn join_events(&self) -> (broadcast::Receiver<UiEvent>, Vec<UiEvent>) {
        let receiver = self.events.subscribe();
        let mut opening: Vec<UiEvent> = [
            self.latest_plan.read().expect("plan lock poisoned").clone(),
            self.latest_battle_script.read().expect("battle script lock poisoned").clone(),
            self.latest_status.read().expect("status lock poisoned").clone(),
        ]
        .into_iter()
        .flatten()
        .collect();
        opening.sort_by_key(|event| event.seq);
        (receiver, opening)
    }

    /// The heartbeat as last published, for something that wants the whole picture at a moment
    /// rather than a subscription to every change of it.
    pub fn latest_status(&self) -> Option<UiEvent> {
        self.latest_status.read().expect("status lock poisoned").clone()
    }

    /// Record what the run is doing, and say so if it changed.
    pub fn set_status(&self, status: RunStatus) {
        // Read first, because the overwhelmingly common call is a repeat — `Streaming` is set
        // once per token of a reply — and a repeat should not take the write lock at all.
        if *self.status.read().expect("status lock poisoned") == status {
            return;
        }
        {
            let mut current = self.status.write().expect("status lock poisoned");
            if *current == status {
                return;
            }
            *current = status.clone();
        }
        self.publish_event(UiEventBody::Run { status });
    }

    pub fn run_status(&self) -> RunStatus {
        self.status.read().expect("status lock poisoned").clone()
    }

    /// Park the run until `until_ms` (Unix milliseconds), or `0` to release it.
    pub fn set_throttled_until(&self, until_ms: u64) {
        self.throttled_until.store(until_ms, Ordering::Relaxed);
    }

    /// The moment the run may resume, if it is parked. Read by
    /// `crate::host::EmulatorHost::tick`.
    pub fn throttled_until(&self) -> Option<u64> {
        match self.throttled_until.load(Ordering::Relaxed) {
            0 => None,
            until => Some(until),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fold behind `usage()`/`turns()`, which the emulator thread reads when it files a
    /// finished run.
    #[test]
    fn a_decision_is_what_counts_towards_a_runs_bill() {
        let published = Published::new();
        assert_eq!(published.turns(), 0);
        assert!(published.usage().is_none(), "nothing has been spent under --policy random, ever");

        let spent = |prompt| UsageView {
            context_tokens: 100,
            context_limit: 1000,
            prompt_tokens: prompt,
            completion_tokens: 7,
            completions: 3,
            estimated: false,
        };
        published.publish_event(UiEventBody::Decision {
            turn: 41,
            summary: "walk to Oak's lab".into(),
            narration: None,
            usage: Some(spent(1_000)),
        });
        assert_eq!(published.turns(), 1);
        assert_eq!(published.usage().map(|u| u.prompt_tokens), Some(1_000));

        // A turn the endpoint reported no usage for still decided something.
        published.publish_event(UiEventBody::Decision { turn: 42, summary: "fight".into(), narration: None, usage: None });
        assert_eq!(published.turns(), 2);
        assert_eq!(published.usage().map(|u| u.prompt_tokens), Some(1_000), "the last real figure stands");

        published.publish_event(UiEventBody::Decision {
            turn: 43,
            summary: "fight".into(),
            narration: None,
            usage: Some(spent(2_500)),
        });
        assert_eq!(published.usage().map(|u| u.prompt_tokens), Some(2_500), "and is replaced when one arrives");

        let before = published.turns();
        published.publish_event(UiEventBody::TurnCancelled { turn: 44, reason: "the game moved on".into() });
        published.publish_event(UiEventBody::Agent { kind: "text_box", text: "HELLO".into() });
        assert_eq!(published.turns(), before, "an abandoned turn and a text box are not decisions");

        // A new run must not inherit the last one's bill — the worker's own reset is
        // asynchronous, so between the swap and the next decision this cell is the only thing
        // that says whose tokens these were.
        published.forget_usage();
        assert!(published.usage().is_none());
        assert_eq!(published.turns(), before, "…and the turn counter is a mark the host subtracts, \
                                               not something that goes backwards under a reader");
    }

    #[test]
    fn events_are_numbered_from_zero_and_reach_a_subscriber() {
        let published = Published::new();
        published.publish_event(UiEventBody::Notice { level: "info", message: "before".into() });

        let mut receiver = published.subscribe_events();
        let seq = published.publish_event(UiEventBody::Notice { level: "info", message: "after".into() });
        assert_eq!(seq, 1, "sequence numbers count every event, not only the delivered ones");

        let received = receiver.try_recv().expect("subscribed before the send");
        assert_eq!(received.seq, 1);
        assert!(receiver.try_recv().is_err(), "a subscriber does not get events from before it joined");

        let resumed = Published::resuming(500);
        assert_eq!(resumed.publish_event(UiEventBody::Notice { level: "info", message: "later".into() }), 500);
    }

    /// Every event is stamped with the wall clock, and the stamp survives to the wire.
    #[test]
    fn an_event_carries_the_time_it_was_published() {
        let published = Published::new();
        let mut receiver = published.subscribe_events();
        let before = now_ms();
        published.publish_event(UiEventBody::Notice { level: "info", message: "hello".into() });
        published.publish_status(snapshot("wait", 1));
        let after = now_ms();

        let event = receiver.try_recv().expect("the notice");
        assert!((before..=after).contains(&event.at), "{} is not between {before} and {after}", event.at);
        let json = serde_json::to_value(&event).expect("serialises");
        assert_eq!(json["at"], event.at, "the SPA reads `at` off the event itself, beside `seq`");

        let heartbeat = receiver.try_recv().expect("the heartbeat");
        assert!(heartbeat.at >= event.at, "the stamps are in publication order");
    }

    /// Who is playing rides on the heartbeat, because the page's title says it and a title that
    /// waits for the first decision would be wrong for the first minute of every run.
    #[test]
    fn the_heartbeat_says_which_model_is_playing() {
        let json = serde_json::to_value(snapshot("wait", 1)).expect("serialises");
        assert_eq!(json["model"], "gpt-5");
        assert_eq!(json["policy"], "llm");

        let random = StatusSnapshot { policy: "random", model: None, ..snapshot("wait", 1) };
        assert!(random.model.is_none(), "`random` is not a model name and must not be shown as one");
        assert_eq!(serde_json::to_value(&random).expect("serialises")["model"], serde_json::Value::Null);

        // And it takes part in the suppression, or the first heartbeat after a change would be
        // held back for saying nothing new.
        assert!(!random.says_the_same_as(&snapshot("wait", 1)));
    }

    #[test]
    fn a_status_is_broadcast_on_transition_and_only_on_transition() {
        let published = Published::new();
        assert_eq!(published.run_status(), RunStatus::Booting, "nothing has emulated a cycle yet");
        let mut receiver = published.subscribe_events();

        published.set_status(RunStatus::Playing);
        published.set_status(RunStatus::Playing);
        published.set_status(RunStatus::AwaitingLlm { kind: "overworld" });
        published.set_status(RunStatus::Playing);

        let states: Vec<RunStatus> = std::iter::from_fn(|| receiver.try_recv().ok())
            .filter_map(|event| match event.body {
                UiEventBody::Run { status } => Some(status),
                _ => None,
            })
            .collect();
        assert_eq!(states, [
            RunStatus::Playing,
            RunStatus::AwaitingLlm { kind: "overworld" },
            RunStatus::Playing,
        ]);
        assert_eq!(published.run_status(), RunStatus::Playing, "and the latest is readable directly");
    }

    fn snapshot(agent_state: &str, wall_ms: u64) -> StatusSnapshot {
        StatusSnapshot {
            wall_ms,
            emulated_ms: wall_ms,
            run_emulated_ms: wall_ms,
            dropped_ms: 0,
            target_speed: 1.0,
            policy: "llm",
            model: Some("gpt-5".to_string()),
            agent_state: agent_state.to_string(),
            frame_seq: wall_ms / 33,
            game: None,
            run: RunStatus::Playing,
        }
    }

    /// The comparison the whole send-on-change rule rests on.
    #[test]
    fn a_heartbeat_is_the_same_as_another_when_only_the_clock_has_moved() {
        assert!(snapshot("wait", 5_000).says_the_same_as(&snapshot("wait", 100)));
        assert!(!snapshot("move→Warp", 100).says_the_same_as(&snapshot("wait", 100)));

        let mut moved = snapshot("wait", 100);
        moved.run = RunStatus::Streaming;
        assert!(!moved.says_the_same_as(&snapshot("wait", 100)), "the run status is state, not clock");

        // `dropped_ms` sits beside the two clocks and is the opposite of them.
        let mut behind = snapshot("wait", 100);
        behind.dropped_ms = 400;
        assert!(
            !behind.says_the_same_as(&snapshot("wait", 100)),
            "time the host dropped is news, not a clock",
        );
        assert!(
            behind.says_the_same_as(&StatusSnapshot { dropped_ms: 400, ..snapshot("wait", 9_000) }),
            "and having dropped the same amount a while ago is not news again",
        );
    }

    /// The other half of send-on-change: a page that opens while nothing is happening must not
    /// wait for something to happen.
    #[test]
    fn a_joiner_is_handed_the_last_heartbeat_rather_than_an_empty_panel() {
        let published = Published::new();
        assert!(published.join_events().1.is_empty(), "nothing has been published yet");

        published.publish_status(snapshot("wait", 100));
        published.publish_status(snapshot("move→Warp", 600));

        let (mut receiver, opening) = published.join_events();
        assert_eq!(opening.len(), 1, "no plan has been published, so only the heartbeat");
        let latest = opening.into_iter().next().expect("the joiner opens with the most recent one");
        let UiEventBody::Status(status) = latest.body else { panic!("a status") };
        assert_eq!(status.agent_state, "move→Warp");
        assert_eq!(latest.seq, 1, "…and it keeps the sequence number it was published with");
        assert!(receiver.try_recv().is_err(), "the backlog is one heartbeat, not the history");

        published.publish_status(snapshot("wait", 1_100));
        let UiEvent { body: UiEventBody::Status(next), .. } = receiver.try_recv().expect("live") else {
            panic!("a status")
        };
        assert_eq!(next.agent_state, "wait", "and the stream carries on from there");
    }

    /// The plan is the send-on-change event a reload could not recover, and it failed silently:
    /// the panel simply was not there (`PlanPanel` renders nothing for an empty list), so it read
    /// as a styling problem rather than as a missing event.
    #[test]
    fn a_joiner_is_handed_the_plan_as_well_as_the_heartbeat() {
        let published = Published::new();
        let item = |id: u32, text: &str| TodoView { id, text: text.to_string(), done: false };

        published.publish_event(UiEventBody::Plan { items: vec![item(1, "get the Boulder Badge")] });
        for _ in 0..50 {
            published.publish_event(UiEventBody::AssistantReasoning { turn: 1, text: "…".into() });
        }
        published.publish_status(snapshot("wait", 100));

        let (_receiver, opening) = published.join_events();
        assert_eq!(opening.len(), 2, "the plan and the heartbeat: {opening:#?}");
        // Oldest first, so the page applies them in the order they happened.
        let UiEventBody::Plan { items } = &opening[0].body else { panic!("the plan first") };
        assert_eq!(items.len(), 1);
        assert!(matches!(opening[1].body, UiEventBody::Status(_)), "then the heartbeat");

        // Absolutely stated, so the newest one is the whole answer and replaces the last.
        published.publish_event(UiEventBody::Plan { items: vec![item(1, "done"), item(2, "Cerulean")] });
        let (_receiver, opening) = published.join_events();
        let plan = opening.iter().find_map(|event| match &event.body {
            UiEventBody::Plan { items } => Some(items),
            _ => None,
        });
        assert_eq!(plan.map(Vec::len), Some(2), "the latest list, not an accumulation of every one");
        assert!(opening.windows(2).all(|pair| pair[0].seq < pair[1].seq), "oldest first: {opening:#?}");
    }

    /// The same hole as the plan's, one order of magnitude worse.
    #[test]
    fn a_joiner_is_handed_the_battle_script_as_well() {
        let published = Published::new();
        let armed = |source: &str| UiEventBody::BattleScript {
            source: Some(source.to_string()),
            armed: true,
            is_default: false,
            last_failure: None,
        };

        published.publish_event(armed("battle.fight(battle.best_move);"));
        // A run's worth of noise on top, which is what a `MAX_BACKLOG` window walks past in
        // minutes.
        for _ in 0..50 {
            published.publish_event(UiEventBody::AssistantReasoning { turn: 1, text: "…".into() });
        }
        published.publish_event(UiEventBody::Plan { items: vec![TodoView {
            id: 1,
            text: "get the Boulder Badge".to_string(),
            done: false,
        }] });
        published.publish_status(snapshot("wait", 100));

        let (_receiver, opening) = published.join_events();
        assert_eq!(opening.len(), 3, "the script, the plan and the heartbeat: {opening:#?}");
        assert!(opening.windows(2).all(|pair| pair[0].seq < pair[1].seq), "oldest first: {opening:#?}");

        // Absolutely stated, exactly as the plan is: the newest replaces the last rather than
        // adding to it, so a disarm is delivered to a joiner as the whole current state.
        published.publish_event(UiEventBody::BattleScript {
            source: Some("battle.fight(battle.best_move);".to_string()),
            armed: false,
            is_default: false,
            last_failure: Some("it named a move the Pokémon does not know".to_string()),
        });
        let (_receiver, opening) = published.join_events();
        let script = opening.iter().find_map(|event| match &event.body {
            UiEventBody::BattleScript { armed, last_failure, .. } => Some((*armed, last_failure.clone())),
            _ => None,
        });
        assert_eq!(
            script,
            Some((false, Some("it named a move the Pokémon does not know".to_string()))),
            "the latest state, not the first one it was armed with",
        );
    }

    /// A picture is referenced by seq and fetched separately, so the ring is what decides whether
    /// a viewer can still open it.
    #[test]
    fn a_tool_picture_is_kept_for_a_while_and_then_is_not() {
        let published = Published::new();
        for seq in 0..(TOOL_IMAGE_CACHE as u64 + 4) {
            published.put_tool_image(seq, vec![seq as u8]);
        }
        assert!(published.tool_image(0).is_none(), "the oldest have fallen off the back");
        assert!(published.tool_image(3).is_none());
        let newest = TOOL_IMAGE_CACHE as u64 + 3;
        assert_eq!(published.tool_image(newest).as_deref(), Some(&vec![newest as u8]));
    }

    /// The wire shape, because the SPA's `api.ts` is written against it by hand: a run status is
    /// one flat object with a `state` discriminator, not a nested one.
    #[test]
    fn a_run_status_serialises_flat_with_a_state_discriminator() {
        let json = serde_json::to_value(UiEvent {
            seq: 7,
            at: 1_760_000_000_000,
            body: UiEventBody::Run { status: RunStatus::RunningTool { name: "read_map".into() } },
        })
        .expect("serialises");
        assert_eq!(json["type"], "run_status");
        assert_eq!(json["status"]["state"], "running_tool");
        assert_eq!(json["status"]["name"], "read_map");

        let json = serde_json::to_value(UiEventBody::Run { status: RunStatus::Booting }).expect("serialises");
        assert_eq!(json["status"]["state"], "booting", "a unit variant is still an object");
    }
}
