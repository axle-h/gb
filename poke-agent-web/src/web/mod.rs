//! ```text
//! GET  /                            the SPA (`web/dist`, embedded — see `assets.rs`)
//! GET  /{*path}                     its assets
//! GET  /favicon.png, /favicon.ico   the overworld Poké Ball, decoded from the cartridge
//! GET  /reset-game                  start the game over; HTTP Basic (needs GB_ADMIN_TOKEN)
//! GET  /version                     which build this is: crate version, build date, branch, commit
//! GET  /api/healthz                 liveness
//! GET  /api/events                  SSE — the status heartbeat, plus agent events as they happen
//! GET  /api/video                   binary — a keyframe, then block deltas, deflated per connection
//! GET  /api/audio                   binary — a header, then raw Opus packets; **not** compressed
//! GET  /api/badges.png              the eight gym badges, decoded from the cartridge
//! GET  /api/pokemon/{dex}/front.png one Pokémon's front sprite, decompressed from the cartridge
//! GET  /api/tool-image/{seq}/image.png  the picture a tool answered with, while it is still held
//! GET  /api/history?since=          the transcript from a sequence number, for a fresh page
//! GET  /api/leaderboard?limit=      the runs that have finished the game, fastest first
//! POST /api/new-run                 the same reset, for a script; `X-GB-Token`
//! POST /api/clear                   throw away the model's conversation and plan; `X-GB-Token`
//! ```

pub mod assets;
pub mod audio;
pub mod badges;
pub mod leaderboard;
pub mod sprites;
pub mod version;
pub mod video;

use std::convert::Infallible;
use std::future::IntoFuture;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::{BroadcastStream, IntervalStream};
use tokio_stream::{Stream, StreamExt};

use crate::cli::ServePolicy;
use gb::game_boy::GameBoy;
use crate::host::{ControlRequest, ControlRequests, EmulatorHost, HostConfig};
use gb::model::Model;
use poke_agent::pokemon::policy::RandomPolicy;
use poke_agent::run::{CurrentRun, Origin, RunDir, transcript};
use poke_agent::published::{self, Published};

/// Proxies close an idle connection.
const KEEP_ALIVE: Duration = Duration::from_secs(2);

/// How long the runtime is given to stop once the accept loop has been dropped.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);

/// The seed `--policy deterministic` plays under.
const SCRIPTED_SEED: u64 = 42;

/// The header the two JSON admin routes read their token from.
const ADMIN_TOKEN_HEADER: &str = "x-gb-token";

/// How long the handler waits for the emulator thread to act.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) struct AppState {
    published: Arc<Published>,
    started: Instant,
    /// The run directory, read through rather than copied out, because `POST /api/new-run` can
    /// change it under a live server.
    run: Arc<CurrentRun>,
    /// The seam into the emulator thread.
    control: Arc<ControlRequests>,
    /// `GB_ADMIN_TOKEN`.
    admin_token: Option<String>,
    /// The header every `/api/audio` connection opens with, or `None` when audio is off.
    audio: Option<[u8; audio::HEADER_LEN]>,
}

