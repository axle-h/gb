//! **W1.2 / W2** — the HTTP server.
//!
//! Read-only endpoints, and **one** that is not. Viewer controls are still out of scope by decision
//! (§1.1 of `docs/llm-web-playthrough-plan.md`) — nothing here can press a button, choose a move or
//! steer the run — and for everything except `POST /api/new-run` that remains structural rather than
//! editorial: this module can reach [`published::Published`] and nothing else.
//!
//! ⚠️ **`POST /api/new-run` is the exception, and it is the only one.** It reaches the emulator
//! through [`crate::host::NewRunRequests`], which carries no data inwards and is answered by the
//! emulator thread between instructions. It is **off unless `GB_ADMIN_TOKEN` is set** — without it
//! the route 404s exactly as if it had never been registered — because the deployment this exists
//! for serves the public internet and starting the game over is not something a passer-by should be
//! able to do.
//!
//! ```text
//! GET  /                    the SPA (`web/dist`, embedded — see `assets.rs`)
//! GET  /{*path}             its assets
//! GET  /api/healthz         liveness
//! GET  /api/events          SSE — status heartbeat at 10 Hz, plus agent events as they happen
//! GET  /api/video           SSE — a keyframe, then base64 block deltas
//! GET  /api/badges.png      the eight gym badges, decoded from the cartridge
//! GET  /api/history?since=  W7 — the transcript from a sequence number, for a page that just loaded
//! POST /api/new-run         start the game over in a fresh run directory (needs GB_ADMIN_TOKEN)
//! ```

pub mod assets;
pub mod badges;
pub mod published;
pub mod video;

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use crate::cli::ServePolicy;
use crate::game_boy::GameBoy;
use crate::host::{EmulatorHost, HostConfig, NewRunRequests};
use crate::pokemon::policy::RandomPolicy;
use crate::run::{CurrentRun, Origin, RunDir, transcript};
use published::{Published, VideoMessage};

/// Proxies close an idle connection; a comment every two seconds stops that without costing
/// anything a client has to parse.
const KEEP_ALIVE: Duration = Duration::from_secs(2);

/// The header `POST /api/new-run` reads its token from.
const ADMIN_TOKEN_HEADER: &str = "x-gb-token";

/// How long the handler waits for the emulator thread to act. It answers at the top of its next
/// tick, which is a millisecond away — so reaching this at all means the thread is gone, and the
/// only useful thing left to do is say so rather than hold the connection open.
const NEW_RUN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct AppState {
    published: Arc<Published>,
    started: Instant,
    /// **W7** — the run directory, read through rather than copied out, because `POST /api/new-run`
    /// can change it under a live server. `/api/history` and `/api/healthz` both ask it per request.
    run: Arc<CurrentRun>,
    /// The seam into the emulator thread. See [`NewRunRequests`].
    new_runs: Arc<NewRunRequests>,
    /// `GB_ADMIN_TOKEN`. `None` — the default — makes `POST /api/new-run` 404.
    admin_token: Option<String>,
}

