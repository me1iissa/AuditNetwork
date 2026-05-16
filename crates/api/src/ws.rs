//! WebSocket endpoint at `/ws`. M3 uses it for **replay control**: the
//! frontend already has the full graph from `/api/sessions/:id/graph`,
//! and the server's job here is to emit a cursor that advances through
//! virtual session time. The client uses that cursor to fade nodes and
//! edges in by `ts` so the spider grows over the timeline.
//!
//! Same socket is intended to carry M5 live tailing (server emits
//! `kind: "event"` when a new tool_use lands on disk), which is why
//! `replay_open` carries a session id rather than being a global stream.
//!
//! Protocol
//! ========
//!
//! Client → Server (text frames, JSON):
//!   {"op":"replay_open","session_id":"<uuid>"[,"speed":1.0][,"from_ts":<ms>]}
//!   {"op":"replay_control","action":"play|pause|seek|speed","value":<num>?}
//!   {"op":"ping"}
//!
//! Server → Client:
//!   {"kind":"replay_bounds","from_ts":<ms>,"to_ts":<ms>,"speed":<f>,"playing":<bool>}
//!   {"kind":"cursor","ts_ms":<ms>}
//!   {"kind":"error","message":"<text>"}
//!   {"kind":"pong"}
//!
//! Pause / seek / speed are inflight-safe — the pacer task reads the
//! shared `Replay` state on each tick so changes apply within ≤ tick
//! interval (50 ms).

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};

use crate::AppState;

/// Pacer tick. 50 ms keeps cursor updates feeling continuous without
/// flooding the wire (~20 messages/sec).
const TICK: Duration = Duration::from_millis(50);

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ClientMsg {
    ReplayOpen {
        session_id: String,
        #[serde(default = "default_speed")]
        speed: f64,
        #[serde(default)]
        from_ts: Option<i64>,
    },
    ReplayControl {
        action: String,
        #[serde(default)]
        value: Option<f64>,
    },
    Ping,
}

fn default_speed() -> f64 {
    1.0
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ServerMsg {
    ReplayBounds {
        from_ts: i64,
        to_ts: i64,
        speed: f64,
        playing: bool,
    },
    Cursor {
        ts_ms: i64,
    },
    Error {
        message: String,
    },
    Pong,
}

#[derive(Debug, Default)]
struct Replay {
    session_id: Option<String>,
    from_ts: i64,
    to_ts: i64,
    cursor: i64,
    speed: f64,
    playing: bool,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run_socket(state, socket))
}

async fn run_socket(state: Arc<AppState>, socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();

    let replay: Arc<Mutex<Replay>> = Arc::new(Mutex::new(Replay::default()));

    // One mpsc fans all outgoing messages — control acks, replay_bounds,
    // pacer cursors — into the single sink. This is how we get a recv
    // loop and a pacer task without contending for the WebSocket.
    let (out_tx, mut out_rx) = mpsc::channel::<ServerMsg>(64);

    let sender_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            let json = match serde_json::to_string(&msg) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("ws serialise failed: {e}");
                    continue;
                }
            };
            if sender.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
        let _ = sender.close().await;
    });

    let pacer_replay = replay.clone();
    let pacer_tx = out_tx.clone();
    let pacer_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            let (playing, speed, cursor, to_ts) = {
                let r = pacer_replay.lock().await;
                (r.playing, r.speed, r.cursor, r.to_ts)
            };
            if !playing || to_ts == 0 || cursor >= to_ts {
                continue;
            }
            let advance_ms = (TICK.as_millis() as f64 * speed) as i64;
            let new_cursor = (cursor + advance_ms.max(1)).min(to_ts);
            {
                let mut r = pacer_replay.lock().await;
                r.cursor = new_cursor;
                if new_cursor >= r.to_ts {
                    r.playing = false;
                }
            }
            if pacer_tx
                .send(ServerMsg::Cursor { ts_ms: new_cursor })
                .await
                .is_err()
            {
                break;
            }
        }
    });

    while let Some(frame) = receiver.next().await {
        let frame = match frame {
            Ok(f) => f,
            Err(e) => {
                tracing::debug!("ws recv error: {e}");
                break;
            }
        };
        let text = match frame {
            Message::Text(t) => t,
            Message::Close(_) => break,
            Message::Ping(p) => {
                // axum auto-handles low-level pings; this branch just
                // never fires in practice.
                let _ = p;
                continue;
            }
            _ => continue,
        };

        match serde_json::from_str::<ClientMsg>(&text) {
            Ok(ClientMsg::Ping) => {
                let _ = out_tx.send(ServerMsg::Pong).await;
            }
            Ok(ClientMsg::ReplayOpen {
                session_id,
                speed,
                from_ts,
            }) => {
                handle_open(&state, &replay, &out_tx, session_id, speed, from_ts).await;
            }
            Ok(ClientMsg::ReplayControl { action, value }) => {
                handle_control(&replay, &out_tx, &action, value).await;
            }
            Err(e) => {
                let _ = out_tx
                    .send(ServerMsg::Error {
                        message: format!("invalid message: {e}"),
                    })
                    .await;
            }
        }
    }

    // Receiver loop exited (close frame or client gone). Drop the sender
    // channel so the sender task exits, then cancel the pacer.
    drop(out_tx);
    pacer_task.abort();
    let _ = sender_task.await;
}

