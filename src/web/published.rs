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
}

/// Context occupancy after a turn. The full accounting — cumulative totals, a cost estimate,
/// estimated-vs-reported — is W6's; this is the one number the conversation pane can show now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct UsageView {
    pub context_tokens: u64,
    pub context_limit: u64,
}

/// What the status panel renders, and the cheapest thing the host can publish at 10 Hz.
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
}

impl Published {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            video: broadcast::channel(VIDEO_CAPACITY).0,
            keyframe: RwLock::new(None),
            frame: RwLock::new(Arc::new(FrameSnapshot {
                seq: 0,
                pixels: Box::new([LcdColor::WHITE; PIXELS]),
            })),
            events: broadcast::channel(EVENT_CAPACITY).0,
            next_event_seq: AtomicU64::new(0),
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
    }
}
