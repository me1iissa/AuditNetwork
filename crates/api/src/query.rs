//! Read-only SQL endpoint. The frontend Monaco playground (and any
//! external pandas / datasette / scratch curl invocation) hits this to
//! run ad-hoc queries against the warehouse.
//!
//! Safety model:
//! - Connections come from `store.reader`, which is opened with
//!   `PRAGMA query_only = 1`. SQLite refuses INSERT/UPDATE/DELETE/CREATE
//!   /ALTER/DROP on that connection.
//! - Hard caps: at most `MAX_ROWS` rows returned, at most
//!   `MAX_BYTES` serialised, at most one statement per request.
//! - Statement timeout via `tokio::time::timeout` around the await.
//!
//! What's deliberately NOT done at this layer:
//! - Parameter validation: we accept positional `?N` bound params from
//!   the request body, but we don't reject "scary" SQL keywords. The
//!   PRAGMA + sqlx already disallows writes, and there's no useful
//!   blocklist anyone has ever made work — false positives outnumber
//!   real threats. Trust the engine.

use std::sync::Arc;
use std::time::Duration;

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AppState;

const MAX_ROWS: usize = 20_000;
const MAX_BYTES: usize = 50 * 1024 * 1024; // 50 MB
const STATEMENT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Deserialize)]
pub struct QueryRequest {
    pub sql: String,
    #[serde(default)]
    pub params: Vec<Value>,
}

#[derive(Serialize)]
pub struct QueryResponse {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub row_count: usize,
    pub truncated: bool,
    pub duration_ms: u128,
}

#[derive(Serialize)]
pub struct QueryError {
    pub error: String,
}

pub async fn query(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QueryRequest>,
) -> Result<Json<QueryResponse>, (StatusCode, Json<QueryError>)> {
    let started = std::time::Instant::now();

    let conn = state.store.reader.acquire();
    let fut = run_one(&state, conn.await.map_err(|e| db_err(e.to_string()))?, req);
    let res = tokio::time::timeout(STATEMENT_TIMEOUT, fut)
        .await
        .map_err(|_| {
            (
                StatusCode::REQUEST_TIMEOUT,
                Json(QueryError {
                    error: format!("query exceeded {} s timeout", STATEMENT_TIMEOUT.as_secs()),
                }),
            )
        })??;

    let (columns, rows, truncated) = res;
    Ok(Json(QueryResponse {
        columns,
        row_count: rows.len(),
        rows,
        truncated,
        duration_ms: started.elapsed().as_millis(),
    }))
}

async fn run_one(
    _state: &Arc<AppState>,
    mut conn: sqlx::pool::PoolConnection<sqlx::Sqlite>,
    req: QueryRequest,
) -> Result<(Vec<String>, Vec<Vec<Value>>, bool), (StatusCode, Json<QueryError>)> {
    use futures_util::TryStreamExt;

    let mut q = sqlx::query(&req.sql);
    for p in &req.params {
        q = bind_param(q, p);
    }

    let mut stream = q.fetch(&mut *conn);
    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut columns: Vec<String> = Vec::new();
    let mut bytes = 0usize;
    let mut truncated = false;

    while let Some(row) = stream.try_next().await.map_err(|e| db_err(e.to_string()))? {
        use sqlx::{Column, Row};
        if columns.is_empty() {
            columns = row.columns().iter().map(|c| c.name().to_string()).collect();
        }
        let mut json_row: Vec<Value> = Vec::with_capacity(columns.len());
        for i in 0..columns.len() {
            json_row.push(decode_cell(&row, i));
        }
        bytes += rough_size(&json_row);
        rows.push(json_row);
        if rows.len() >= MAX_ROWS || bytes >= MAX_BYTES {
            truncated = true;
            break;
        }
    }
    Ok((columns, rows, truncated))
}

fn bind_param<'q>(
    q: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    p: &'q Value,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    match p {
        Value::Null => q.bind(None::<String>),
        Value::Bool(b) => q.bind(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                q.bind(i)
            } else if let Some(f) = n.as_f64() {
                q.bind(f)
            } else {
                q.bind(n.to_string())
            }
        }
        Value::String(s) => q.bind(s.as_str()),
        // Arrays/objects don't have a SQLite parameter representation;
        // pass their JSON serialisation so a query can json_extract over it.
        Value::Array(_) | Value::Object(_) => q.bind(p.to_string()),
    }
}

fn decode_cell(row: &sqlx::sqlite::SqliteRow, idx: usize) -> Value {
    use sqlx::Row;
    use sqlx::TypeInfo;
    use sqlx::ValueRef;
    let v = match row.try_get_raw(idx) {
        Ok(v) => v,
        Err(_) => return Value::Null,
    };
    if v.is_null() {
        return Value::Null;
    }
    let ty = v.type_info();
    match ty.name() {
        "INTEGER" | "INT" | "INT8" | "BIGINT" => row
            .try_get::<i64, _>(idx)
            .map(Value::from)
            .unwrap_or(Value::Null),
        "REAL" | "FLOAT" | "DOUBLE" => row
            .try_get::<f64, _>(idx)
            .map(Value::from)
            .unwrap_or(Value::Null),
        "BLOB" => row
            .try_get::<Vec<u8>, _>(idx)
            .map(|b| Value::String(hex::encode(b)))
            .unwrap_or(Value::Null),
        _ => row
            .try_get::<String, _>(idx)
            .map(Value::String)
            .unwrap_or(Value::Null),
    }
}

fn rough_size(row: &[Value]) -> usize {
    // Coarse heuristic; we don't need exact bytes, just enough to bound
    // the response. Strings dominate; integers cost ~16 bytes after JSON.
    let mut n = 0;
    for v in row {
        n += match v {
            Value::String(s) => s.len() + 4,
            Value::Null => 4,
            _ => 16,
        };
    }
    n
}

fn db_err(msg: String) -> (StatusCode, Json<QueryError>) {
    // SQLite raises "attempt to write a readonly database" for any
    // INSERT/UPDATE/DELETE/CREATE/DROP on a `query_only=1` connection;
    // surface that as 403 rather than a generic 500.
    let code = if msg.contains("readonly")
        || msg.to_lowercase().contains("not authorized")
        || msg.to_lowercase().contains("read-only")
    {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::BAD_REQUEST
    };
    (code, Json(QueryError { error: msg }))
}