async fn handle_open(
    state: &Arc<AppState>,
    replay: &Arc<Mutex<Replay>>,
    out: &mpsc::Sender<ServerMsg>,
    session_id: String,
    speed: f64,
    from_ts: Option<i64>,
) {
    // Filter out ts=0 — some event kinds (`ai-title`, `queue-operation`)
    // arrive without a timestamp and would otherwise drag `from_ts` to
    // epoch, making the timeline span decades.
    type Bounds = (Option<i64>, Option<i64>);
    let bounds_q: Result<Option<Bounds>, sqlx::Error> =
        sqlx::query_as("SELECT MIN(ts), MAX(ts) FROM events WHERE session_id = ?1 AND ts > 0")
            .bind(&session_id)
            .fetch_optional(&state.store.reader)
            .await;
    let bounds = match bounds_q {
        Ok(Some((Some(lo), Some(hi)))) if hi > 0 => (lo, hi),
        Ok(_) => {
            let _ = out
                .send(ServerMsg::Error {
                    message: format!("unknown or empty session: {session_id}"),
                })
                .await;
            return;
        }
        Err(e) => {
            let _ = out
                .send(ServerMsg::Error {
                    message: format!("db error: {e}"),
                })
                .await;
            return;
        }
    };

    let (from_db, to_ts) = bounds;
    let cursor = from_ts.unwrap_or(from_db).clamp(from_db, to_ts);

    {
        let mut r = replay.lock().await;
        r.session_id = Some(session_id);
        r.from_ts = from_db;
        r.to_ts = to_ts;
        r.cursor = cursor;
        r.speed = speed.max(0.0);
        r.playing = false;
    }
    let _ = out
        .send(ServerMsg::ReplayBounds {
            from_ts: from_db,
            to_ts,
            speed,
            playing: false,
        })
        .await;
    let _ = out.send(ServerMsg::Cursor { ts_ms: cursor }).await;
}

async fn handle_control(
    replay: &Arc<Mutex<Replay>>,
    out: &mpsc::Sender<ServerMsg>,
    action: &str,
    value: Option<f64>,
) {
    let mut snapshot_cursor: Option<i64> = None;
    {
        let mut r = replay.lock().await;
        match action {
            "play" => {
                if r.cursor >= r.to_ts {
                    // At the end — rewind to the start on play.
                    r.cursor = r.from_ts;
                    snapshot_cursor = Some(r.cursor);
                }
                r.playing = true;
            }
            "pause" => r.playing = false,
            "speed" => {
                if let Some(v) = value {
                    r.speed = v.max(0.0);
                }
            }
            "seek" => {
                if let Some(v) = value {
                    let target = (v as i64).clamp(r.from_ts, r.to_ts);
                    r.cursor = target;
                    snapshot_cursor = Some(target);
                }
            }
            _ => {}
        }
    }
    if let Some(cursor) = snapshot_cursor {
        let _ = out.send(ServerMsg::Cursor { ts_ms: cursor }).await;
    }
}
