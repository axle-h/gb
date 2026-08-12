//! **W1.2 / W2** — the HTTP server.
//!
//! Read-only endpoints, and **one** that is not. Viewer controls are still out of scope by decision
//! (§1.1 of `docs/llm-web-playthrough-plan.md`) — nothing here can press a button, choose a move or
//! steer the run — and for everything except `POST /api/new-run` that remains structural rather than
//! editorial: this module can reach [`published::Published`] and nothing else.
//!
//! ⚠️ **Starting the game over is the exception, and it is the only one.** It reaches the emulator
//! through [`crate::host::NewRunRequests`], which carries no data inwards and is answered by the
//! emulator thread between instructions. It is **off unless `GB_ADMIN_TOKEN` is set** — without it
//! both routes that offer it 404 exactly as if they had never been registered — because the
//! deployment this exists for serves the public internet and starting the game over is not
//! something a passer-by should be able to do.
//!
//! There are two doors onto it and they differ only in how they ask for the token. `/reset-game` is
//! the one a person uses: it answers an unauthenticated GET with `WWW-Authenticate: Basic`, so the
//! *browser* collects the password in its own dialog and the page needs no token-handling code at
//! all. `POST /api/new-run` is the one a script uses, and keeps its `X-GB-Token` header.
//!
//! ```text
//! GET  /                            the SPA (`web/dist`, embedded — see `assets.rs`)
//! GET  /{*path}                     its assets
//! GET  /favicon.png, /favicon.ico   the overworld Poké Ball, decoded from the cartridge
//! GET  /reset-game                  start the game over; HTTP Basic (needs GB_ADMIN_TOKEN)
//! GET  /version                     which build this is: crate version, build date, branch, commit
//! GET  /api/healthz                 liveness
//! GET  /api/events                  SSE — the status heartbeat, plus agent events as they happen
//! GET  /api/video                   binary — a keyframe, then block deltas, deflated per connection
//! GET  /api/badges.png              the eight gym badges, decoded from the cartridge
//! GET  /api/pokemon/{dex}/front.png one Pokémon's front sprite, decompressed from the cartridge
//! GET  /api/history?since=          W7 — the transcript from a sequence number, for a fresh page
//! GET  /api/leaderboard?limit=      the runs that have finished the game, fastest first
//! POST /api/new-run                 the same reset, for a script; `X-GB-Token`
//! ```

pub mod assets;
pub mod badges;
pub mod leaderboard;
pub mod published;
pub mod sprites;
pub mod version;
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
use tokio_stream::wrappers::{BroadcastStream, IntervalStream};
use tokio_stream::{Stream, StreamExt};

use crate::cli::ServePolicy;
use crate::game_boy::GameBoy;
use crate::host::{EmulatorHost, HostConfig, NewRunRequests};
use crate::pokemon::policy::RandomPolicy;
use crate::run::{CurrentRun, Origin, RunDir, transcript};
use published::Published;

