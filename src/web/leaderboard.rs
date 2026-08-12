//! `GET /api/leaderboard` — the runs that have finished the game, fastest first.
//!
//! Everything interesting is in [`crate::run::hall_of_fame`]: this reads the ledger it appends to
//! and hands the rows straight out. The only decisions here are how many to return and what to do
//! when there are none.

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Json, Response};

use crate::run::hall_of_fame;

/// The default page, and what the SPA's overlay asks for.
const DEFAULT_LIMIT: usize = 10;

/// Above this, the answer stops being a leaderboard and starts being a database dump. The ledger is
/// read and sorted in memory, so the cap is also what stops a query from being a way to make the
/// server allocate.
const MAX_LIMIT: usize = 100;

#[derive(serde::Deserialize)]
pub struct Top {
    limit: Option<usize>,
}

/// ⚠️ **An empty list is the normal answer**, not an error. A server nobody has finished a game on
/// is the state of every fresh deployment, and a 500 there would have the page shouting about a
/// failure on day one. `hall_of_fame::top` returns an empty `Vec` for a missing ledger for exactly
/// this reason.
pub async fn leaderboard(State(state): State<super::AppState>, Query(top): Query<Top>) -> Response {
    let limit = top.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    // Read per request rather than cached: a completion is rare, the file is a few kilobytes, and a
    // cache would need invalidating from the emulator thread — which is the one direction this
    // module deliberately cannot reach.
    Json(hall_of_fame::top(state.run.root(), limit)).into_response()
}