/// `gb serve`. Blocks until the process is interrupted.
///
/// Two threads: this one becomes the tokio runtime serving HTTP, and a plain `std::thread` runs the
/// emulator. The runtime never touches the emulator and the emulator never enters the runtime.
pub fn run(port: u16, policy: ServePolicy, new_run: bool) -> Result<(), String> {
    let shutdown = Arc::new(AtomicBool::new(false));

    // The LLM configuration is read first, because a missing API key should be an error before a
    // run directory is created for a run that cannot start.
    #[cfg(feature = "llm")]
    let llm = match policy {
        ServePolicy::Llm => Some(crate::llm::LlmConfig::from_env()?),
        ServePolicy::Random => None,
    };
    #[cfg(not(feature = "llm"))]
    let llm: Option<()> = None;

    let model = match policy {
        ServePolicy::Random => "random".to_string(),
        #[cfg(feature = "llm")]
        ServePolicy::Llm => llm.as_ref().expect("built above").model.clone(),
        #[cfg(not(feature = "llm"))]
        ServePolicy::Llm => {
            return Err("this build has no LLM — it was built without the `llm` feature. \
                        `--policy random` plays now."
                .to_string());
        }
    };

    // **W7 / §11.** `GameBoy::load_state` applies to a clone, so a state that does not load leaves
    // nothing behind — which is what makes it safe to use as the validity test for a resume.
    let root = std::env::var("GB_RUN_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(crate::run::DEFAULT_ROOT));
    let (run, origin, resumed) = RunDir::open(&root, new_run, &model, &|bytes| {
        GameBoy::dmg(crate::pokemon::roms::POKERED).load_state(bytes).is_ok()
    })?;
    // From here on nothing holds the `RunDir` directly: `POST /api/new-run` can replace it under a
    // live process, and the checkpointer, the transcript thread, `/api/history` and the model's
    // notes all have to move together when it does.
    let current = Arc::new(CurrentRun::new(root, model.clone(), run));
    let run = current.get();
    println!(
        "gb serve — {} run {} in {}",
        match origin {
            Origin::Fresh => "new",
            Origin::Resumed => "resuming",
        },
        run.run_id(),
        run.path().display(),
    );
    let starting_state = resumed.unwrap_or_else(|| crate::pokemon::data::START_OF_GAME.to_vec());

    // The transcript writer is started before the emulator, so the first event of the run is in it —
    // and the event counter continues from where the last process left off, which is what makes
    // `/api/history?since=` mean anything across a restart.
    let transcript_path = run.transcript_path();
    let published = Published::resuming(transcript::last_seq(&transcript_path).map_or(0, |seq| seq + 1));
    let transcript =
        transcript::spawn(Arc::clone(&current), Arc::clone(&published), Arc::clone(&shutdown))?;
    // Published *after* the writer is subscribed, so it lands in the transcript: it is the marker
    // that explains why turn numbers start again from 1 below it, the generation counter being
    // per-process.
    published.publish_event(published::UiEventBody::Notice {
        level: "info",
        message: match origin {
            Origin::Fresh => format!("new run {} — from the beginning of the game", run.run_id()),
            Origin::Resumed => format!("resumed run {} from its last checkpoint", run.run_id()),
        },
    });

    // The policy is a **factory**, built on the emulator thread — `Policy` is not `Send` and
    // `LlmPolicy` owns channel endpoints. The pieces it needs are assembled here, where a bad
    // configuration is still a clean error before anything is listening.
    let make_policy: Box<dyn FnOnce() -> Box<dyn crate::pokemon::policy::Policy> + Send> = match policy
    {
        ServePolicy::Random => Box::new(|| Box::new(RandomPolicy::default())),
        #[cfg(feature = "llm")]
        ServePolicy::Llm => {
            use crate::llm::{client::OpenAiClient, notes::Notes, worker};
            use crate::pokemon::llm_policy::LlmPolicy;

            let config = llm.expect("built above");
            println!("gb serve — {} via {}", config.model, config.base_url);
            let endpoint = Box::new(OpenAiClient::new(&config));
            // **W9** — read off before `config` is moved into the worker. The watchdog belongs to
            // the policy (it is what the agent asks how long to wait), not to the turn loop.
            let stuck_timeout = config.stuck_timeout;
            // **W6b** — the model's own notes live in the run directory, so they survive both a
            // compaction and a restart.
            let notes = Notes::open(Some(run.path()));
            let (worker, handles) =
                worker::channels(endpoint, config, Arc::clone(&published), notes);
            // The worker outlives this function; it ends when the policy is dropped and its channels
            // close, which happens when the emulator thread stops.
            worker.spawn()?;
            Box::new(move || Box::new(LlmPolicy::new(handles, stuck_timeout)))
        }
        #[cfg(not(feature = "llm"))]
        ServePolicy::Llm => unreachable!("rejected above"),
    };

    let new_runs = Arc::new(NewRunRequests::default());
    let admin_token = admin_token();
    let emulator = EmulatorHost::spawn(
        starting_state,
        make_policy,
        Arc::clone(&published),
        HostConfig {
            policy_name: policy_name(policy),
            run: Some(Arc::clone(&current)),
            new_runs: Some(Arc::clone(&new_runs)),
            status_interval: status_interval()?,
            ..HostConfig::default()
        },
        Arc::clone(&shutdown),
    )?;

    println!(
        "gb serve — POST /api/new-run is {}",
        match admin_token {
            Some(_) => "enabled (GB_ADMIN_TOKEN is set)",
            None => "off — set GB_ADMIN_TOKEN to enable it",
        },
    );
    let result = serve_http(port, Arc::clone(&published), current, new_runs, admin_token);

    // ⚠️ The order matters: the emulator's last act is a checkpoint (`EmulatorHost::run`), so it is
    // joined *before* the process is allowed to end. The transcript thread is woken by the next
    // event after `shutdown`, which the checkpoint's own notices provide; it is not waited on
    // indefinitely, because a run with nothing left to say would never wake it.
    shutdown.store(true, Ordering::Relaxed);
    let _ = emulator.join();
    published.publish_event(published::UiEventBody::Notice {
        level: "info",
        message: "the run stopped cleanly".to_string(),
    });
    let _ = transcript.join();
    result
}

/// How often the game state is sampled for the heartbeat, from `GB_STATUS_HZ`.
///
/// An environment variable rather than a flag, for the reason `GB_RUN_DIR` is one: it is deployment
/// configuration, and it applies to `--policy random` too. The default of 2 Hz is what the panel
/// needs; the knob exists because "what a viewer needs" is a judgement, and someone watching the
/// agent's state machine step through a menu may reasonably want 10.
fn status_interval() -> Result<Duration, String> {
    let Some(value) = std::env::var("GB_STATUS_HZ").ok().filter(|value| !value.trim().is_empty()) else {
        return Ok(HostConfig::default().status_interval);
    };
    match value.trim().parse::<f64>() {
        Ok(hz) if (0.1..=60.0).contains(&hz) => Ok(Duration::from_secs_f64(1.0 / hz)),
        _ => Err(format!("`GB_STATUS_HZ={value}` is not a rate between 0.1 and 60")),
    }
}

fn policy_name(policy: ServePolicy) -> &'static str {
    match policy {
        ServePolicy::Random => "random",
        ServePolicy::Llm => "llm",
    }
}

