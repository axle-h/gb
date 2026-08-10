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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use base64::Engine;
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

/// One video message, base64'd **once** for every subscriber rather than per connection.
#[derive(Debug, Clone)]
pub struct VideoMessage {
    /// Unwrapped, unlike the `u16` on the wire — a late joiner compares these to decide what to
    /// discard and that comparison is wrong across a wrap (~36 minutes at 30 fps).
    pub seq: u64,
    pub keyframe: bool,
    pub data: Arc<str>,
}

impl From<Encoded> for VideoMessage {
    fn from(encoded: Encoded) -> Self {
        Self {
            seq: encoded.seq,
            keyframe: encoded.keyframe,
            data: base64::engine::general_purpose::STANDARD.encode(&encoded.bytes).into(),
        }
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
    #[serde(flatten)]
    pub body: UiEventBody,
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
    /// A tool the model called. `arguments` is the raw JSON string it sent.
    ToolCall { turn: u64, name: String, arguments: String },
    /// The terminal call that ended the turn.
    Decision { turn: u64, summary: String, usage: Option<UsageView> },
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
    /// The last turn could not be completed. Left in place until the next turn starts, because a
    /// status that flicks straight back to `Playing` is a status nobody ever sees.
    Error { message: String },
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
        let Self { wall_ms: _, emulated_ms: _, frame_seq: _, target_speed, policy, agent_state, game, run } = self;
        target_speed == &previous.target_speed
            && policy == &previous.policy
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
    /// The most recent heartbeat, for a client that has just connected.
    ///
    /// ⚠️ **This is what makes send-on-change safe, and it is one cell rather than one per client.**
    /// Once the host stops resending an unchanged status, a viewer who connects during a quiet
    /// stretch would otherwise stare at an empty panel until something moved. Same shape as the
    /// video keyframe (§5.2): subscribe, then read the latest.
    latest_status: RwLock<Option<UiEvent>>,
}

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
    /// W7's transcript writer needs.
    pub fn publish_event(&self, body: UiEventBody) -> u64 {
        let seq = self.next_event_seq.fetch_add(1, Ordering::Relaxed);
        let _ = self.events.send(UiEvent { seq, body });
        seq
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
        let event = UiEvent { seq, body: UiEventBody::Status(Box::new(snapshot)) };
        *self.latest_status.write().expect("status lock poisoned") = Some(event.clone());
        let _ = self.events.send(event);
        seq
    }

    /// Subscribe, **then** take the heartbeat to open with — never the other way round.
    ///
    /// The duplicate this can produce (a client that subscribed just before the heartbeat it also
    /// reads here) is harmless in a way the video path's would not be: every status is complete in
    /// itself, and the browser folds it into one piece of state rather than appending it to a list.
    pub fn join_events(&self) -> (broadcast::Receiver<UiEvent>, Option<UiEvent>) {
        let receiver = self.events.subscribe();
        let latest = self.latest_status.read().expect("status lock poisoned").clone();
        (receiver, latest)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::video::{VideoDecoder, VideoEncoder};

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
            decoder.apply(&decode64(&keyframe.data)).unwrap();
            let mut applied = 0;
            while let Ok(message) = receiver.try_recv() {
                if message.seq <= keyframe.seq {
                    continue; // already folded into the keyframe
                }
                decoder.apply(&decode64(&message.data)).unwrap();
                applied += 1;
            }

            assert!(applied > 0, "gap {gap}: no deltas followed the keyframe, so nothing was proved");
            assert_eq!(decoder.pixels(), frame(11).as_ref(), "gap {gap}: joiner is not pixel-exact");
        }
    }

    fn decode64(data: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD.decode(data).expect("we produced this")
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
        assert!(published.join_events().1.is_none(), "nothing has been published yet");

        published.publish_status(snapshot("wait", 100));
        published.publish_status(snapshot("move→Warp", 600));

        let (mut receiver, latest) = published.join_events();
        let latest = latest.expect("the joiner opens with the most recent one");
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

    /// The wire shape, because the SPA's `api.ts` is written against it by hand: a run status is one
    /// flat object with a `state` discriminator, not a nested one.
    #[test]
    fn a_run_status_serialises_flat_with_a_state_discriminator() {
        let json = serde_json::to_value(UiEvent {
            seq: 7,
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
