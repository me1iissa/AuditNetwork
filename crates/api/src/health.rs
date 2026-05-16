use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use serde_json::Value;

use crate::AppState;

pub async fn healthz() -> Json<Value> {
    Json(serde_json::json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

pub async fn readyz(State(state): State<Arc<AppState>>) -> Result<Json<Value>, StatusCode> {
    let n: i64 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&state.store.reader)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(serde_json::json!({ "ok": n == 1 })))
}