/// Proxies close an idle connection. Every two seconds `/api/events` sends a comment and
/// `/api/video` an empty message — neither is anything a client has to parse, and an idle screen
/// under `--policy llm` is a long silence, not a rare one.
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
            use crate::llm::{client::OpenAiClient, todo::TodoList, worker};
            use crate::pokemon::llm_policy::LlmPolicy;

            let config = llm.expect("built above");
            println!("gb serve — {} via {}", config.model, config.base_url);
            let endpoint = Box::new(OpenAiClient::new(&config));
            // **W9** — read off before `config` is moved into the worker. The watchdog belongs to
            // the policy (it is what the agent asks how long to wait), not to the turn loop.
            let stuck_timeout = config.stuck_timeout;
            // **W6b** — the model's own plan lives in the run directory, so it survives both a
            // compaction and a restart.
            let todo = TodoList::open(Some(run.path()));
            let (worker, handles) =
                worker::channels(endpoint, config, Arc::clone(&published), todo);
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
            run: Some(Arc::clone(&current)),
            new_runs: Some(Arc::clone(&new_runs)),
            status_interval: status_interval()?,
            ..HostConfig::default()
        },
        Arc::clone(&shutdown),
    )?;

    println!(
        "gb serve — starting a new run (/reset-game, POST /api/new-run) is {}",
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
        let app = routes().with_state(state);

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

/// Every route, in one place — the module doc's table above says the same thing in prose.
///
/// ⚠️ **A path axum's router cannot parse is a panic when the router is *built*, i.e. at startup**,
/// not a 404 at request time. `/api/pokemon/{dex}.png` is exactly that mistake: matchit does not
/// support a static suffix after a parameter, so the sprite route gives the parameter a segment of
/// its own. `every_route_pattern_is_one_axum_accepts` is the guard, and it can only be a guard
/// because this list is not duplicated inside `serve_http`.
fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/healthz", get(healthz))
        .route("/api/events", get(events))
        .route("/api/history", get(history))
        .route("/api/leaderboard", get(leaderboard::leaderboard))
        .route("/api/video", get(video_stream))
        .route("/api/badges.png", get(badges::badges))
        .route("/api/pokemon/{dex}/front.png", get(sprites::front_pic))
        .route("/api/new-run", post(new_run))
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
        // Here rather than only in a run's record, so "which build is this?" is answerable against a
        // live deployment without reading a file off the volume.
        "version": crate::cli::VERSION,
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

/// The password out of an `Authorization: Basic` header, or `None` if there is not one to be had.
///
/// ⚠️ **The username is ignored.** Basic auth is `user:pass` and there is only one secret here, so
/// whatever a viewer types in the first box is discarded and the second is compared against
/// `GB_ADMIN_TOKEN`. ⚠️ And the split is on the **first** colon, not the last and not `split('=')`:
/// a colon is a perfectly ordinary character in a generated token, and splitting wrongly would
/// reject exactly the tokens most likely to be in use.
fn basic_password(headers: &HeaderMap) -> Option<String> {
    use base64::Engine;
    let value = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    let encoded = value.strip_prefix("Basic ").or_else(|| value.strip_prefix("basic "))?;
    let decoded = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    decoded.split_once(':').map(|(_, password)| password.to_string())
}

/// **Start the game over, in place**, and answer with the id of the run that just began.
///
/// The whole reason [`NewRunRequests`] exists. What it is *not* is a general control channel: it
/// carries no data inwards, and the emulator thread decides when to act on it.
///
/// The old run is not deleted, or even stopped in any sense that loses anything: the emulator
/// checkpoints it before swapping, so it is left complete on the volume and can be resumed by
/// pointing a process back at it.
async fn start_new_run(state: &AppState) -> Result<String, (StatusCode, String)> {
    let receiver = state.new_runs.request().map_err(|failure| (StatusCode::CONFLICT, failure))?;
    match tokio::time::timeout(NEW_RUN_TIMEOUT, receiver).await {
        Ok(Ok(Ok(run_id))) => Ok(run_id),
        // The emulator tried and could not — a full disk, most likely. Its message is the useful one.
        Ok(Ok(Err(failure))) => Err((StatusCode::INTERNAL_SERVER_ERROR, failure)),
        // The sender was dropped, or never taken: either way the emulator thread is not running, and
        // `Obituary` will have said so on `/api/events` already.
        Ok(Err(_)) | Err(_) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "the emulator thread did not answer — see /api/events".to_string(),
        )),
    }
}

/// `POST /api/new-run` — [`start_new_run`] for a script, gated on the `X-GB-Token` header.
///
/// ⚠️ **404, not 403, when `GB_ADMIN_TOKEN` is unset.** The route is registered unconditionally
/// because the router is built before the token is a question, so "off" has to be a response — and
/// it should be the response a route that does not exist would give, rather than one advertising a
/// reset endpoint to anyone who scans for it.
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

    match start_new_run(&state).await {
        Ok(run_id) => (StatusCode::OK, Json(serde_json::json!({ "run_id": run_id }))).into_response(),
        Err((status, error)) => (status, Json(serde_json::json!({ "error": error }))).into_response(),
    }
}