/// `gb serve`. Blocks until the process is interrupted.
pub fn run(port: u16, policy: ServePolicy, new_run: bool) -> Result<(), String> {
    let shutdown = Arc::new(AtomicBool::new(false));

    // The LLM configuration is read first, because a missing API key should be an error before a
    // run directory is created for a run that cannot start.
    let llm = match policy {
        ServePolicy::Llm => Some(poke_agent::llm::LlmConfig::from_env()?),
        ServePolicy::Random | ServePolicy::Deterministic => None,
    };
    // `RunMeta::model` is the *policy's* name under anything but `--policy llm`, and it is a
    // model id only there.
    let model = match policy {
        ServePolicy::Random => "random".to_string(),
        ServePolicy::Deterministic => "scripted".to_string(),
        ServePolicy::Llm => llm.as_ref().expect("built above").model.clone(),
    };

    let root = std::env::var("GB_RUN_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(poke_agent::run::DEFAULT_ROOT));
    let (run, origin, resumed) = RunDir::open(&root, new_run, &model, &|bytes| {
        GameBoy::dmg(poke_agent::pokemon::roms::POKERED).load_state(bytes).is_ok()
    })?;
    // From here on nothing holds the `RunDir` directly: `POST /api/new-run` can replace it under
    // a live process, and the checkpointer, the transcript thread, `/api/history` and the model's
    // notes all have to move together when it does.
    let current = Arc::new(CurrentRun::new(root, model.clone(), run));
    let run = current.get();
    println!(
        "gb serve {} — {} run {} in {}",
        // The same four facts `/version` serves: the first question asked of a container that is
        // behaving oddly is which build it is, and its log is the first place anyone looks.
        version::BuildInfo::current().summary(),
        match origin {
            Origin::Fresh => "new",
            Origin::Resumed => "resuming",
        },
        run.run_id(),
        run.path().display(),
    );
    let starting_state = resumed.unwrap_or_else(|| poke_agent::pokemon::data::START_OF_GAME.to_vec());

    // The transcript writer is started before the emulator, so the first event of the run is in
    // it — and the event counter continues from where the last process left off, which is what
    // makes `/api/history?since=` mean anything across a restart.
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
            Origin::Fresh => format!("new run {}, from the beginning of the game", run.run_id()),
            Origin::Resumed => format!("resumed run {} from its last checkpoint", run.run_id()),
        },
    });

    // The policy is a factory, built on the emulator thread — `Policy` is not `Send` and
    // `LlmPolicy` owns channel endpoints.
    let make_policy: Box<dyn FnOnce() -> Box<dyn poke_agent::pokemon::policy::Policy> + Send> = match policy
    {
        ServePolicy::Random => Box::new(|| Box::new(RandomPolicy::default())),
        // The same policy, the same seed and the same queue `full_playthrough` runs — it is that
        // test with the page in front of it rather than a second route that could disagree with
        // it.
        ServePolicy::Deterministic => {
            let run_dir = run.path().to_path_buf();
            // Only this knows a cursorless run from a new one, and `resuming_in` parks rather
            // than guessing — see its doc comment for the rollout that made the difference.
            let from_the_beginning = matches!(origin, Origin::Fresh);
            Box::new(move || {
                use poke_agent::pokemon::policy::{DeterministicPolicy, PolicyStep};
                Box::new(DeterministicPolicy::new(SCRIPTED_SEED, PolicyStep::complete_game_steps())
                    .resuming_in(&run_dir, from_the_beginning))
            })
        }
        ServePolicy::Llm => {
            use poke_agent::llm::{client::OpenAiClient, todo::TodoList, worker};
            use poke_agent::pokemon::llm_policy::LlmPolicy;

            let config = llm.expect("built above");
            println!("gb serve — {} via {}", config.model, config.base_url);
            let endpoint = Box::new(OpenAiClient::new(&config));
            // Read off before `config` is moved into the worker.
            let stuck_timeout = config.stuck_timeout;
            // The model's own plan lives in the run directory, so it survives both a compaction
            // and a restart.
            let todo = TodoList::open(Some(run.path()));
            // The battle script, beside the plan and for the same reason: it is a decision about
            // how to play that has to survive both a compaction and a restart.
            let battle_script = poke_agent::llm::battle_script::BattleScript::open(Some(run.path()));
            // The conversation itself, beside the plan and for the same reason.
            let history = poke_agent::llm::history::History::open(Some(run.path()));
            if let Some(restored) = history.restored() {
                published.publish_event(published::UiEventBody::Notice {
                    level: "info",
                    message: format!("resumed the conversation: {} messages", restored.messages),
                });
                // A changed system prompt is a `warn`, not an `info`, and it is deliberately
                // loud.
                if restored.system_prompt_changed {
                    published.publish_event(published::UiEventBody::Notice {
                        level: "warn",
                        message: "the system prompt changed since this conversation was saved; \
                                  the new one is now in force"
                            .to_string(),
                    });
                }
            }
            // `with_run` rather than a constructor argument: it is the one thing the worker does
            // that is not part of answering a turn, and the records go to whichever run is
            // current when the press happens rather than to this one — see `llm::incident`.
            let (worker, handles) = worker::channels(endpoint, config, Arc::clone(&published), todo, battle_script, history);
            let worker = worker.with_run(Arc::clone(&current));
            // The worker outlives this function; it ends when the policy is dropped and its
            // channels close, which happens when the emulator thread stops.
            worker.spawn()?;
            Box::new(move || Box::new(LlmPolicy::new(handles, stuck_timeout)))
        }
    };

    let control = Arc::new(ControlRequests::default());
    let admin_token = admin_token();
    let audio_bitrate = audio_bitrate(std::env::var("GB_AUDIO_BITRATE").ok().as_deref())?;
    let emulator = EmulatorHost::spawn(
        starting_state,
        make_policy,
        Arc::clone(&published),
        HostConfig {
            run: Some(Arc::clone(&current)),
            control: Some(Arc::clone(&control)),
            status_interval: status_interval()?,
            model: hardware_model(std::env::var("GB_HARDWARE").ok().as_deref())?,
            audio_bitrate,
            // A resume must keep the trainer it already has; only a game starting from
            // `START_OF_GAME` is named after whoever is about to play it.
            fresh_game: matches!(origin, Origin::Fresh),
            ..HostConfig::default()
        },
        Arc::clone(&shutdown),
    )?;

    println!(
        "gb serve — the admin routes (/reset-game, POST /api/new-run, POST /api/clear) are {}",
        match admin_token {
            Some(_) => "enabled (GB_ADMIN_TOKEN is set)",
            None => "off — set GB_ADMIN_TOKEN to enable them",
        },
    );
    let result = serve_http(
        port,
        Arc::clone(&published),
        current,
        control,
        admin_token,
        audio_bitrate.map(|_| audio::header()),
    );

    // The order matters: the emulator's last act is a checkpoint (`EmulatorHost::run`), so it is
    // joined *before* the process is allowed to end.
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
fn status_interval() -> Result<Duration, String> {
    let Some(value) = std::env::var("GB_STATUS_HZ").ok().filter(|value| !value.trim().is_empty()) else {
        return Ok(HostConfig::default().status_interval);
    };
    match value.trim().parse::<f64>() {
        Ok(hz) if (0.1..=60.0).contains(&hz) => Ok(Duration::from_secs_f64(1.0 / hz)),
        _ => Err(format!("`GB_STATUS_HZ={value}` is not a rate between 0.1 and 60")),
    }
}

