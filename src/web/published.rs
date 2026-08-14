//! **W1.2 / W2** — the buffers the emulator thread writes and the HTTP layer reads.
//!
//! This module is the *whole* interface between the two. The web layer never holds a `GameBoy`, a
//! `PokemonAgent` or a channel back into either, which is what makes "strictly view-only" (§1.1 of
//! `docs/llm-web-playthrough-plan.md`) structural rather than a matter of not exposing a POST route:
//! there is nothing to write to.
//!
//! The only `tokio` types here are the two [`broadcast::Sender`]s, and they are used purely as the
//! sync→async bridge — `broadcast::Sender::send` is synchronous and non-blocking, callable from a
//! plain `std::thread` with no runtime handle and no `block_on`. `broadcast` drops the oldest message
//! for a slow client rather than blocking the producer, which is exactly right for video (the client
//! is told it lagged and re-syncs from a keyframe) and acceptable for events (the client recovers via
//! `/api/history` in W7).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use tokio::sync::broadcast;

use crate::pokemon::observe::StatusView;
use crate::web::video::{Encoded, Frame, PIXELS};
use crate::lcd_palette::LcdColor;

/// ~2 s of video at 30 fps. A client further behind than this has a problem a bigger buffer would
/// not fix, and is better served by being told it lagged and handed a fresh keyframe.
const VIDEO_CAPACITY: usize = 64;
/// Events are small and a viewer catching up on a burst of dialogue is normal, so this is generous.
const EVENT_CAPACITY: usize = 1024;

// ── Video ────────────────────────────────────────────────────────────────────────────────────────

/// One video message, encoded **once** and shared with every subscriber.
///
/// ⚠️ **Binary, and not base64.** It used to be base64'd here, once for all subscribers, so the SSE
/// route could put it on a `data:` line. `src/web/video/bench.rs` measured what that costs: 33%
/// before compression, as expected — but **69–113% after it**, because base64 shifts a repeating
/// byte pattern into three different alphabet phases and an LZ77 window can no longer see it as a
/// repeat. Since the connection is deflated (see `src/web/mod.rs`), base64 was the single most
/// expensive thing in the video path. The compression is per connection, so there is nothing left to
/// share here beyond the bytes themselves.
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

/// The most recent frame, in pixels rather than on the wire — what W5's `screenshot` tool PNG-encodes
/// on the worker thread. Published beside the video so a screenshot never costs an emulator round
/// trip.
pub struct FrameSnapshot {
    pub seq: u64,
    pub pixels: Box<Frame>,
}

// ── Events ───────────────────────────────────────────────────────────────────────────────────────

/// Everything the UI is told about, and (from W7) one line of `transcript.jsonl` each.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UiEvent {
    /// Monotonic from process start. `GET /api/history?since=<seq>` in W7 replays from here.
    pub seq: u64,
    /// Unix milliseconds — when this was published, on the *wall* clock.
    ///
    /// ⚠️ **Wall clock rather than the run's own clocks, and it is the only one that can answer the
    /// question the page asks.** `StatusSnapshot`'s `wall_ms`/`emulated_ms` are elapsed times since
    /// *this process* started, so a run resumed nightly restarts both — a log row stamped from
    /// either would say a line arrived four minutes in when it arrived last Tuesday. This is
    /// absolute, so it survives the resume, and it is stamped once here rather than at the browser
    /// because `/api/history` replays a backlog that may be hours old: a client-side clock would
    /// date the whole backfill to the moment the page loaded.
    ///
    /// A transcript written before this field existed has no `at`, which is why the SPA's copy is
    /// optional — an old run's backlog is still readable, it just has no times.
    pub at: u64,
    #[serde(flatten)]
    pub body: UiEventBody,
}