/// `GET /reset-game` — [`start_new_run`] for a person, gated on **HTTP Basic**, so the browser
/// collects the password in its own dialog and the SPA needs no token-handling code at all. That is
/// the whole reason it exists: the page used to carry a button, a `confirm`, a `prompt` and a
/// `sessionStorage` key to do what `WWW-Authenticate` does natively.
///
/// ⚠️ **Nothing links here, and nothing should.** A GET that resets the game must not be reachable
/// by a prefetch, a crawler or a middle-click; it is a URL somebody types. Once the browser has the
/// credentials it will re-send them without asking, so a *refresh* of this page starts another run —
/// which the response says, because a viewer who does not expect it has no other way to find out.
///
/// ⚠️ **404 when `GB_ADMIN_TOKEN` is unset, and byte-identical to the catch-all's** — challenging
/// for a password would tell a scanner the endpoint is here, which is the same argument
/// [`new_run`]'s 404 is making.
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

    let (status, body) = match start_new_run(&state).await {
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

/// The one page this server renders itself. Self-contained and in the SPA's palette, because it is
/// reached by typing a URL and may well be the first thing a viewer sees.
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

/// Enough for the two things that reach the page: a run id and an error message from the emulator.
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
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
///
/// ## Why this is not SSE any more
///
/// It was, for W2 through W9: one base64 line per message, read by an `EventSource`. `video/bench.rs`
/// measured four minutes of real play through it at **565 kbit/s**, not the ~19 the README claimed
/// — that number was an idle screen. Three things fixed it, and only the third needed the transport
/// to change:
///
/// 1. the v2 wire format (see [`video`]) — 565 → 445 kbit/s;
/// 2. **deflate across the connection rather than per message** — 445 → 21. A Game Boy screen is
///    built of repeated 8×8 tiles, so the same payload bytes recur within a frame *and* across
///    frames; a compressor with a window that spans the whole stream sees all of it. Per message
///    it would be 54, so the shared window is worth 2.5× on its own;
/// 3. **binary, because base64 costs far more after compression than before**. 33% before, but
///    69–113% *after* — base64 shifts a repeating byte pattern into three alphabet phases and the
///    LZ77 window stops recognising it. SSE cannot carry binary, so SSE had to go.
///
/// Not a WebSocket, though: nothing here is bidirectional and this module reaching the emulator is
/// exactly the property the whole file is built to avoid. A chunked binary response needs no upgrade
/// handshake, no ping/pong, and no second reconnection story — the client reads it with `fetch` and
/// a `DecompressionStream`.
///
/// ⚠️ **The compression is `Content-Type`, not `Content-Encoding`.** A declared encoding is an
/// invitation for a proxy to decompress and recompress it, which would buffer whole messages and
/// hand a livestream a latency problem that only shows up in production. These are opaque bytes and
/// the client inflates them itself.
///
/// The framing is a `u32` little-endian length before each message. Zero length is a keep-alive,
/// which is also what keeps the deflate stream ticking over an idle screen.
async fn video_stream(State(state): State<AppState>) -> Response {
    let (receiver, keyframe) = state.published.join_video();
    let mut floor = keyframe.as_ref().map_or(0, |k| k.seq);
    let published = Arc::clone(&state.published);
    let mut stream = VideoStream::default();

    let opening = keyframe.map(|keyframe| stream.frame(&keyframe.bytes)).unwrap_or_default();

    // Merged rather than `Sse::keep_alive`d: the keep-alive has to go through the same compressor as
    // everything else, since a deflate stream is a single ordered thing and not a sequence of
    // messages that can be interleaved with something else.
    let mut interval = tokio::time::interval(KEEP_ALIVE);
    // A starved task must not come back and fire every missed tick at once — the point is liveness,
    // and a burst of keep-alives says nothing a single one does not.
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
            // suspect. A fresh keyframe repairs both; dropping the connection would work too, but
            // re-syncing in place is invisible to the viewer.
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
            // nginx-family proxies buffer an unknown-length body by default, which would turn a
            // livestream into a slideshow. Traefik does not, but the header costs nothing.
            (axum::http::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        axum::body::Body::from_stream(body),
    )
        .into_response()
}

/// One connection's compressor: a single deflate stream, flushed after every message.
///
/// ⚠️ **The flush is the whole design and it is not free to forget.** Without it the encoder holds a
/// message back until its internal buffer fills, which is right for a file and useless for a
/// livestream — the screen would arrive in bursts seconds apart. Flushing per message costs a few
/// bytes of block boundary and keeps the window, which is where the compression actually comes from.
///
/// Per connection, so two viewers cost two compressors. That is the price of the shared window: at
/// 30 fps and ~3 kB a message it is well under a percent of a core each, and the alternative —
/// compressing once for everyone — is the per-message case that measured 2.5× worse.
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
        // In-memory writes: the only way `write_all` fails here is an allocation failure, which is
        // not something this connection can do anything about.
        let _ = self.deflate.write_all(&(message.len() as u32).to_le_bytes());
        let _ = self.deflate.write_all(message);
        let _ = self.deflate.flush();
        std::mem::take(self.deflate.get_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The transport contract, end to end without a socket: what `/api/video` writes must inflate as
    /// **one** deflate stream and split back into exactly the messages that went in.
    ///
    /// ⚠️ The interesting half is the *incremental* read. A test that inflated the whole body at the
    /// end would pass even if the encoder buffered everything until the connection closed, which is
    /// the one failure mode that matters here — a livestream that arrives in bursts. So each
    /// message's bytes are inflated as they are produced, and the message has to come out then.
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

    /// A zero-length message is the keep-alive, and it has to reach the wire — an idle screen that
    /// sends nothing at all is a connection a proxy closes.
    #[test]
    fn a_keepalive_puts_bytes_on_the_wire_and_no_message_in_the_stream() {
        let mut stream = VideoStream::default();
        stream.frame(&[1, 2, 3]);
        let beat = stream.frame(&[]);
        assert!(!beat.is_empty(), "a keep-alive that produced no bytes keeps nothing alive");
    }

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

    fn authorization(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::AUTHORIZATION, value.parse().expect("a header value"));
        headers
    }

    /// `/reset-game`'s half of the token check. Everything that is not a well-formed `Basic`
    /// credential has to come out as `None` rather than as a panic or an empty-string match —
    /// [`tokens_match`] refuses the empty string, so `None` and "wrong" land in the same place.
    #[test]
    fn only_a_well_formed_basic_header_yields_a_password() {
        assert_eq!(basic_password(&HeaderMap::new()), None, "no header at all");
        assert_eq!(basic_password(&authorization("Bearer s3cret")), None, "the wrong scheme");
        assert_eq!(basic_password(&authorization("Basic !!!not base64!!!")), None);
        // `dXNlcg==` is "user" — no colon, so there is no password in it.
        assert_eq!(basic_password(&authorization("Basic dXNlcg==")), None);
    }

    /// ⚠️ **The username is ignored and the split is on the *first* colon.** A generated token may
    /// well contain one, and splitting on the last would hand `tokens_match` a truncated password
    /// that could never match — a failure that looks exactly like the operator mistyping it.
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

    /// ⚠️ **The router is built at startup and a bad path panics there**, so a route axum's matcher
    /// rejects — `/api/pokemon/{dex}.png`, which needs a dynamic suffix it does not support — would
    /// take the server down on boot rather than 404. Nothing else in the suite builds the router.
    #[test]
    fn every_route_pattern_is_one_axum_accepts() {
        let _: Router<AppState> = routes();
    }
}
