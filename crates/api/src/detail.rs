//! Per-node detail endpoints powering the M3 right-hand inspector.
//!
//! Two endpoints in this module:
//! - `GET /api/tool_calls/:id` — everything about one tool_call: tool name,
//!   the (already-redacted) input JSON, timing, error state, the result
//!   preview, the artifacts it touched, and a back-link to the event uuid
//!   so the SPA can highlight neighbours in the causal view.
//! - `GET /api/artifacts/:id?session_id=…` — artifact metadata + the list
//!   of tool_calls in the supplied session that touched it. The
//!   `session_id` query param keeps the cardinality bounded; without it
//!   a hot file from many sessions could return thousands of rows.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::AppState;

#[derive(Serialize, FromRow)]
pub struct ToolCallDetail {
    pub id: i64,
    pub event_uuid: String,
    pub session_id: String,
    pub ts: i64,
    pub tool_use_id: String,
    pub tool_name: String,
    /// Already redacted by the ingest pipeline.
    pub input_json: String,
    pub duration_ms: Option<i64>,
    pub success: Option<i64>,
    pub error_kind: Option<String>,
    pub is_sidechain: i64,
    pub agent_id: Option<String>,
    pub result_output_bytes: Option<i64>,
    pub result_is_error: Option<i64>,
    pub result_error_text: Option<String>,
    pub artifact_count: i64,
}

#[derive(Serialize, FromRow)]
pub struct ArtifactTouch {
    pub tool_call_id: i64,
    pub ts: i64,
    pub tool_name: String,
    pub access_kind: String,
}

#[derive(Serialize, FromRow)]
pub struct ArtifactDetail {
    pub id: i64,
    pub kind: String,
    pub canonical_key: String,
    pub display: String,
    pub first_seen_ts: i64,
    pub last_seen_ts: i64,
}

#[derive(Serialize)]
pub struct ArtifactResponse {
    #[serde(flatten)]
    pub artifact: ArtifactDetail,
    pub touches: Vec<ArtifactTouch>,
}

#[derive(Deserialize)]
pub struct ArtifactParams {
    pub session_id: Option<String>,
}

pub async fn tool_call(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<ToolCallDetail>, StatusCode> {
    let row: Option<ToolCallDetail> = sqlx::query_as(
        r#"
        SELECT
          tc.id,
          tc.event_uuid,
          tc.session_id,
          tc.ts,
          tc.tool_use_id,
          tc.tool_name,
          tc.input_json,
          tc.duration_ms,
          tc.success,
          tc.error_kind,
          tc.is_sidechain,
          tc.agent_id,
          tr.output_bytes AS result_output_bytes,
          tr.is_error    AS result_is_error,
          tr.error_text  AS result_error_text,
          (SELECT COUNT(*) FROM tool_artifact_edges WHERE tool_call_id = tc.id)
                          AS artifact_count
        FROM tool_calls tc
        LEFT JOIN tool_results tr ON tr.tool_call_id = tc.id
        WHERE tc.id = ?1
        "#,
    )
    .bind(id)
    .fetch_optional(&state.store.reader)
    .await
    .map_err(|e| {
        tracing::warn!("tool_call detail query failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    row.map(Json).ok_or(StatusCode::NOT_FOUND)
}

pub async fn artifact(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(params): Query<ArtifactParams>,
) -> Result<Json<ArtifactResponse>, StatusCode> {
    let artifact: Option<ArtifactDetail> = sqlx::query_as(
        "SELECT id, kind, canonical_key, display, first_seen_ts, last_seen_ts FROM artifacts WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(&state.store.reader)
    .await
    .map_err(|e| {
        tracing::warn!("artifact detail query failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let artifact = artifact.ok_or(StatusCode::NOT_FOUND)?;

    // Scope to a session when given. Without a session filter, hot files
    // touched in many sessions could return thousands of touches; we keep
    // the response bounded.
    let touches: Vec<ArtifactTouch> = match params.session_id {
        Some(sid) => sqlx::query_as(
            r#"
            SELECT tae.tool_call_id, tc.ts, tc.tool_name, tae.access_kind
            FROM tool_artifact_edges tae
            JOIN tool_calls tc ON tc.id = tae.tool_call_id
            WHERE tae.artifact_id = ?1 AND tc.session_id = ?2
            ORDER BY tc.ts
            "#,
        )
        .bind(id)
        .bind(sid)
        .fetch_all(&state.store.reader)
        .await
        .map_err(|e| {
            tracing::warn!("artifact touches query failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?,
        None => Vec::new(),
    };

    Ok(Json(ArtifactResponse { artifact, touches }))
}
