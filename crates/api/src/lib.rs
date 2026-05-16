//! HTTP API for AuditNetwork. Read-only over the SQLite warehouse.
//!
//! Endpoints:
//! - `GET  /healthz`                         → liveness probe
//! - `GET  /readyz`                          → readiness (DB SELECT 1)
//! - `GET  /api/sessions`                    → list sessions with summary metrics
//! - `GET  /api/sessions/:id`                → session detail
//! - `GET  /api/sessions/:id/graph?mode=…`   → bipartite or causal graph
//! - `GET  /api/tool_calls/:id`              → tool_call detail for the panel (M3)
//! - `GET  /api/artifacts/:id?session_id=…`  → artifact detail + per-session touches (M3)
//! - `GET  /ws`                              → WebSocket replay control + cursor stream (M3)
//!
//! M4 will add the SQL query playground; M5 will add live tailing on the
//! same `/ws` endpoint.

use std::sync::Arc;

use axum::{routing::get, Router};
use store::Store;
use tower_http::cors::CorsLayer;

mod detail;
mod graph;
mod health;
mod sessions;
mod ws;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
}

/// Build the application router.
///
/// `dev_cors` opts into `CorsLayer::very_permissive()`, which lets the
/// Vite dev server at `:5173` call the Rust backend at `:8080` without
/// auth. **Do not enable this on a public bind** — combined with
/// `AN_BIND=0.0.0.0:8080` it would let any origin read all session
/// metadata. In production (M6) the SPA is served from the same origin
/// as the API via `rust-embed`, so no CORS layer is needed.
pub fn router(store: Store, dev_cors: bool) -> Router {
    let state = Arc::new(AppState { store });
    let base = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .route("/api/sessions", get(sessions::list))
        .route("/api/sessions/:id", get(sessions::detail))
        .route("/api/sessions/:id/graph", get(graph::session_graph))
        .route("/api/tool_calls/:id", get(detail::tool_call))
        .route("/api/artifacts/:id", get(detail::artifact))
        .route("/ws", get(ws::ws_handler));
    let r = if dev_cors {
        tracing::warn!(
            "permissive CORS enabled — only safe behind 127.0.0.1 or a trusted reverse proxy"
        );
        base.layer(CorsLayer::very_permissive())
    } else {
        base
    };
    r.with_state(state)
}
