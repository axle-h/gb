//! `GET /api/leaderboard` — the runs that have finished the game, fastest first.

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Json, Response};

use poke_agent::run::hall_of_fame;

const DEFAULT_LIMIT: usize = 10;

const MAX_LIMIT: usize = 100;

#[derive(serde::Deserialize)]
pub struct Top {
    limit: Option<usize>,
}

pub async fn leaderboard(State(state): State<super::AppState>, Query(top): Query<Top>) -> Response {
    let limit = top.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    // Read per request: a cache would need invalidating from the emulator thread.
    Json(hall_of_fame::top(state.run.root(), limit)).into_response()
}