fn serve_http(
    port: u16,
    published: Arc<Published>,
    run: Arc<CurrentRun>,
    new_runs: Arc<NewRunRequests>,
    admin_token: Option<String>,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("could not start the HTTP runtime: {e}"))?;

    runtime.block_on(async move {
        let state = AppState { published, started: Instant::now(), run, new_runs, admin_token };
        let app = Router::new()
            .route("/api/healthz", get(healthz))
            .route("/api/events", get(events))
            .route("/api/history", get(history))
            .route("/api/video", get(video_stream))
            .route("/api/badges.png", get(badges::badges))
            .route("/api/new-run", post(new_run))
            // Last, so the catch-all cannot shadow an API route it happens to match.
            .route("/", get(assets::index))
            .route("/{*path}", get(assets::asset))
            .with_state(state);

        // 0.0.0.0: the container publishes the port. Every endpoint is read-only except
        // `POST /api/new-run`, which is off unless `GB_ADMIN_TOKEN` is set and needs it when it is.
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
            .await
            .map_err(|e| format!("could not bind port {port}: {e}"))?;
        println!("gb serve — http://localhost:{port}");

        axum::serve(listener, app)
            // ⚠️ **SIGTERM as well as Ctrl-C.** `docker stop` sends the former, and a container that
            // only handled the latter would lose every checkpoint-worth of play since the last
            // periodic write — up to a minute — on every deploy.
            .with_graceful_shutdown(async {
                shutdown_signal().await;
                println!("shutting down — checkpointing");
            })
            .await
            .map_err(|e| format!("server failed: {e}"))
    })
}

/// Whatever the supervisor uses to ask for a clean stop.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            // Nothing to be done about it, and it must not stop the server starting.
            Err(_) => return tokio::signal::ctrl_c().await.map(|_| ()).unwrap_or(()),
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

async fn healthz(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "uptime_ms": state.started.elapsed().as_millis() as u64,
        "video_seq": state.published.latest_keyframe().map(|k| k.seq),
        // Read per request rather than captured: `POST /api/new-run` changes it, and a liveness
        // endpoint reporting the run the process *started* with would be quietly wrong afterwards.
        "run_id": state.run.get().run_id(),
    }))
}

/// `GB_ADMIN_TOKEN`, or `None` if it is unset or blank.
///
/// An environment variable for the reason `GB_RUN_DIR` and `GB_STATUS_HZ` are: it is deployment
/// configuration. Blank counts as unset so that a Kubernetes Secret with an empty value — the shape
/// a placeholder usually takes — leaves the endpoint off rather than open to the empty string.
fn admin_token() -> Option<String> {
    std::env::var("GB_ADMIN_TOKEN").ok().map(|token| token.trim().to_string()).filter(|token| !token.is_empty())
}

/// Compare without an early return, so the time taken says nothing about how much of the token was
/// right. Overkill for a token nobody is going to grind over the internet, and cheaper than deciding
/// that on a destructive endpoint reachable from it.
fn tokens_match(offered: &str, expected: &str) -> bool {
    // `expected` is this server's own configuration rather than anything a caller sent, so returning
    // early on it leaks nothing — and it is what keeps the index below in bounds.
    if expected.is_empty() {
        return false;
    }
    let (offered, expected) = (offered.as_bytes(), expected.as_bytes());
    let mut difference = offered.len() ^ expected.len();
    for (index, byte) in offered.iter().enumerate() {
        // Index into `expected` cyclically, so a wrong *length* still walks the whole offered token.
        difference |= (byte ^ expected[index % expected.len()]) as usize;
    }
    difference == 0
}

