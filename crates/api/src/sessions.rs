use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use sqlx::FromRow;

use crate::AppState;

#[derive(Serialize, FromRow)]
pub struct SessionSummary {
    pub id: String,
    pub ai_title: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub claude_version: Option<String>,
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cache_read: i64,
    pub total_cache_creation: i64,
    pub tool_calls: i64,
    pub artifacts_touched: i64,
    pub events_total: i64,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<SessionSummary>>, StatusCode> {
    let rows: Vec<SessionSummary> = sqlx::query_as(
        r#"
        SELECT
          s.id,
          s.ai_title,
          s.started_at,
          s.ended_at,
          s.claude_version,
          s.model,
          s.cwd,
          s.git_branch,
          s.total_input_tokens,
          s.total_output_tokens,
          s.total_cache_read,
          s.total_cache_creation,
          (SELECT COUNT(*) FROM tool_calls WHERE session_id = s.id)              AS tool_calls,
          (SELECT COUNT(DISTINCT tae.artifact_id)
             FROM tool_artifact_edges tae
             JOIN tool_calls tc ON tc.id = tae.tool_call_id
            WHERE tc.session_id = s.id)                                          AS artifacts_touched,
          (SELECT COUNT(*) FROM events WHERE session_id = s.id)                  AS events_total
        FROM sessions s
        ORDER BY s.started_at DESC
        "#,
    )
    .fetch_all(&state.store.reader)
    .await
    .map_err(|e| {
        tracing::warn!("sessions list query failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(rows))
}

pub async fn detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionSummary>, StatusCode> {
    let row: Option<SessionSummary> = sqlx::query_as(
        r#"
        SELECT
          s.id,
          s.ai_title,
          s.started_at,
          s.ended_at,
          s.claude_version,
          s.model,
          s.cwd,
          s.git_branch,
          s.total_input_tokens,
          s.total_output_tokens,
          s.total_cache_read,
          s.total_cache_creation,
          (SELECT COUNT(*) FROM tool_calls WHERE session_id = s.id)              AS tool_calls,
          (SELECT COUNT(DISTINCT tae.artifact_id)
             FROM tool_artifact_edges tae
             JOIN tool_calls tc ON tc.id = tae.tool_call_id
            WHERE tc.session_id = s.id)                                          AS artifacts_touched,
          (SELECT COUNT(*) FROM events WHERE session_id = s.id)                  AS events_total
        FROM sessions s
        WHERE s.id = ?1
        "#,
    )
    .bind(&id)
    .fetch_optional(&state.store.reader)
    .await
    .map_err(|e| {
        tracing::warn!("session detail query failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    row.map(Json).ok_or(StatusCode::NOT_FOUND)
}