/// Which Game Boy the cartridge runs on, from `GB_HARDWARE`.
fn hardware_model(value: Option<&str>) -> Result<Model, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(HostConfig::default().model),
        Some(value) if value.eq_ignore_ascii_case("dmg") => Ok(Model::Dmg),
        Some(value) if value.eq_ignore_ascii_case("cgb") => Ok(Model::Cgb),
        Some(value) => Err(format!("`GB_HARDWARE={value}` is not `dmg` or `cgb`")),
    }
}

/// The Opus stream's target rate, from `GB_AUDIO_BITRATE`, in bits per second.
fn audio_bitrate(value: Option<&str>) -> Result<Option<i32>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(HostConfig::default().audio_bitrate);
    };
    match value.parse::<i32>() {
        Ok(0) => Ok(None),
        Ok(rate) if (audio::MIN_BITRATE..=audio::MAX_BITRATE).contains(&rate) => Ok(Some(rate)),
        _ => Err(format!(
            "`GB_AUDIO_BITRATE={value}` is not 0 or a rate between {} and {}",
            audio::MIN_BITRATE,
            audio::MAX_BITRATE
        )),
    }
}

fn serve_http(
    port: u16,
    published: Arc<Published>,
    run: Arc<CurrentRun>,
    control: Arc<ControlRequests>,
    admin_token: Option<String>,
    audio: Option<[u8; audio::HEADER_LEN]>,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("could not start the HTTP runtime: {e}"))?;

    let result = runtime.block_on(async move {
        let state =
            AppState { published, started: Instant::now(), run, control, admin_token, audio };
        let app = routes().with_state(state);

        // 0.0.0.0: the container publishes the port.
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
            .await
            .map_err(|e| format!("could not bind port {port}: {e}"))?;
        println!("gb serve — http://localhost:{port}");

        // SIGTERM as well as Ctrl-C.
        tokio::select! {
            result = axum::serve(listener, app).into_future() => {
                result.map_err(|e| format!("server failed: {e}"))
            }
            _ = shutdown_signal() => {
                println!("shutting down — dropping every connection, then checkpointing");
                Ok(())
            }
        }
    });

    // Dropping the `serve` future above ends the *accept* loop and nothing else.
    runtime.shutdown_timeout(SHUTDOWN_TIMEOUT);
    result
}

