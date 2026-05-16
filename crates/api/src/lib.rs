//! HTTP API for AuditNetwork. Read-only over the SQLite warehouse.
//!
//! Endpoints (M2):
//! - `GET  /healthz`               → liveness probe
//! - `GET  /readyz`                → readiness (DB SELECT 1)
//! - `GET  /api/sessions`          → list sessions with summary metrics
//! - `GET  /api/sessions/:id`      → session detail
//! - `GET  /api/sessions/:id/graph?mode=bipartite|causal`
//!
//! M3 will add WebSocket replay; M4 will add the SQL query playground.

use std::sync::Arc;

use axum::{routing::get, Router};
use store::Store;
use tower_http::cors::CorsLayer;

mod graph;
mod health;
mod sessions;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
}

pub fn router(store: Store) -> Router {
    let state = Arc::new(AppState { store });
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .route("/api/sessions", get(sessions::list))
        .route("/api/sessions/:id", get(sessions::detail))
        .route("/api/sessions/:id/graph", get(graph::session_graph))
        // Liberal CORS in dev so the Vite dev server at :5173 can hit the
        // backend at :8080. Production builds will serve the SPA from the
        // same origin via rust-embed and won't need this layer.
        .layer(CorsLayer::very_permissive())
        .with_state(state)
}