/// Now, in Unix milliseconds. Saturating rather than `expect`: a host whose clock is set before 1970
/// should lose its timestamps, not its run.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiEventBody {
    /// The 10 Hz heartbeat. Also the thing that makes `curl -N /api/events` a useful liveness check.
    Status(Box<StatusSnapshot>),
    /// A [`crate::pokemon::agent::AgentEvent`], flattened. `kind` is the variant name so a UI can
    /// style it without parsing prose; `text` is the `Display` the console has always printed.
    Agent { kind: &'static str, text: String },
    /// Something the operator should see — a failed agent tick, a policy that ran out.
    Notice { level: &'static str, message: String },

    // ── W4: the LLM's side of the conversation ───────────────────────────────────────────────────
    //
    // Every one of these carries `turn`, and the client groups on it: a turn is one bubble, not one
    // bubble per token. That is also what lets a viewer joining mid-turn drop the fragments of a
    // turn it did not see the start of.
    /// A decision has been asked for. `headline` is a sentence, not the thousand tokens that were
    /// actually sent — the full prompt is the transcript's business (W7).
    TurnStarted { turn: u64, kind: &'static str, headline: String },
    /// One fragment of the assistant's prose, as it arrives.
    AssistantDelta { turn: u64, text: String },
    /// One fragment of the model's *thinking*, as it arrives, for the endpoints that stream it
    /// separately from the reply (`reasoning_content`).
    ///
    /// A separate event rather than an `AssistantDelta` because the page treats it differently — it
    /// is shown live and then collapsed once the thought ends — and because it is the one thing the
    /// model says that is never sent back to it. ⚠️ **The turn alone does not delimit a thought**:
    /// a turn that reads before it decides thinks once per completion, so the client closes the
    /// block on the next event of any other kind rather than on the turn changing.
    AssistantReasoning { turn: u64, text: String },
    /// A tool the model called. `arguments` is the raw JSON string it sent.
    ///
    /// `id` is the endpoint's own call id, and it is here for one reason: it is the only thing that
    /// pairs this with the [`Self::ToolResult`] that answers it. A turn can call several tools in
    /// one message and they are answered as a batch, so position and arrival order are both
    /// unreliable. `kind` is `tools::CallKind`'s discriminant — `read`, `todo`, `terminal`,
    /// `rejected` — which the client cannot work out from the name alone and which is what lets it
    /// show a refused call as refused rather than as a call that did nothing.
    ToolCall { turn: u64, id: String, kind: &'static str, name: String, arguments: String },
    /// What one tool call answered, paired with its [`Self::ToolCall`] by `id`.
    ///
    /// ⚠️ **`content` is what the *model* was told, truncated.** It is the same string that went
    /// into the history, so the page shows the conversation rather than a second rendering of it —
    /// but a `read_map` answer is a few kilobytes of JSON and every one of these lands in
    /// `transcript.jsonl` for the length of the run, so it is cut at [`MAX_TOOL_RESULT`] with a note
    /// saying so.
    ///
    /// ⚠️ **A picture is referenced, never carried.** `read_map` and `screenshot` answer with a
    /// caption *and* an image; the image is a couple of hundred kilobytes of PNG, which would be a
    /// third as much again as base64 in an SSE frame and would then be written to the transcript
    /// on top of that. `image` says only that there is one, and it is fetched from
    /// `/api/tool-image/{seq}.png` against this event's own seq — served out of a small ring in
    /// [`Published`], so a live viewer gets it and a page replaying an hour-old backlog gets a 404
    /// and shows the caption alone. Same reasoning as the video stream's: never put bytes on a
    /// channel that is also an archive.
    ToolResult { turn: u64, id: String, name: String, ok: bool, content: String, image: bool },
    /// The terminal call that ended the turn.
    ///
    /// `summary` is *ours* — `worker::describe`, the mechanical account of what the agent was told
    /// to do. `narration` is the **model's**, from the `summary` argument every terminal tool
    /// carries: one or two sentences of why. It is `None` when the model omitted it and on the
    /// forced wait, which is the loop's decision and not the model's.
    Decision { turn: u64, summary: String, narration: Option<String>, usage: Option<UsageView> },
    /// The turn was abandoned — the game moved on to a different question, or the model would not
    /// produce a decision. §17's risk 2b is that this becomes a *rate*, so it is an event rather
    /// than a silence.
    TurnCancelled { turn: u64, reason: String },

    // ── W6: what the run is doing, and what it has spent ─────────────────────────────────────────
    /// A [`RunStatus`] transition. Sent only when the status actually changes, and mirrored on every
    /// [`StatusSnapshot`] so a viewer joining mid-run does not have to wait for the next transition
    /// to find out what is happening.
    #[serde(rename = "run_status")]
    Run { status: RunStatus },
    /// **W6b / §10** — the model's plan, in full, whenever it changes.
    ///
    /// The one thing on this page that is neither the game nor the conversation: it is what the run
    /// is *trying* to do, which a viewer cannot infer from either. Published on change rather than
    /// on a timer, and replayed by `/api/history`, so a page opened an hour in shows the current
    /// list — the fold keeps the latest and discards every earlier one.
    Plan { items: Vec<TodoView> },
    /// §9 — the history was compacted. `before` and `after` are tokens, on the calibrated scale
    /// `llm::accounting` describes.
    Compacted {
        before: u64,
        after: u64,
        /// How many screenshots stage 1 turned into a line of text.
        images_evicted: usize,
        /// Whether stage 2 ran: eviction alone was not enough and the model wrote a summary.
        summarised: bool,
    },
}

/// **W6 / §9** — what the run is doing right now.
///
/// W1 deliberately had no such type and let the UI infer the state from the event stream; five
/// phases in, that inference has become "look at which event arrived last and hope", so this is the
/// answer written down. It rides on both the transition event and the 10 Hz heartbeat: the event so
/// a viewer sees `Streaming` the instant it starts, the heartbeat so a late joiner is never more
/// than 100 ms from the truth without needing W7's history endpoint.
///
/// `kind` is a `&'static str` — `DecisionKind::label` — rather than the enum itself, because this
/// module is the interface to the *web* half and must compile without the `llm` feature.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RunStatus {
    /// Before the emulator has run a single cycle.
    Booting,
    /// The agent is driving and no decision is pending. Under `--policy random` this is the whole
    /// state machine.
    Playing,
    /// A request is out and nothing has come back yet.
    AwaitingLlm { kind: &'static str },
    /// Tokens are arriving.
    Streaming,
    /// A tool batch is with the emulator thread, or a screenshot is being encoded.
    RunningTool { name: String },
    Compacting,
    /// A retry is being waited out. Named for the case that dominates, but any retryable failure
    /// lands here — what matters to a viewer is that the run is stalled and for how long.
    RateLimited { retry_in_ms: u64 },
    /// The endpoint's quota is exhausted and it said when it reopens, so **the whole run is
    /// paused** — no requests, and the emulator stopped with it.
    ///
    /// ⚠️ **Distinct from [`Self::RateLimited`], which is a backoff of a few seconds.** This one can
    /// be hours, and the two want opposite things from a viewer: one is "wait a moment", the other
    /// is "come back later, and nothing is lost in the meantime".
    ///
    /// `until_ms` is Unix milliseconds, not a duration, for the reason `UiEvent::at` is: it is
    /// replayed on the heartbeat and read by a page that may have joined long after it was set, so a
    /// countdown has to be derived from an absolute moment rather than from a number that was true
    /// once.
    Throttled { until_ms: u64, message: String },
    /// The last turn could not be completed. Left in place until the next turn starts, because a
    /// status that flicks straight back to `Playing` is a status nobody ever sees.
    Error { message: String },
}

/// One item on the model's plan, as the page draws it.
///
/// ⚠️ **Its own type rather than `llm::todo::TodoItem`**, for the same reason [`RunStatus`] takes a
/// `&'static str` where the worker has a `DecisionKind`: this module is the interface to the *web*
/// half and must compile with the `llm` feature off.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TodoView {
    pub id: u32,
    pub text: String,
    pub done: bool,
}

/// Context occupancy and the run's bill so far. Published with every decision, so a viewer sees the
/// context fill up in real time rather than discovering it in a 400.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct UsageView {
    /// Prompt + completion of the most recent response: how full the window was, last time we knew.
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
///
/// `game` is `Option` because a `GameState` is not always readable — during a screen transition, or
/// before the save state has settled — and a status heartbeat that stops arriving is much harder to
/// diagnose than one that arrives saying it could not read the game.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusSnapshot {
    /// Wall-clock milliseconds since the host started.
    pub wall_ms: u64,
    /// Emulated milliseconds since the host started. The ratio of the two is the speed the emulator
    /// is actually achieving, as against the speed it is targeting.
    pub emulated_ms: u64,
    pub target_speed: f64,
    /// `"random"` or `"llm"`.
    pub policy: &'static str,
    /// `GB_MODEL` — who is actually playing. `None` under any policy that is not an LLM.
    ///
    /// ⚠️ **The same rule the leaderboard's column follows** (see `EmulatorHost::file_completed_run`):
    /// `RunMeta::model` is the literal `"random"` under `--policy random`, and a page saying "random
    /// plays Pokémon Red" as if that were a model would be a small lie. The page's own fallback is
    /// the policy name, which is already beside it.
    pub model: Option<String>,
    /// [`crate::pokemon::agent::PokemonAgent::state_debug`] — which arm of the state machine is
    /// driving. The single most useful field when a run looks stuck.
    pub agent_state: String,
    pub frame_seq: u64,
    pub game: Option<StatusView>,
    /// **W6** — the same value the last [`UiEventBody::Run`] carried, repeated on the heartbeat so a
    /// viewer that joined between two transitions still knows what the run is doing.
    pub run: RunStatus,
}

impl StatusSnapshot {
    /// Whether this says anything the previous one did not — the test the host suppresses a
    /// heartbeat on.
    ///
    /// ⚠️ **A derived `PartialEq` would be useless here**, and quietly so: `wall_ms` and
    /// `emulated_ms` differ on every single sample, so nothing would ever compare equal and the
    /// suppression would silently never fire. They are excluded for the same reason `Audio` and
    /// `PPU` exclude their derived state (see `CLAUDE.md`) — they are the clock, not the state.
    ///
    /// `frame_seq` is excluded too. It advances at 30 Hz whenever *anything* on screen moves, which
    /// is most of the time, and nothing in the UI renders it; leaving it in would mean the picture
    /// moving forced a status resend, which is precisely the traffic this is here to remove.
    pub fn says_the_same_as(&self, previous: &Self) -> bool {
        let Self { wall_ms: _, emulated_ms: _, frame_seq: _, target_speed, policy, model, agent_state, game, run } =
            self;
        target_speed == &previous.target_speed
            && policy == &previous.policy
            && model == &previous.model
            && agent_state == &previous.agent_state
            && game == &previous.game
            && run == &previous.run
    }
}

// ── The buffers ──────────────────────────────────────────────────────────────────────────────────

pub struct Published {
    video: broadcast::Sender<VideoMessage>,
    /// The keyframe a late joiner starts from. **Stored before the matching delta is broadcast** —
    /// see [`Published::publish_video`], where the ordering argument lives.
    keyframe: RwLock<Option<VideoMessage>>,
    frame: RwLock<Arc<FrameSnapshot>>,
    events: broadcast::Sender<UiEvent>,
    next_event_seq: AtomicU64,
    /// **W6** — the current [`RunStatus`]. A lock rather than an event-stream fold, because the
    /// heartbeat has to be able to ask for it and a fold cannot answer a question.
    status: RwLock<RunStatus>,
    /// The last [`UsageView`] a decision carried, and how many decisions have landed.
    ///
    /// ⚠️ **A fold, and it is here for the reason `status` is** (see its ⚠️): the emulator thread
    /// has to be able to *ask* what the run has spent — at the moment it finishes the game, so it
    /// can write it into the run's permanent record — and a broadcast channel cannot answer a
    /// question. `Accounting` lives on the LLM worker thread and this module is the whole interface
    /// between the two halves, so the answer travels the interface that already exists rather than
    /// growing a second one.
    ///
    /// ⚠️ **Folded inside [`Self::publish_event`], not at a second call site.** The one thing that
    /// must not happen is a decision that reaches the page but not the record.
    usage: RwLock<Option<UsageView>>,
    /// ⚠️ **Decisions that landed, not `max(turn)`.** A turn id is `llm::worker`'s cancellation
    /// generation: it counts turns that were abandoned as well, and it restarts at 1 in every
    /// process, so it is not a count of anything a run's record wants.
    turns: AtomicU64,
    /// The most recent heartbeat, for a client that has just connected.
    ///
    /// ⚠️ **This is what makes send-on-change safe, and it is one cell rather than one per client.**
    /// Once the host stops resending an unchanged status, a viewer who connects during a quiet
    /// stretch would otherwise stare at an empty panel until something moved. Same shape as the
    /// video keyframe (§5.2): subscribe, then read the latest.
    latest_status: RwLock<Option<UiEvent>>,
    /// The model's plan, as last published, for a client that has just connected.
    ///
    /// ⚠️ **The plan is published on change and a change can be an hour apart, so without this a
    /// reload showed no plan at all.** It looked like a client bug and was not: the panel is fed by
    /// `UiEventBody::Plan`, `join_events` opened with the heartbeat alone, and the other route —
    /// `/api/history` — keeps only the most recent `MAX_BACKLOG` events. A reasoning model publishes
    /// one event *per streamed token*, so a couple of turns is thousands of rows and the last `Plan`
    /// is off the end of the window within minutes. Both paths failed for different reasons, which
    /// is why the fix is here, in the one place that cannot be outrun: the same subscribe-then-read
    /// handshake as the heartbeat and the video keyframe.
    ///
    /// The plan is *absolutely* stated — every event carries the whole list — so replaying the last
    /// one is complete, and a duplicate is idempotent at the client.
    latest_plan: RwLock<Option<UiEvent>>,
    /// The last few pictures a tool answered with, keyed by the seq of the `ToolResult` naming them.
    ///
    /// ⚠️ **Bounded, and a miss is an expected answer rather than an error.** A map render is a
    /// couple of hundred kilobytes; the point of holding them here is that a viewer watching live
    /// can open the picture the model was just looking at, not that the run keeps every one it ever
    /// drew. Old entries fall off the back and the page shows the caption without the picture.
    tool_images: RwLock<VecDeque<(u64, Arc<Vec<u8>>)>>,
    /// The Unix millisecond the endpoint's quota reopens, or `0` for "the run is not parked".
    ///
    /// The one thing in here the *emulator* thread reads on its hot path rather than writes — see
    /// [`Self::set_throttled_until`] for why it is an atomic and not the run status itself.
    throttled_until: AtomicU64,
}

/// How many tool pictures [`Published::tool_images`] holds. Sized for "what is on screen in the
/// conversation log right now" rather than for history — a Celadon render is ~200 KB, so this is a
/// few megabytes at worst.
const TOOL_IMAGE_CACHE: usize = 16;

impl Published {
    pub fn new() -> Arc<Self> {
        Self::resuming(0)
    }

    /// **W7** — the same, but with the event counter continued from a previous process.
    ///
    /// ⚠️ Sequence numbers are the transcript's only ordering and the browser's only key. A resumed
    /// run that started them again at zero would write a second `seq: 0` into a file that already
    /// has one, which breaks `/api/history?since=` and duplicates keys in the page.
    pub fn resuming(next_seq: u64) -> Arc<Self> {
        Arc::new(Self {
            video: broadcast::channel(VIDEO_CAPACITY).0,
            keyframe: RwLock::new(None),
            frame: RwLock::new(Arc::new(FrameSnapshot {
                seq: 0,
                pixels: Box::new([LcdColor::WHITE; PIXELS]),
            })),
            events: broadcast::channel(EVENT_CAPACITY).0,
            next_event_seq: AtomicU64::new(next_seq),
            status: RwLock::new(RunStatus::Booting),
            latest_status: RwLock::new(None),
            latest_plan: RwLock::new(None),
            tool_images: RwLock::new(VecDeque::new()),
            throttled_until: AtomicU64::new(0),
            usage: RwLock::new(None),
            turns: AtomicU64::new(0),
        })
    }

    /// Publish one encoded frame: the standalone keyframe describing the new state, and the delta
    /// that gets an already-connected client there.
    ///
    /// ⚠️ **The keyframe is stored first, and the order is load bearing.** A joiner subscribes and
    /// *then* reads the keyframe (see [`Self::join_video`]). If the delta went out first, a joiner
    /// that subscribed in the gap would read the *previous* keyframe and never see the delta that
    /// followed it — a permanently stale corner of the screen. Storing first makes the worst case a
    /// delta the joiner already has, which it discards by sequence number.
    pub fn publish_video(&self, keyframe: Encoded, delta: Encoded) {
        *self.keyframe.write().expect("video keyframe lock poisoned") = Some(keyframe.into());
        let _ = self.video.send(delta.into());
    }

    /// Subscribe, **then** take the keyframe to start from — never the other way round.
    ///
    /// The caller sends the keyframe, then forwards messages from the receiver, **discarding any
    /// with `seq <= keyframe.seq`**. `None` means nothing has been published yet, in which case the
    /// caller just waits for the first message, which is always a keyframe.
    pub fn join_video(&self) -> (broadcast::Receiver<VideoMessage>, Option<VideoMessage>) {
        let receiver = self.video.subscribe();
        let keyframe = self.keyframe.read().expect("video keyframe lock poisoned").clone();
        (receiver, keyframe)
    }

    /// The keyframe on its own, for a subscriber that has already lagged out of the ring buffer and
    /// needs to re-sync without dropping its connection.
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

    /// Stamp a sequence number on an event body and broadcast it. Returns the sequence number, which
    /// W7's transcript writer needs — and which the host uses to know how far a finished run's
    /// transcript has to be followed before it is archived.
    pub fn publish_event(&self, body: UiEventBody) -> u64 {
        // The fold behind `usage()`/`turns()`. See their fields for why it is here and not at the
        // call site that builds the event.
        if let UiEventBody::Decision { usage, .. } = &body {
            self.turns.fetch_add(1, Ordering::Relaxed);
            if let Some(view) = usage {
                *self.usage.write().expect("usage lock poisoned") = Some(*view);
            }
        }
        let seq = self.next_event_seq.fetch_add(1, Ordering::Relaxed);
        let event = UiEvent { seq, at: now_ms(), body };
        // Kept **before** the send, for the reason `publish_status` gives: a client that joins in
        // the gap should see a stale plan rather than none.
        if matches!(event.body, UiEventBody::Plan { .. }) {
            *self.latest_plan.write().expect("plan lock poisoned") = Some(event.clone());
        }
        let _ = self.events.send(event);
        seq
    }

    /// Keep a picture a tool answered with, under the seq of the `ToolResult` that announced it.
    ///
    /// Called by the worker immediately after publishing that event, so the seq it is filed under
    /// is the one the page will ask for. See [`Self::tool_images`] for why this is a small ring.
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
    ///
    /// ⚠️ The LLM worker rebuilds its `Accounting` on a restart, but it does so asynchronously and
    /// only reports again at its next decision — so without this, a checkpoint landing in the gap
    /// credits a run seconds old with the whole bill of the one before it. `turns` is deliberately
    /// *not* reset: the host subtracts a mark it takes at the same moment, which is the same answer
    /// without a counter that can go backwards under a concurrent reader.
    pub fn forget_usage(&self) {
        *self.usage.write().expect("usage lock poisoned") = None;
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<UiEvent> {
        self.events.subscribe()
    }

    /// Publish a heartbeat and keep it as the one a new client is handed.
    ///
    /// Stored **before** it is broadcast, for the reason [`Self::publish_video`] gives. The stakes
    /// are lower here — a status is absolute, not a delta, so the worst case of the wrong order is
    /// a viewer showing a 500 ms-old panel for 500 ms rather than a corrupted screen forever — but
    /// the ordering is free and there is no reason for the two paths to differ.
    pub fn publish_status(&self, snapshot: StatusSnapshot) -> u64 {
        let seq = self.next_event_seq.fetch_add(1, Ordering::Relaxed);
        let event = UiEvent { seq, at: now_ms(), body: UiEventBody::Status(Box::new(snapshot)) };
        *self.latest_status.write().expect("status lock poisoned") = Some(event.clone());
        let _ = self.events.send(event);
        seq
    }

    /// Subscribe, **then** take the events to open with — never the other way round.
    ///
    /// The duplicate this can produce (a client that subscribed just before the heartbeat it also
    /// reads here) is harmless in a way the video path's would not be: a status and a plan are each
    /// complete in themselves, and the browser folds each into one piece of state rather than
    /// appending it to a list.
    ///
    /// ⚠️ **Two cells, and the plan is the one that is easy to forget.** Both are published on
    /// change; the heartbeat changes every couple of seconds and the plan can go an hour, so the
    /// plan is the one where "wait for the next one" means an empty panel for the length of a
    /// viewing. Anything else that becomes send-on-change belongs here too.
    ///
    /// Returned oldest-first so the page applies them in the order they happened.
    pub fn join_events(&self) -> (broadcast::Receiver<UiEvent>, Vec<UiEvent>) {
        let receiver = self.events.subscribe();
        let mut opening: Vec<UiEvent> = [
            self.latest_plan.read().expect("plan lock poisoned").clone(),
            self.latest_status.read().expect("status lock poisoned").clone(),
        ]
        .into_iter()
        .flatten()
        .collect();
        opening.sort_by_key(|event| event.seq);
        (receiver, opening)
    }

    /// **W6 / §9** — record what the run is doing, and say so **if it changed**.
    ///
    /// The guard is not an optimisation: `Playing` is set from the emulator loop and `RunningTool`
    /// from a batch that can be answered fifty times a second, so an unguarded version would put
    /// thousands of identical events a minute into a stream a browser is reading.
    pub fn set_status(&self, status: RunStatus) {
        // Read first, because the overwhelmingly common call is a repeat — `Streaming` is set once
        // per token of a reply — and a repeat should not take the write lock at all.
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
    ///
    /// ⚠️ **An atomic rather than a read of [`Self::run_status`], because the emulator thread asks
    /// this on every tick.** `RunStatus` carries a `String` in three of its variants, so answering
    /// from it would clone one fifty times a second on the hot path to say "no" — and the run status
    /// itself is set right beside this, so the two cannot disagree about *whether* we are parked.
    pub fn set_throttled_until(&self, until_ms: u64) {
        self.throttled_until.store(until_ms, Ordering::Relaxed);
    }

    /// The moment the run may resume, if it is parked. Read by [`crate::host::EmulatorHost::tick`].
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
    use crate::web::video::{VideoDecoder, VideoEncoder};

    /// The fold behind `usage()`/`turns()`, which the emulator thread reads when it files a finished
    /// run. ⚠️ It counts **decisions that landed**: a turn that was abandoned cost tokens but did
    /// not decide anything, and a heartbeat is not a turn at all.
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

        // ⚠️ A new run must not inherit the last one's bill — the worker's own reset is
        // asynchronous, so between the swap and the next decision this cell is the only thing that
        // says whose tokens these were.
        published.forget_usage();
        assert!(published.usage().is_none());
        assert_eq!(published.turns(), before, "…and the turn counter is a mark the host subtracts, \
                                               not something that goes backwards under a reader");
    }

    /// A frame that differs from its neighbours in a handful of blocks — enough for a delta to be
    /// non-empty, which is all the ordering test needs. Codec fidelity is `video::tests`' job.
    fn frame(n: usize) -> Box<Frame> {
        let mut pixels = Box::new([LcdColor::WHITE; PIXELS]);
        for p in 0..PIXELS {
            if (p / crate::ppu::LCD_WIDTH + p % crate::ppu::LCD_WIDTH + n) % 37 == 0 {
                pixels[p] = LcdColor::rgb(n as u8, 0x20, 0x40);
            }
        }
        pixels
    }

    /// §5.2. A viewer subscribes at some arbitrary instant and reads the keyframe some time later;
    /// whatever the publisher did in between, the viewer must end up pixel-exact.
    ///
    /// The loop is over that gap, because the gap is the hazard: it is the window in which a
    /// broadcast-then-store ordering silently loses a delta.
    #[test]
    fn late_joiner_never_misses_a_delta() {
        for gap in 0..6 {
            let published = Published::new();
            let mut encoder = VideoEncoder::default();
            let mut publish = |n: usize, encoder: &mut VideoEncoder| {
                if let Some(delta) = encoder.encode(&frame(n)) {
                    published.publish_video(encoder.keyframe().expect("state exists"), delta);
                }
            };

            for n in 0..4 {
                publish(n, &mut encoder);
            }

            // The joiner subscribes here…
            let (mut receiver, keyframe) = {
                let receiver = published.video.subscribe();
                for n in 4..4 + gap {
                    publish(n, &mut encoder);
                }
                // …and only reads the keyframe `gap` frames later.
                (receiver, published.latest_keyframe())
            };
            for n in 4 + gap..12 {
                publish(n, &mut encoder);
            }

            let keyframe = keyframe.expect("something was published before the join");
            let mut decoder = VideoDecoder::default();
            decoder.apply(&keyframe.bytes).unwrap();
            let mut applied = 0;
            while let Ok(message) = receiver.try_recv() {
                if message.seq <= keyframe.seq {
                    continue; // already folded into the keyframe
                }
                decoder.apply(&message.bytes).unwrap();
                applied += 1;
            }

            assert!(applied > 0, "gap {gap}: no deltas followed the keyframe, so nothing was proved");
            assert_eq!(decoder.pixels(), frame(11).as_ref(), "gap {gap}: joiner is not pixel-exact");
        }
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

        // **W7.** A resumed process continues the transcript's numbering rather than writing a
        // second `seq: 0` into a file that already has one.
        let resumed = Published::resuming(500);
        assert_eq!(resumed.publish_event(UiEventBody::Notice { level: "info", message: "later".into() }), 500);
    }

    /// Every event is stamped with the wall clock, and the stamp survives to the wire.
    ///
    /// It is the only clock on the page that means anything across a restart — `wall_ms` and
    /// `emulated_ms` are both elapsed times *since this process started*, so a run resumed nightly
    /// reports both from zero. The browser cannot supply one either: `/api/history` replays a backlog
    /// that may be hours old, and a client-side clock would date the whole of it to the page load.
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

        // ⚠️ And it takes part in the suppression, or the first heartbeat after a change would be
        // held back for saying nothing new.
        assert!(!random.says_the_same_as(&snapshot("wait", 1)));
    }

    /// §9's status. The interesting part is the *silence*: `set_status` is called from loops that run
    /// at 50 Hz, so a repeat must not be an event.
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
            target_speed: 1.0,
            policy: "llm",
            model: Some("gpt-5".to_string()),
            agent_state: agent_state.to_string(),
            frame_seq: wall_ms / 33,
            game: None,
            run: RunStatus::Playing,
        }
    }

    /// The comparison the whole send-on-change rule rests on. ⚠️ The clocks and the frame counter
    /// must not count, or nothing is ever equal and the suppression silently never fires.
    #[test]
    fn a_heartbeat_is_the_same_as_another_when_only_the_clock_has_moved() {
        assert!(snapshot("wait", 5_000).says_the_same_as(&snapshot("wait", 100)));
        assert!(!snapshot("move→Warp", 100).says_the_same_as(&snapshot("wait", 100)));

        let mut moved = snapshot("wait", 100);
        moved.run = RunStatus::Streaming;
        assert!(!moved.says_the_same_as(&snapshot("wait", 100)), "the run status is state, not clock");
    }

    /// ⚠️ The other half of send-on-change: a page that opens while nothing is happening must not
    /// wait for something to happen. One shared cell, not one buffer per client.
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

    /// ⚠️ **The plan is the send-on-change event a reload could not recover**, and it failed
    /// silently: the panel simply was not there (`PlanPanel` renders nothing for an empty list), so
    /// it read as a styling problem rather than as a missing event.
    ///
    /// Its two delivery routes both had a hole. The stream opened with the heartbeat alone, and the
    /// backlog `/api/history` replays is capped — at a length a reasoning model, which publishes an
    /// event per streamed token, walks past in minutes. So the last `Plan` was reliably older than
    /// both windows, and the panel came back only when the model next edited its own list, which can
    /// be an hour.
    #[test]
    fn a_joiner_is_handed_the_plan_as_well_as_the_heartbeat() {
        let published = Published::new();
        let item = |id: u32, text: &str| TodoView { id, text: text.to_string(), done: false };

        published.publish_event(UiEventBody::Plan { items: vec![item(1, "get the Boulder Badge")] });
        // …a turn's worth of noise on top, which is what used to bury it.
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

    /// A picture is referenced by seq and fetched separately, so the ring is what decides whether a
    /// viewer can still open it. A miss is an ordinary answer — the page shows the caption alone.
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

    /// The wire shape, because the SPA's `api.ts` is written against it by hand: a run status is one
    /// flat object with a `state` discriminator, not a nested one.
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