/// Every route, in one place — the module doc's table above says the same thing in prose.
fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/healthz", get(healthz))
        .route("/api/events", get(events))
        .route("/api/history", get(history))
        .route("/api/leaderboard", get(leaderboard::leaderboard))
        .route("/api/video", get(video_stream))
        .route("/api/audio", get(audio_stream))
        .route("/api/badges.png", get(badges::badges))
        .route("/api/pokemon/{dex}/front.png", get(sprites::front_pic))
        .route("/api/tool-image/{seq}/image.png", get(tool_image))
        .route("/api/new-run", post(new_run))
        .route("/api/clear", post(clear))
        .route("/reset-game", get(reset_game))
        .route("/favicon.png", get(sprites::favicon))
        .route("/favicon.ico", get(sprites::favicon))
        // At the root rather than under `/api`, because it is not the SPA's business — it is the
        // one URL an operator types to find out what is deployed.
        .route("/version", get(version::version))
        // Last, so the catch-all cannot shadow an API route it happens to match.
        .route("/", get(assets::index))
        .route("/{*path}", get(assets::asset))
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
        "version": crate::cli::VERSION,
        "uptime_ms": state.started.elapsed().as_millis() as u64,
        "video_seq": state.published.latest_keyframe().map(|k| k.seq),
        // Read per request rather than captured: `POST /api/new-run` changes it, and a liveness
        // endpoint reporting the run the process *started* with would be quietly wrong
        // afterwards.
        "run_id": state.run.get().run_id(),
    }))
}

/// `GB_ADMIN_TOKEN`, or `None` if it is unset or blank.
fn admin_token() -> Option<String> {
    std::env::var("GB_ADMIN_TOKEN").ok().map(|token| token.trim().to_string()).filter(|token| !token.is_empty())
}

/// Compare without an early return, so the time taken says nothing about how much of the token
/// was right.
fn tokens_match(offered: &str, expected: &str) -> bool {
    // `expected` is this server's own configuration rather than anything a caller sent, so
    // returning early on it leaks nothing — and it is what keeps the index below in bounds.
    if expected.is_empty() {
        return false;
    }
    let (offered, expected) = (offered.as_bytes(), expected.as_bytes());
    let mut difference = offered.len() ^ expected.len();
    for (index, byte) in offered.iter().enumerate() {
        // Index into `expected` cyclically, so a wrong *length* still walks the whole offered
        // token.
        difference |= (byte ^ expected[index % expected.len()]) as usize;
    }
    difference == 0
}

/// The password out of an `Authorization: Basic` header, or `None` if there is not one to be had.
fn basic_password(headers: &HeaderMap) -> Option<String> {
    use base64::Engine;
    let value = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    let encoded = value.strip_prefix("Basic ").or_else(|| value.strip_prefix("basic "))?;
    let decoded = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    decoded.split_once(':').map(|(_, password)| password.to_string())
}

/// Put one [`ControlRequest`] to the emulator thread and wait for the run id it answers with.
async fn ask(
    state: &AppState,
    what: ControlRequest,
    refused: StatusCode,
) -> Result<String, (StatusCode, String)> {
    let receiver = state.control.request(what).map_err(|failure| (StatusCode::CONFLICT, failure))?;
    match tokio::time::timeout(CONTROL_TIMEOUT, receiver).await {
        Ok(Ok(Ok(run_id))) => Ok(run_id),
        // The emulator tried and could not.
        Ok(Ok(Err(failure))) => Err((refused, failure)),
        // The sender was dropped, or never taken: either way the emulator thread is not running,
        // and `Obituary` will have said so on `/api/events` already.
        Ok(Err(_)) | Err(_) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "the emulator thread did not answer — see /api/events".to_string(),
        )),
    }
}

/// The `X-GB-Token` gate both JSON admin routes share: `Some(refusal)` to answer with, or `None`
/// to carry on.
fn admin_gate(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let Some(expected) = state.admin_token.as_deref() else {
        return Some((StatusCode::NOT_FOUND, Json(serde_json::json!({
            "error": "this server has no GB_ADMIN_TOKEN set, so the admin endpoints are disabled",
        }))).into_response());
    };
    let offered = headers.get(ADMIN_TOKEN_HEADER).and_then(|value| value.to_str().ok()).unwrap_or_default();
    if !tokens_match(offered, expected) {
        return Some((StatusCode::FORBIDDEN, Json(serde_json::json!({
            "error": format!("a matching {ADMIN_TOKEN_HEADER} header is required"),
        }))).into_response());
    }
    None
}

