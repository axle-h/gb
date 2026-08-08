//! **W1.2 / W2** — the HTTP server.
//!
//! Four read-only endpoints and no write endpoints at all. Viewer controls are out of scope by
//! decision (§1.1 of `docs/llm-web-playthrough-plan.md`), and the way that decision is enforced is
//! structural rather than editorial: this module can reach [`published::Published`] and nothing else.
//! There is no channel from here back into the emulator to expose.
//!
//! ```text
//! GET /                the SPA (`web/dist`, embedded — see `assets.rs`)
//! GET /{*path}         its assets
//! GET /api/healthz     liveness
//! GET /api/events      SSE — status heartbeat at 10 Hz, plus agent events as they happen
//! GET /api/video       SSE — a keyframe, then base64 block deltas
//! GET /api/badges.png  the eight gym badges, decoded from the cartridge
//! ```

pub mod assets;
pub mod badges;
pub mod published;
pub mod video;

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Json;
use axum::routing::get;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use crate::cli::ServePolicy;
use crate::host::{EmulatorHost, HostConfig};
use crate::pokemon::policy::RandomPolicy;
use published::{Published, VideoMessage};

/// Proxies close an idle connection; a comment every two seconds stops that without costing
/// anything a client has to parse.
const KEEP_ALIVE: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct AppState {
    published: Arc<Published>,
    started: Instant,
}

/// `gb serve`. Blocks until the process is interrupted.
///
/// Two threads: this one becomes the tokio runtime serving HTTP, and a plain `std::thread` runs the
/// emulator. The runtime never touches the emulator and the emulator never enters the runtime.
pub fn run(port: u16, policy: ServePolicy) -> Result<(), String> {
    let make_policy: Box<dyn FnOnce() -> Box<dyn crate::pokemon::policy::Policy> + Send> = match policy
    {
        ServePolicy::Random => Box::new(|| Box::new(RandomPolicy)),
        // W4 builds this. Failing here beats starting a server that would sit at a decision point
        // forever with nothing to answer it.
        ServePolicy::Llm => {
            return Err("`--policy llm` is not built yet — it arrives with phase W4 of \
                        docs/llm-web-playthrough-plan.md. `--policy random` plays now."
                .to_string());
        }
    };

    let published = Published::new();
    let shutdown = Arc::new(AtomicBool::new(false));

    let emulator = EmulatorHost::spawn(
        crate::pokemon::data::START_OF_GAME,
        make_policy,
        Arc::clone(&published),
        HostConfig { policy_name: policy_name(policy), ..HostConfig::default() },
        Arc::clone(&shutdown),
    )?;

    let result = serve_http(port, Arc::clone(&published));

    shutdown.store(true, Ordering::Relaxed);
    let _ = emulator.join();
    result
}

fn policy_name(policy: ServePolicy) -> &'static str {
    match policy {
        ServePolicy::Random => "random",
        ServePolicy::Llm => "llm",
    }
}

fn serve_http(port: u16, published: Arc<Published>) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("could not start the HTTP runtime: {e}"))?;

    runtime.block_on(async move {
        let state = AppState { published, started: Instant::now() };
        let app = Router::new()
            .route("/api/healthz", get(healthz))
            .route("/api/events", get(events))
            .route("/api/video", get(video_stream))
            .route("/api/badges.png", get(badges::badges))
            // Last, so the catch-all cannot shadow an API route it happens to match.
            .route("/", get(assets::index))
            .route("/{*path}", get(assets::asset))
            .with_state(state);

        // 0.0.0.0: the container publishes the port, and there is nothing here worth binding to
        // loopback for — every endpoint is read-only.
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
            .await
            .map_err(|e| format!("could not bind port {port}: {e}"))?;
        println!("gb serve — http://localhost:{port}");

        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
                println!("shutting down");
            })
            .await
            .map_err(|e| format!("server failed: {e}"))
    })
}

async fn healthz(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "uptime_ms": state.started.elapsed().as_millis() as u64,
        "video_seq": state.published.latest_keyframe().map(|k| k.seq),
    }))
}

/// The conversation and status stream. One JSON object per message, exactly the shape W7 appends to
/// `transcript.jsonl`.
async fn events(State(state): State<AppState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(state.published.subscribe_events()).filter_map(|item| {
        // A lagged client has missed events it cannot recover here; W7's `/api/history?since=` is
        // where it catches up. Dropping the notification is the right call — the alternative is
        // tearing down a connection that is otherwise working.
        let event = item.ok()?;
        Some(Ok(Event::default().json_data(event).expect("UiEvent serialises")))
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
}

/// The video stream, with §5.2's handshake: **subscribe first**, then take the keyframe, then
/// forward deltas newer than it.
///
/// [`Published::join_video`] does the first two in that order and
/// [`Published::publish_video`] stores the keyframe before broadcasting the delta, so the worst case
/// here is a delta the keyframe already contains — which `seq` filters out. The opposite ordering
/// loses a delta outright, and the loss is invisible: a stale eighth of the screen that never
/// repairs.
async fn video_stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (receiver, keyframe) = state.published.join_video();
    let mut floor = keyframe.as_ref().map_or(0, |k| k.seq);
    let published = Arc::clone(&state.published);

    let opening = tokio_stream::iter(keyframe.into_iter().map(sse_video));
    let live = BroadcastStream::new(receiver).filter_map(move |item| match item {
        Ok(message) if message.seq > floor => {
            floor = message.seq;
            Some(sse_video(message))
        }
        // Already covered by the keyframe this connection opened with.
        Ok(_) => None,
        // The client fell out of the ring buffer, so its palette and its screen are both suspect.
        // A fresh keyframe repairs both; dropping the connection would work too, but re-syncing in
        // place is invisible to the viewer.
        Err(BroadcastStreamRecvError::Lagged(_)) => published.latest_keyframe().map(|keyframe| {
            floor = keyframe.seq;
            sse_video(keyframe)
        }),
    });

    Sse::new(opening.chain(live)).keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
}

fn sse_video(message: VideoMessage) -> Result<Event, Infallible> {
    Ok(Event::default().data(&*message.data))
}
