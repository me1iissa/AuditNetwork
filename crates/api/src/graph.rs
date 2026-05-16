use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::AppState;

#[derive(Deserialize)]
pub struct GraphParams {
    /// `bipartite` (default) — tools ↔ artifacts.
    /// `causal`              — events linked by parent_uuid.
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Serialize)]
pub struct GraphResponse {
    pub mode: &'static str,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Serialize)]
pub struct Node {
    pub id: String,
    pub kind: String, // bipartite: 'tool_call' | 'artifact'   |  causal: 'event'
    pub label: String,
    /// Earliest event timestamp this node participates in, ms-since-epoch.
    pub ts: i64,
    /// Optional sub-typing — for tool_call nodes: tool name; for artifact: file/url/command/...
    /// For event nodes: the event kind ('user'/'assistant'/...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
}

#[derive(Serialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub ts: i64,
    /// 'read'|'write'|'edit'|'fetch'|'exec'|'list'|'grep' for bipartite,
    /// 'parent' for causal.
    pub label: String,
}

pub async fn session_graph(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<GraphParams>,
) -> Result<Json<GraphResponse>, StatusCode> {
    let mode = params.mode.as_deref().unwrap_or("bipartite");
    // Surface unknown session ids as 404 instead of an empty graph — consistent
    // with sessions::detail and avoids hiding typos in the SPA.
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM sessions WHERE id = ?1")
        .bind(&id)
        .fetch_optional(&state.store.reader)
        .await
        .map_err(map_db_err)?;
    if exists.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    match mode {
        "bipartite" => bipartite(state, &id).await,
        "causal" => causal(state, &id).await,
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

async fn bipartite(
    state: Arc<AppState>,
    session_id: &str,
) -> Result<Json<GraphResponse>, StatusCode> {
    // Tool-call nodes for this session.
    let tool_rows: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT id, tool_name, ts FROM tool_calls WHERE session_id = ?1 ORDER BY ts",
    )
    .bind(session_id)
    .fetch_all(&state.store.reader)
    .await
    .map_err(map_db_err)?;

    // Artifact nodes touched by this session, with the earliest tool_call
    // ts that produced an edge to the artifact (drives timeline fade-in).
    let art_rows: Vec<(i64, String, String, String, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT a.id, a.kind, a.canonical_key, a.display, MIN(tc.ts) AS first_ts
        FROM artifacts a
        JOIN tool_artifact_edges tae ON tae.artifact_id = a.id
        JOIN tool_calls tc           ON tc.id          = tae.tool_call_id
        WHERE tc.session_id = ?1
        GROUP BY a.id
        "#,
    )
    .bind(session_id)
    .fetch_all(&state.store.reader)
    .await
    .map_err(map_db_err)?;

    // Edges: each (tool_call, artifact, access_kind) triple within this
    // session's tool_calls.
    let edge_rows: Vec<(i64, i64, String, i64)> = sqlx::query_as(
        r#"
        SELECT tae.tool_call_id, tae.artifact_id, tae.access_kind, tc.ts
        FROM tool_artifact_edges tae
        JOIN tool_calls tc ON tc.id = tae.tool_call_id
        WHERE tc.session_id = ?1
        ORDER BY tc.ts
        "#,
    )
    .bind(session_id)
    .fetch_all(&state.store.reader)
    .await
    .map_err(map_db_err)?;

    let mut nodes: Vec<Node> = Vec::with_capacity(tool_rows.len() + art_rows.len());
    for (id, name, ts) in tool_rows {
        nodes.push(Node {
            id: format!("tc:{id}"),
            kind: "tool_call".into(),
            label: name.clone(),
            ts,
            sub: Some(name),
        });
    }
    for (id, kind, _key, display, first_ts) in art_rows {
        nodes.push(Node {
            id: format!("art:{id}"),
            kind: "artifact".into(),
            label: display,
            ts: first_ts.unwrap_or(0),
            sub: Some(kind),
        });
    }

    let edges = edge_rows
        .into_iter()
        .map(|(tc, art, ak, ts)| Edge {
            source: format!("tc:{tc}"),
            target: format!("art:{art}"),
            ts,
            label: ak,
        })
        .collect();

    Ok(Json(GraphResponse {
        mode: "bipartite",
        nodes,
        edges,
    }))
}

async fn causal(state: Arc<AppState>, session_id: &str) -> Result<Json<GraphResponse>, StatusCode> {
    let event_rows: Vec<(String, Option<String>, i64, String)> = sqlx::query_as(
        "SELECT uuid, parent_uuid, ts, kind FROM events WHERE session_id = ?1 ORDER BY ts",
    )
    .bind(session_id)
    .fetch_all(&state.store.reader)
    .await
    .map_err(map_db_err)?;

    let nodes: Vec<Node> = event_rows
        .iter()
        .map(|(uuid, _, ts, kind)| Node {
            id: uuid.clone(),
            kind: "event".into(),
            label: kind.clone(),
            ts: *ts,
            sub: Some(kind.clone()),
        })
        .collect();

    // Only emit edges whose parent is also in this session — cross-session
    // parent links (rare but possible with sidechain transcripts) would
    // dangle on the client and trigger a Cytoscape "nonexistent source"
    // warning.
    let in_session: std::collections::HashSet<&str> =
        event_rows.iter().map(|(u, _, _, _)| u.as_str()).collect();
    let edges: Vec<Edge> = event_rows
        .iter()
        .filter_map(|(uuid, parent, ts, _)| {
            parent
                .as_ref()
                .filter(|p| in_session.contains(p.as_str()))
                .map(|p| Edge {
                    source: p.clone(),
                    target: uuid.clone(),
                    ts: *ts,
                    label: "parent".into(),
                })
        })
        .collect();

    Ok(Json(GraphResponse {
        mode: "causal",
        nodes,
        edges,
    }))
}

fn map_db_err(e: sqlx::Error) -> StatusCode {
    tracing::warn!("graph query failed: {e}");
    StatusCode::INTERNAL_SERVER_ERROR
}