/// `POST /api/new-run` — [`ask`] for a script, gated on the `X-GB-Token` header.
async fn new_run(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(refusal) = admin_gate(&state, &headers) {
        return refusal;
    }
    match ask(&state, ControlRequest::NewRun, StatusCode::INTERNAL_SERVER_ERROR).await {
        Ok(run_id) => (StatusCode::OK, Json(serde_json::json!({ "run_id": run_id }))).into_response(),
        Err((status, error)) => (status, Json(serde_json::json!({ "error": error }))).into_response(),
    }
}

/// `POST /api/clear` — throw away the model's conversation and its plan, and keep playing.
async fn clear(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(refusal) = admin_gate(&state, &headers) {
        return refusal;
    }
    match ask(&state, ControlRequest::ClearConversation, StatusCode::CONFLICT).await {
        Ok(run_id) => (StatusCode::OK, Json(serde_json::json!({
            "run_id": run_id,
            "cleared": "the conversation and the plan; the game, the run and the battle script are untouched",
            "when": "at the model's next turn",
        }))).into_response(),
        Err((status, error)) => (status, Json(serde_json::json!({ "error": error }))).into_response(),
    }
}

/// `GET /reset-game` — [`ask`] for a person, gated on HTTP Basic, so the browser collects the
/// password in its own dialog and the SPA needs no token-handling code at all.
async fn reset_game(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(expected) = state.admin_token.as_deref() else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let offered = basic_password(&headers).unwrap_or_default();
    if !tokens_match(&offered, expected) {
        return (
            StatusCode::UNAUTHORIZED,
            [
                (axum::http::header::WWW_AUTHENTICATE, r#"Basic realm="gb", charset="UTF-8""#),
                (axum::http::header::CACHE_CONTROL, "no-store"),
            ],
            reset_page("Password required", "Enter <code>GB_ADMIN_TOKEN</code> as the password. Any user name will do."),
        )
            .into_response();
    }

    let (status, body) = match ask(&state, ControlRequest::NewRun, StatusCode::INTERNAL_SERVER_ERROR).await {
        Ok(run_id) => (
            StatusCode::OK,
            reset_page(
                &format!("New run {run_id}"),
                "The previous run was checkpointed and left complete on disk. \
                 <strong>Reloading this page starts another one.</strong>",
            ),
        ),
        Err((status, error)) => (status, reset_page("Could not start a new run", &html_escape(&error))),
    };
    (status, [(axum::http::header::CACHE_CONTROL, "no-store")], body).into_response()
}

/// The one page this server renders itself.
fn reset_page(heading: &str, detail: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex, nofollow\">\
         <link rel=\"icon\" type=\"image/png\" href=\"/favicon.png\">\
         <title>gb · new run</title><style>\
         :root {{ color-scheme: dark; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }}\
         body {{ margin: 0; display: grid; place-items: center; min-height: 100vh; \
         background: #0f1115; color: #d7dae0; line-height: 1.55; }}\
         main {{ max-width: 46ch; padding: 24px; border: 1px solid #262b34; border-radius: 4px; background: #161920; }}\
         h1 {{ margin: 0 0 8px; font-size: 15px; }}\
         p {{ margin: 0 0 12px; color: #7d838f; }}\
         code {{ color: #d7dae0; }} a {{ color: #8fd48f; }}\
         </style></head><body><main><h1>{}</h1><p>{detail}</p><p><a href=\"/\">← back to the game</a></p>\
         </main></body></html>",
        html_escape(heading),
    );
    ([(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
}

/// Enough for the two things that reach the page: a run id and an error message from the
/// emulator.
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[derive(serde::Deserialize)]
struct Since {
    #[serde(default)]
    since: u64,
}

/// The backlog, so a page that has just loaded shows the run it joined rather than an empty log
/// until the next thing happens.
async fn history(State(state): State<AppState>, Query(query): Query<Since>) -> Json<serde_json::Value> {
    let path = state.run.get().transcript_path();
    let events = tokio::task::spawn_blocking(move || transcript::read_since(&path, query.since))
        .await
        .unwrap_or_default();
    Json(serde_json::Value::Array(events))
}

/// The conversation and status stream.
async fn events(State(state): State<AppState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (receiver, opening) = state.published.join_events();
    let opening = tokio_stream::iter(opening.into_iter().map(sse_event));
    let live = BroadcastStream::new(receiver).filter_map(|item| {
        // A lagged client has missed events it cannot recover here.
        Some(sse_event(item.ok()?))
    });
    Sse::new(opening.chain(live)).keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
}

/// The picture a tool answered with, by the seq of the `tool_result` event that named it.
async fn tool_image(State(state): State<AppState>, Path(seq): Path<u64>) -> Response {
    match state.published.tool_image(seq) {
        // Immutable: a seq is never reused within a process, so the one answer this URL has ever
        // had is the one it will always have.
        Some(png) => (
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            png.to_vec(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "that picture is no longer held").into_response(),
    }
}

fn sse_event(event: published::UiEvent) -> Result<Event, Infallible> {
    Ok(Event::default().json_data(event).expect("UiEvent serialises"))
}

async fn video_stream(State(state): State<AppState>) -> Response {
    let (receiver, keyframe) = state.published.join_video();
    let mut floor = keyframe.as_ref().map_or(0, |k| k.seq);
    let published = Arc::clone(&state.published);
    let mut stream = VideoStream::default();

    let opening = keyframe.map(|keyframe| stream.frame(&keyframe.bytes)).unwrap_or_default();

    // Merged rather than `Sse::keep_alive`d: the keep-alive has to go through the same compressor
    // as everything else, since a deflate stream is a single ordered thing and not a sequence of
    // messages that can be interleaved with something else.
    let mut interval = tokio::time::interval(KEEP_ALIVE);
    // A starved task must not come back and fire every missed tick at once — the point is
    // liveness, and a burst of keep-alives says nothing a single one does not.
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let beats = IntervalStream::new(interval).map(|_| None);
    let messages = BroadcastStream::new(receiver).map(Some);
    let live = messages.merge(beats).filter_map(move |item| {
        let message = match item {
            // Keep-alive: an empty message, so a proxy sees traffic and the client sees nothing.
            None => return Some(Ok::<_, Infallible>(stream.frame(&[]))),
            Some(Ok(message)) if message.seq > floor => message,
            // Already covered by the keyframe this connection opened with.
            Some(Ok(_)) => return None,
            // The client fell out of the ring buffer, so its palette and its screen are both
            // suspect.
            Some(Err(BroadcastStreamRecvError::Lagged(_))) => published.latest_keyframe()?,
        };
        floor = message.seq;
        Some(Ok(stream.frame(&message.bytes)))
    });

    let body = tokio_stream::iter([Ok::<_, Infallible>(opening)]).chain(live);
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/octet-stream"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
            // Nginx-family proxies buffer an unknown-length body by default, which would turn a
            // livestream into a slideshow.
            (axum::http::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        axum::body::Body::from_stream(body),
    )
        .into_response()
}

/// The audio stream: the header, then raw Opus packets, length-prefixed exactly as video's are.
async fn audio_stream(State(state): State<AppState>) -> Response {
    let Some(header) = state.audio else {
        // 503 and not 404, and the client tells them apart.
        return (StatusCode::SERVICE_UNAVAILABLE, "audio is off (GB_AUDIO_BITRATE=0)").into_response();
    };
    let receiver = state.published.join_audio();
    let mut stream = AudioStream;
    let opening = stream.frame(&header);

    // Merged rather than a separate task, for `video_stream`'s reason: one ordered byte stream.
    let mut interval = tokio::time::interval(KEEP_ALIVE);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let beats = IntervalStream::new(interval).map(|_| None);
    let packets = BroadcastStream::new(receiver).map(Some);
    let live = packets.merge(beats).filter_map(move |item| match item {
        // The keep-alive is doing more work here than it does for video.
        None => Some(Ok::<_, Infallible>(stream.frame(&[]))),
        Some(Ok(packet)) => Some(Ok(stream.frame(&packet))),
        // Skipped, and that is the whole of the handling.
        Some(Err(BroadcastStreamRecvError::Lagged(_))) => None,
    });

    let body = tokio_stream::iter([Ok::<_, Infallible>(opening)]).chain(live);
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/octet-stream"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
            (axum::http::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        axum::body::Body::from_stream(body),
    )
        .into_response()
}

/// One connection's framing, and nothing else.
struct AudioStream;

impl AudioStream {
    fn frame(&mut self, message: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + message.len());
        out.extend_from_slice(&(message.len() as u32).to_le_bytes());
        out.extend_from_slice(message);
        out
    }
}

/// One connection's compressor: a single deflate stream, flushed after every message.
struct VideoStream {
    deflate: flate2::write::ZlibEncoder<Vec<u8>>,
}

impl Default for VideoStream {
    fn default() -> Self {
        Self { deflate: flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6)) }
    }
}

impl VideoStream {
    fn frame(&mut self, message: &[u8]) -> Vec<u8> {
        use std::io::Write;
        // In-memory writes: the only way `write_all` fails here is an allocation failure, which
        // is not something this connection can do anything about.
        let _ = self.deflate.write_all(&(message.len() as u32).to_le_bytes());
        let _ = self.deflate.write_all(message);
        let _ = self.deflate.flush();
        std::mem::take(self.deflate.get_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── /api/audio
    // ───────────────────────────────────────────────────────────────────────────────

    /// The framing is a `u32` length and then the bytes, and a reader has to be able to split a
    /// concatenation back into exactly what went in — including an empty message and a large one.
    #[test]
    fn the_audio_stream_is_length_prefixed_and_carries_its_payload_verbatim() {
        let mut stream = AudioStream;
        let sent: Vec<Vec<u8>> =
            vec![audio::header().to_vec(), vec![0xff; 3], Vec::new(), vec![7; 1200], vec![1, 2, 3]];

        let wire: Vec<u8> = sent.iter().flat_map(|message| stream.frame(message)).collect();

        let mut read: Vec<Vec<u8>> = Vec::new();
        let mut at = 0;
        while at + 4 <= wire.len() {
            let length = u32::from_le_bytes(wire[at..at + 4].try_into().unwrap()) as usize;
            at += 4;
            read.push(wire[at..at + length].to_vec());
            at += length;
        }
        assert_eq!(at, wire.len(), "the last message did not end where the stream did");
        assert_eq!(read, sent);
    }

    /// The guard on the decision, not on the code.
    #[test]
    fn nothing_compresses_the_audio_stream() {
        let mut stream = AudioStream;
        let packet = vec![0u8; 64];
        assert_eq!(stream.frame(&packet).len(), 4 + packet.len());
        assert_eq!(&stream.frame(&packet)[4..], &packet[..], "the payload was transformed");
    }

    /// The twin of `a_keepalive_puts_bytes_on_the_wire_and_no_message_in_the_stream`, and it
    /// matters more here: a parked run produces no audio for hours, so this is the only thing
    /// between a listening page and its `STALE_MS` watchdog.
    #[test]
    fn an_audio_keepalive_puts_bytes_on_the_wire_and_no_packet_in_the_stream() {
        let mut stream = AudioStream;
        let beat = stream.frame(&[]);
        assert_eq!(beat, vec![0, 0, 0, 0], "a keep-alive is a zero length and nothing after it");
    }

    /// `GB_AUDIO_BITRATE`: the default when nobody said, `0` for off, and a loud refusal
    /// otherwise.
    #[test]
    fn the_audio_bitrate_variable_defaults_and_refuses_nonsense() {
        assert_eq!(audio_bitrate(None).unwrap(), Some(audio::DEFAULT_BITRATE));
        assert_eq!(audio_bitrate(Some("  ")).unwrap(), Some(audio::DEFAULT_BITRATE));
        assert_eq!(audio_bitrate(Some("0")).unwrap(), None, "0 is how audio is turned off");
        assert_eq!(audio_bitrate(Some(" 32000 ")).unwrap(), Some(32_000));

        for nonsense in ["12", "1000000", "lots", "-1"] {
            let error = audio_bitrate(Some(nonsense)).expect_err("{nonsense} should be refused");
            assert!(error.contains("GB_AUDIO_BITRATE") && error.contains(nonsense), "{error}");
        }
    }

    /// `GB_HARDWARE` picks the machine, and the default is the DMG — the whole test suite and
    /// every committed fixture were captured on one, and the video bench asserts a four-shade
    /// screen.
    #[test]
    fn the_hardware_variable_defaults_to_a_dmg_and_refuses_anything_it_does_not_know() {
        assert_eq!(hardware_model(None), Ok(Model::Dmg));
        // Blank counts as unset, for the reason `GB_ADMIN_TOKEN` does: that is the shape a
        // placeholder value takes in a Deployment.
        assert_eq!(hardware_model(Some("   ")), Ok(Model::Dmg));
        assert_eq!(hardware_model(Some("dmg")), Ok(Model::Dmg));
        assert_eq!(hardware_model(Some("cgb")), Ok(Model::Cgb));
        assert_eq!(hardware_model(Some(" CGB ")), Ok(Model::Cgb), "trimmed and case-insensitive");

        // Refused rather than ignored: a container that silently served the wrong picture would
        // be diagnosed by looking at the video codec, which is the wrong place entirely.
        let error = hardware_model(Some("color")).unwrap_err();
        assert!(error.contains("GB_HARDWARE") && error.contains("color"), "{error}");
    }

    /// The transport contract, end to end without a socket: what `/api/video` writes must inflate
    /// as one deflate stream and split back into exactly the messages that went in.
    #[test]
    fn the_video_stream_is_one_deflate_stream_of_length_prefixed_messages() {
        use std::io::Write;
        let messages: Vec<Vec<u8>> =
            vec![vec![1, 2, 3], vec![], (0..5000u32).map(|n| (n % 7) as u8).collect(), vec![9]];

        let mut stream = VideoStream::default();
        let mut inflate = flate2::write::ZlibDecoder::new(Vec::new());
        let mut compressed = 0usize;
        for message in &messages {
            let chunk = stream.frame(message);
            compressed += chunk.len();
            inflate.write_all(&chunk).expect("the chunk is a valid continuation");
            inflate.flush().expect("…and a flushed one, so it decodes now rather than at the end");
        }
        let plain = inflate.finish().expect("in-memory");

        let mut at = 0;
        for (n, message) in messages.iter().enumerate() {
            let length =
                u32::from_le_bytes(plain[at..at + 4].try_into().expect("four bytes")) as usize;
            at += 4;
            assert_eq!(length, message.len(), "message {n} announced the wrong length");
            assert_eq!(&plain[at..at + length], &message[..], "message {n} did not survive");
            at += length;
        }
        assert_eq!(at, plain.len(), "trailing bytes after the last message");

        // And it is actually compressing: 5 kB of a repeating pattern must not cost 5 kB.
        assert!(compressed < plain.len() / 2, "{compressed} B for {} B of input", plain.len());
    }

    /// A zero-length message is the keep-alive, and it has to reach the wire — an idle screen
    /// that sends nothing at all is a connection a proxy closes.
    #[test]
    fn a_keepalive_puts_bytes_on_the_wire_and_no_message_in_the_stream() {
        let mut stream = VideoStream::default();
        stream.frame(&[1, 2, 3]);
        let beat = stream.frame(&[]);
        assert!(!beat.is_empty(), "a keep-alive that produced no bytes keeps nothing alive");
    }

    /// The endpoint is off unless a token is set, and blank is not set.
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

    fn authorization(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::AUTHORIZATION, value.parse().expect("a header value"));
        headers
    }

    /// `/reset-game`'s half of the token check.
    #[test]
    fn only_a_well_formed_basic_header_yields_a_password() {
        assert_eq!(basic_password(&HeaderMap::new()), None, "no header at all");
        assert_eq!(basic_password(&authorization("Bearer s3cret")), None, "the wrong scheme");
        assert_eq!(basic_password(&authorization("Basic !!!not base64!!!")), None);
        // `dXNlcg==` is "user" — no colon, so there is no password in it.
        assert_eq!(basic_password(&authorization("Basic dXNlcg==")), None);
    }

    /// The username is ignored and the split is on the *first* colon.
    #[test]
    fn the_password_is_everything_after_the_first_colon() {
        use base64::Engine;
        let basic = |raw: &str| {
            let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
            basic_password(&authorization(&format!("Basic {encoded}")))
        };
        assert_eq!(basic("anyone:s3cret").as_deref(), Some("s3cret"), "the user name is discarded");
        assert_eq!(basic(":s3cret").as_deref(), Some("s3cret"), "an empty user name is the usual case");
        assert_eq!(basic("gb:a:b:c").as_deref(), Some("a:b:c"), "a colon is legal inside a token");
        assert_eq!(basic("user:").as_deref(), Some(""), "…and an empty password reaches tokens_match, which refuses it");
        assert!(!tokens_match("", "s3cret"));
    }

    /// The router is built at startup and a bad path panics there, so a route axum's matcher
    /// rejects — `/api/pokemon/{dex}.png`, which needs a dynamic suffix it does not support —
    /// would take the server down on boot rather than 404.
    #[test]
    fn every_route_pattern_is_one_axum_accepts() {
        let _: Router<AppState> = routes();
    }
}
