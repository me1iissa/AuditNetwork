//! GET /api/sessions/:id/recommendations — list rows from the
//! `recommendations` table for one session. Optimisation detectors run
//! out-of-band (CLI `auditnetwork recommend <id>` or — eventually — on
//! a watcher-triggered session-close event). This endpoint is read-only.

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
pub struct RecommendationRow {
    pub id: i64,
    pub session_id: String,
    pub rule_id: String,
    pub severity: String,
    pub summary: String,
    pub evidence_json: String,
    pub estimated_save: Option<String>,
    pub created_at: i64,
    pub dismissed_at: Option<i64>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<RecommendationRow>>, StatusCode> {
    let rows: Vec<RecommendationRow> = sqlx::query_as(
        r#"
        SELECT id, session_id, rule_id, severity, summary, evidence_json,
               estimated_save, created_at, dismissed_at
        FROM recommendations
        WHERE session_id = ?1 AND dismissed_at IS NULL
        ORDER BY created_at DESC, id DESC
        "#,
    )
    .bind(&session_id)
    .fetch_all(&state.store.reader)
    .await
    .map_err(|e| {
        tracing::warn!("recommendations list query failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(rows))
}