/// **Start the game over, in place**, and answer with the id of the run that just began.
///
/// The one endpoint here that is not read-only, and the whole reason
/// [`NewRunRequests`] exists. What it is *not* is a general control channel: it carries no data
/// inwards, and the emulator thread decides when to act on it.
///
/// ⚠️ **404, not 403, when `GB_ADMIN_TOKEN` is unset.** The route is registered unconditionally
/// because the router is built before the token is a question, so "off" has to be a response — and
/// it should be the response a route that does not exist would give, rather than one advertising a
/// reset endpoint to anyone who scans for it.
///
/// The old run is not deleted, or even stopped in any sense that loses anything: the emulator
/// checkpoints it before swapping, so it is left complete on the volume and can be resumed by
/// pointing a process back at it.
async fn new_run(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(expected) = state.admin_token.as_deref() else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({
            "error": "this server has no GB_ADMIN_TOKEN set, so starting a new run is disabled",
        }))).into_response();
    };
    let offered = headers.get(ADMIN_TOKEN_HEADER).and_then(|value| value.to_str().ok()).unwrap_or_default();
    if !tokens_match(offered, expected) {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({
            "error": format!("a matching {ADMIN_TOKEN_HEADER} header is required"),
        }))).into_response();
    }

    let receiver = match state.new_runs.request() {
        Ok(receiver) => receiver,
        Err(failure) => {
            return (StatusCode::CONFLICT, Json(serde_json::json!({ "error": failure }))).into_response();
        }
    };
    match tokio::time::timeout(NEW_RUN_TIMEOUT, receiver).await {
        Ok(Ok(Ok(run_id))) => (StatusCode::OK, Json(serde_json::json!({ "run_id": run_id }))).into_response(),
        // The emulator tried and could not — a full disk, most likely. Its message is the useful one.
        Ok(Ok(Err(failure))) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "error": failure,
        }))).into_response(),
        // The sender was dropped, or never taken: either way the emulator thread is not running, and
        // `Obituary` will have said so on `/api/events` already.
        Ok(Err(_)) | Err(_) => (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
            "error": "the emulator thread did not answer — see /api/events",
        }))).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct Since {
    #[serde(default)]
    since: u64,
}

/// **W7 / §11** — the backlog, so a page that has just loaded shows the run it joined rather than an
/// empty log until the next thing happens.
///
/// ⚠️ **The client subscribes to `/api/events` first and calls this second**, exactly as the video
/// path does (§5.2). The other order loses everything published in the gap, and loses it invisibly.
/// Reading the file happens on a blocking thread: it is up to a couple of megabytes and the runtime
/// threads are also serving two SSE streams.
async fn history(State(state): State<AppState>, Query(query): Query<Since>) -> Json<serde_json::Value> {
    let path = state.run.get().transcript_path();
    let events = tokio::task::spawn_blocking(move || transcript::read_since(&path, query.since))
        .await
        .unwrap_or_default();
    Json(serde_json::Value::Array(events))
}

/// The conversation and status stream. One JSON object per message, exactly the shape W7 appends to
/// `transcript.jsonl`.
///
/// ⚠️ **It opens with the most recent heartbeat**, because the host only sends one when the status
/// has actually changed. Subscribe first, then read the latest — [`Published::join_events`], the
/// same ordering and the same reason as the video keyframe (§5.2). Without it a page opened while
/// the game is standing still shows an empty status panel until something moves.
async fn events(State(state): State<AppState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (receiver, latest) = state.published.join_events();
    let opening = tokio_stream::iter(latest.into_iter().map(sse_event));
    let live = BroadcastStream::new(receiver).filter_map(|item| {
        // A lagged client has missed events it cannot recover here; W7's `/api/history?since=` is
        // where it catches up. Dropping the notification is the right call — the alternative is
        // tearing down a connection that is otherwise working.
        Some(sse_event(item.ok()?))
    });
    Sse::new(opening.chain(live)).keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
}

fn sse_event(event: published::UiEvent) -> Result<Event, Infallible> {
    Ok(Event::default().json_data(event).expect("UiEvent serialises"))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ **The endpoint is off unless a token is set, and blank is not set.** A Kubernetes Secret
    /// with a placeholder value is usually the empty string, and the failure mode of getting this
    /// wrong is a reset endpoint open to the internet that looks configured.
    #[test]
    fn a_blank_admin_token_leaves_the_endpoint_off() {
        assert!(!tokens_match("", ""), "the empty token must never match, least of all itself");
        assert!(!tokens_match("anything", ""));
    }

    #[test]
    fn a_token_matches_only_itself() {
        assert!(tokens_match("s3cret", "s3cret"));
        assert!(!tokens_match("s3cret", "s3crets"), "a prefix is not a match");
        assert!(!tokens_match("s3crets", "s3cret"), "nor is an extension");
        assert!(!tokens_match("S3CRET", "s3cret"), "and it is case sensitive");
        assert!(!tokens_match("", "s3cret"), "nor is sending no header at all");
    }
}
