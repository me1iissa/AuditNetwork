//! Walk a directory of Claude Code transcripts and load them into the
//! AuditNetwork store. Idempotent: re-running with no source changes is
//! a no-op; growing files are tail-read from the last ingested byte.

use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};
use store::Store;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, BufReader, SeekFrom};

use crate::denormalise::{project, ArtifactRef, ToolProjection};
use crate::parser::{
    parse_ts_to_ms, sha256_hex, text_and_thinking_chars, tool_results, tool_uses, ParsedLine,
};
use crate::redact::redact_value;

const SCHEMA_VERSION: i32 = 1;

#[derive(Debug, Default, Clone)]
pub struct IngestStats {
    pub files_seen: usize,
    pub files_ingested: usize,
    pub events_added: i64,
    pub tool_calls_added: i64,
    pub bytes_read: u64,
}

/// Ingest every `*.jsonl` under `root` recursively. `root` may also be a
/// single `.jsonl` file.
pub async fn ingest_path(store: &Store, root: &Path) -> anyhow::Result<IngestStats> {
    let mut stats = IngestStats::default();
    let files = discover_jsonl(root).await?;
    stats.files_seen = files.len();
    for path in files {
        ingest_one(store, &path, &mut stats).await?;
    }
    Ok(stats)
}

async fn discover_jsonl(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if root.is_file() {
        if root.extension().map(|e| e == "jsonl").unwrap_or(false) {
            out.push(root.to_path_buf());
        }
        return Ok(out);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("read_dir {} failed: {}", dir.display(), e);
                continue;
            }
        };
        while let Some(entry) = rd.next_entry().await? {
            let p = entry.path();
            let ft = entry.file_type().await?;
            if ft.is_dir() {
                stack.push(p);
            } else if p.extension().map(|e| e == "jsonl").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

async fn ingest_one(store: &Store, path: &Path, stats: &mut IngestStats) -> anyhow::Result<()> {
    let meta = tokio::fs::metadata(path).await?;
    let size = meta.len();
    let path_str = path.to_string_lossy().to_string();

    let prior: Option<(String, i64)> = sqlx::query_as(
        "SELECT source_sha256, bytes_ingested FROM ingest_log WHERE source_path = ?1",
    )
    .bind(&path_str)
    .fetch_optional(&store.reader)
    .await?;

    // Always hash the file first so we can detect rewrites that happen to
    // land on the same byte count as a prior ingest (would otherwise look
    // like a no-op).
    let mut file = tokio::fs::File::open(path).await?;
    let full_hash = file_sha256(&mut file).await?;

    let prior_hash = prior.as_ref().map(|(h, _)| h.as_str());
    let prior_bytes = prior.as_ref().map(|(_, b)| *b as u64).unwrap_or(0);

    let rewrite_detected = matches!(prior_hash, Some(h) if h != full_hash) || prior_bytes > size;

    let start_offset = if rewrite_detected {
        tracing::info!(
            "rewrite detected for {} (prior_bytes={prior_bytes}, size={size}, hash_changed={}); purging prior rows and re-ingesting from 0",
            path.display(),
            prior_hash.map(|h| h != full_hash).unwrap_or(false),
        );
        purge_source(store, &path_str).await?;
        0u64
    } else if prior_bytes <= size {
        prior_bytes
    } else {
        0
    };

    if start_offset == size && !rewrite_detected {
        // No new bytes and no rewrite — genuine no-op.
        return Ok(());
    }

    file.seek(SeekFrom::Start(start_offset)).await?;
    let mut reader = BufReader::new(file);

    let mut offset = start_offset;
    let mut events_in_file = 0i64;
    let mut sessions_seen_in_file: std::collections::HashSet<String> = Default::default();
    let mut buf = String::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf).await?;
        if n == 0 {
            break;
        }
        // Skip a half-written tail line that lacks a trailing \n.
        if !buf.ends_with('\n') {
            break;
        }
        let line_offset = offset;
        offset += n as u64;
        stats.bytes_read += n as u64;

        let trimmed = buf.trim_end_matches('\n');
        if trimmed.is_empty() {
            continue;
        }
        let parsed = match ParsedLine::parse(trimmed) {
            Some(p) => p,
            None => {
                tracing::warn!("unparseable JSONL at {}:{}", path.display(), line_offset);
                continue;
            }
        };

        let session_id = match parsed.session_id() {
            Some(s) => s.to_string(),
            None => continue, // skip lines we can't bind to a session
        };
        if sessions_seen_in_file.insert(session_id.clone()) {
            upsert_session(store, &parsed, &session_id, &full_hash).await?;
        }

        let event_inserted = write_event(
            store,
            &parsed,
            &session_id,
            &path_str,
            line_offset,
            &full_hash,
        )
        .await?;
        events_in_file += 1;
        stats.events_added += 1;

        // Per-line specific projections. These are themselves idempotent
        // (INSERT OR IGNORE), so re-running them on a duplicate event is
        // harmless. Token accumulation is NOT idempotent, however — gate
        // it on whether this event is genuinely new.
        write_message_row(store, &parsed, &session_id).await?;
        let tcs_added = write_tool_calls(store, &parsed, &session_id).await?;
        stats.tool_calls_added += tcs_added;
        write_tool_results(store, &parsed, &session_id).await?;
        if event_inserted {
            accumulate_session_tokens(store, &parsed, &session_id).await?;
        }
        write_ai_title(store, &parsed, &session_id).await?;

        // Touch session.ended_at on every event.
        sqlx::query("UPDATE sessions SET ended_at = MAX(ended_at, ?1) WHERE id = ?2")
            .bind(parsed.ts_ms)
            .bind(&session_id)
            .execute(&store.writer)
            .await?;
    }

    // Idempotent UPSERT of ingest_log.
    let now_ms = time::OffsetDateTime::now_utc().unix_timestamp() * 1000;
    sqlx::query(
        "INSERT INTO ingest_log(source_path, source_sha256, ingested_at, bytes_ingested, events_added, schema_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(source_path) DO UPDATE SET
           source_sha256 = excluded.source_sha256,
           ingested_at = excluded.ingested_at,
           bytes_ingested = excluded.bytes_ingested,
           events_added = ingest_log.events_added + excluded.events_added,
           schema_version = excluded.schema_version",
    )
    .bind(&path_str)
    .bind(&full_hash)
    .bind(now_ms)
    .bind(offset as i64)
    .bind(events_in_file)
    .bind(SCHEMA_VERSION)
    .execute(&store.writer)
    .await?;

    if prior_hash != Some(full_hash.as_str()) || events_in_file > 0 {
        stats.files_ingested += 1;
    }
    Ok(())
}

async fn file_sha256(file: &mut tokio::fs::File) -> anyhow::Result<String> {
    file.seek(SeekFrom::Start(0)).await?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

async fn upsert_session(
    store: &Store,
    p: &ParsedLine,
    session_id: &str,
    file_sha: &str,
) -> anyhow::Result<()> {
    let t = &p.transcript;
    let model = t.message.as_ref().and_then(|m| m.model.clone());
    sqlx::query(
        "INSERT INTO sessions(id, started_at, ended_at, claude_version, model, cwd, git_branch, entrypoint, source_file_sha256)
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
           started_at = MIN(sessions.started_at, excluded.started_at),
           claude_version = COALESCE(excluded.claude_version, sessions.claude_version),
           model = COALESCE(excluded.model, sessions.model),
           cwd = COALESCE(excluded.cwd, sessions.cwd),
           git_branch = COALESCE(excluded.git_branch, sessions.git_branch),
           entrypoint = COALESCE(excluded.entrypoint, sessions.entrypoint),
           source_file_sha256 = COALESCE(excluded.source_file_sha256, sessions.source_file_sha256)",
    )
    .bind(session_id)
    .bind(p.ts_ms)
    .bind(t.version.as_deref())
    .bind(model.as_deref())
    .bind(t.cwd.as_deref())
    .bind(t.git_branch.as_deref())
    .bind(t.entrypoint.as_deref())
    .bind(file_sha)
    .execute(&store.writer)
    .await?;
    Ok(())
}

/// Returns `true` if this event was newly inserted (vs. a no-op on the
/// uuid PK). Callers gate non-idempotent work like token accumulation
/// on this signal.
async fn write_event(
    store: &Store,
    p: &ParsedLine,
    session_id: &str,
    source_path: &str,
    offset: u64,
    file_sha: &str,
) -> anyhow::Result<bool> {
    let uuid = match p.transcript.uuid.clone() {
        Some(u) => u,
        None => synthetic_uuid(source_path, offset),
    };
    // Redact raw_json before storage. Parse → walk → re-serialize.
    let mut raw_value: Value = serde_json::from_str(&p.raw).unwrap_or(Value::Null);
    let hits = redact_value(&mut raw_value, "");
    let raw_redacted = serde_json::to_string(&raw_value).unwrap_or_else(|_| p.raw.clone());

    let kind = p.kind_str();
    let is_sidechain = p.transcript.is_sidechain.unwrap_or(false) as i32;

    let res = sqlx::query(
        "INSERT OR IGNORE INTO events(uuid, parent_uuid, session_id, ts, kind, is_sidechain, agent_id, prompt_id, request_id, raw_json, source_path, source_offset, source_sha256)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
    )
    .bind(&uuid)
    .bind(p.transcript.parent_uuid.as_deref())
    .bind(session_id)
    .bind(p.ts_ms)
    .bind(kind)
    .bind(is_sidechain)
    .bind(p.transcript.agent_id.as_deref())
    .bind(p.transcript.prompt_id.as_deref())
    .bind(p.transcript.request_id.as_deref())
    .bind(&raw_redacted)
    .bind(source_path)
    .bind(offset as i64)
    .bind(file_sha)
    .execute(&store.writer)
    .await?;
    let inserted = res.rows_affected() > 0;

    for h in &hits {
        sqlx::query(
            "INSERT OR IGNORE INTO redactions(event_uuid, field_path, rule) VALUES (?1, ?2, ?3)",
        )
        .bind(&uuid)
        .bind(&h.field_path)
        .bind(h.rule)
        .execute(&store.writer)
        .await?;
    }
    Ok(inserted)
}

/// Delete all rows derived from a given source file, in FK-safe order.
/// Called when we detect a rewrite (sha256 changed or file shrunk) so the
/// new bytes can be ingested cleanly without orphaned offsets or stale
/// tool_calls. Sessions and project-scoped artifacts are intentionally
/// preserved — they may be referenced by other source files.
async fn purge_source(store: &Store, source_path: &str) -> anyhow::Result<()> {
    let mut tx = store.writer.begin().await?;

    // Collect the set of event uuids and tool_call ids derived from this
    // source so we can delete dependents before parents.
    let event_uuids: Vec<String> =
        sqlx::query_scalar("SELECT uuid FROM events WHERE source_path = ?1")
            .bind(source_path)
            .fetch_all(&mut *tx)
            .await?;
    if event_uuids.is_empty() {
        // Nothing to purge; still drop the ingest_log row so the next
        // ingest treats the file as fresh.
        sqlx::query("DELETE FROM ingest_log WHERE source_path = ?1")
            .bind(source_path)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(());
    }

    // SQLite has no `IN (?bind list)` for variable-length, so we route
    // through a temp table to keep the SQL simple and the row count
    // unbounded.
    sqlx::query("CREATE TEMP TABLE IF NOT EXISTS _purge_uuids(uuid TEXT PRIMARY KEY)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM _purge_uuids")
        .execute(&mut *tx)
        .await?;
    for u in &event_uuids {
        sqlx::query("INSERT INTO _purge_uuids(uuid) VALUES (?1)")
            .bind(u)
            .execute(&mut *tx)
            .await?;
    }

    // Children of tool_calls first.
    for stmt in [
        "DELETE FROM tool_artifact_edges WHERE tool_call_id IN (SELECT id FROM tool_calls WHERE event_uuid IN (SELECT uuid FROM _purge_uuids))",
        "DELETE FROM file_ops             WHERE tool_call_id IN (SELECT id FROM tool_calls WHERE event_uuid IN (SELECT uuid FROM _purge_uuids))",
        "DELETE FROM bash_commands        WHERE tool_call_id IN (SELECT id FROM tool_calls WHERE event_uuid IN (SELECT uuid FROM _purge_uuids))",
        "DELETE FROM web_fetches          WHERE tool_call_id IN (SELECT id FROM tool_calls WHERE event_uuid IN (SELECT uuid FROM _purge_uuids))",
        "DELETE FROM tool_results         WHERE tool_call_id IN (SELECT id FROM tool_calls WHERE event_uuid IN (SELECT uuid FROM _purge_uuids))",
        "DELETE FROM permission_events    WHERE tool_call_id IN (SELECT id FROM tool_calls WHERE event_uuid IN (SELECT uuid FROM _purge_uuids))",
        "UPDATE subagent_runs SET parent_tool_call_id = NULL WHERE parent_tool_call_id IN (SELECT id FROM tool_calls WHERE event_uuid IN (SELECT uuid FROM _purge_uuids))",
        "DELETE FROM tool_calls           WHERE event_uuid IN (SELECT uuid FROM _purge_uuids)",
        "DELETE FROM messages             WHERE event_uuid IN (SELECT uuid FROM _purge_uuids)",
        "DELETE FROM redactions           WHERE event_uuid IN (SELECT uuid FROM _purge_uuids)",
        "DELETE FROM events               WHERE uuid       IN (SELECT uuid FROM _purge_uuids)",
        "DELETE FROM ingest_log           WHERE source_path = ?1",
    ] {
        let mut q = sqlx::query(stmt);
        if stmt.contains("source_path") {
            q = q.bind(source_path);
        }
        q.execute(&mut *tx).await?;
    }

    sqlx::query("DROP TABLE _purge_uuids")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

fn synthetic_uuid(path: &str, offset: u64) -> String {
    let mut h = Sha256::new();
    h.update(path.as_bytes());
    h.update(offset.to_le_bytes());
    let hex = hex::encode(h.finalize());
    format!("synthetic-{}", &hex[..32])
}

async fn write_message_row(store: &Store, p: &ParsedLine, session_id: &str) -> anyhow::Result<()> {
    let Some(uuid) = p.transcript.uuid.as_deref() else {
        return Ok(());
    };
    let Some(msg) = p.transcript.message.as_ref() else {
        return Ok(());
    };
    let role = msg.role.as_deref().unwrap_or(p.kind_str()).to_string();
    let (text_chars, thinking_chars) = text_and_thinking_chars(p);
    let content_hash = match &msg.content {
        Some(v) => sha256_hex(serde_json::to_string(v).unwrap_or_default().as_bytes()),
        None => String::new(),
    };
    let usage = msg.usage.as_ref();

    sqlx::query(
        "INSERT OR IGNORE INTO messages(event_uuid, session_id, ts, role, model, content_hash, text_chars, thinking_chars, tokens_in, tokens_out, cache_read_tokens, cache_creation_tokens, stop_reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
    )
    .bind(uuid)
    .bind(session_id)
    .bind(p.ts_ms)
    .bind(&role)
    .bind(msg.model.as_deref())
    .bind(&content_hash)
    .bind(text_chars)
    .bind(thinking_chars)
    .bind(usage.and_then(|u| u.input_tokens))
    .bind(usage.and_then(|u| u.output_tokens))
    .bind(usage.and_then(|u| u.cache_read_input_tokens))
    .bind(usage.and_then(|u| u.cache_creation_input_tokens))
    .bind(msg.stop_reason.as_deref())
    .execute(&store.writer)
    .await?;
    Ok(())
}

async fn write_tool_calls(store: &Store, p: &ParsedLine, session_id: &str) -> anyhow::Result<i64> {
    let Some(uuid) = p.transcript.uuid.as_deref() else {
        return Ok(0);
    };
    let is_sidechain = p.transcript.is_sidechain.unwrap_or(false) as i32;
    let agent_id = p.transcript.agent_id.as_deref();

    let mut added = 0i64;
    for (tool_use_id, name, input) in tool_uses(p) {
        // Redact the input before persisting.
        let mut input_redacted = input.clone();
        redact_value(&mut input_redacted, &format!("tool_calls.{name}.input"));
        let input_json = serde_json::to_string(&input_redacted).unwrap_or_default();

        let res = sqlx::query(
            "INSERT OR IGNORE INTO tool_calls(event_uuid, session_id, ts, tool_use_id, tool_name, input_json, is_sidechain, agent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(uuid)
        .bind(session_id)
        .bind(p.ts_ms)
        .bind(&tool_use_id)
        .bind(&name)
        .bind(&input_json)
        .bind(is_sidechain)
        .bind(agent_id)
        .execute(&store.writer)
        .await?;

        if res.rows_affected() == 0 {
            continue;
        }
        added += 1;
        let tc_id: i64 = sqlx::query_scalar(
            "SELECT id FROM tool_calls WHERE tool_use_id = ?1 AND session_id = ?2",
        )
        .bind(&tool_use_id)
        .bind(session_id)
        .fetch_one(&store.reader)
        .await?;

        let projection: ToolProjection = project(&name, &input);
        write_projection(store, tc_id, &name, &projection, p.ts_ms).await?;
    }
    Ok(added)
}

async fn write_projection(
    store: &Store,
    tool_call_id: i64,
    tool_name: &str,
    proj: &ToolProjection,
    ts_ms: i64,
) -> anyhow::Result<()> {
    for a in &proj.artifacts {
        let artifact_id = upsert_artifact(store, a, ts_ms).await?;
        sqlx::query(
            "INSERT OR IGNORE INTO tool_artifact_edges(tool_call_id, artifact_id, access_kind) VALUES (?1, ?2, ?3)",
        )
        .bind(tool_call_id)
        .bind(artifact_id)
        .bind(a.access_kind)
        .execute(&store.writer)
        .await?;
    }
    if let Some(fo) = &proj.file_op {
        // file_ops requires an artifact_id; fetch the file one.
        let aid: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM artifacts WHERE kind='file' AND canonical_key=?1 LIMIT 1",
        )
        .bind(&fo.file_path)
        .fetch_optional(&store.reader)
        .await?;
        if let Some(aid) = aid {
            sqlx::query(
                "INSERT OR IGNORE INTO file_ops(tool_call_id, artifact_id, file_path, op) VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(tool_call_id)
            .bind(aid)
            .bind(&fo.file_path)
            .bind(fo.op)
            .execute(&store.writer)
            .await?;
        }
    }
    if let Some(b) = &proj.bash {
        sqlx::query(
            "INSERT OR IGNORE INTO bash_commands(tool_call_id, command, argv0, command_hash) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(tool_call_id)
        .bind(&b.command)
        .bind(&b.argv0)
        .bind(&b.command_hash)
        .execute(&store.writer)
        .await?;
    }
    if let Some(w) = &proj.web {
        sqlx::query(
            "INSERT OR IGNORE INTO web_fetches(tool_call_id, url, host, path, url_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(tool_call_id)
        .bind(&w.url)
        .bind(&w.host)
        .bind(&w.path)
        .bind(&w.url_hash)
        .execute(&store.writer)
        .await?;
    }
    let _ = tool_name;
    Ok(())
}

async fn upsert_artifact(store: &Store, a: &ArtifactRef, ts_ms: i64) -> anyhow::Result<i64> {
    sqlx::query(
        "INSERT INTO artifacts(kind, canonical_key, display, first_seen_ts, last_seen_ts)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(kind, canonical_key, project_id) DO UPDATE SET
           last_seen_ts = MAX(artifacts.last_seen_ts, excluded.last_seen_ts)",
    )
    .bind(a.kind.as_str())
    .bind(&a.canonical_key)
    .bind(&a.display)
    .bind(ts_ms)
    .execute(&store.writer)
    .await?;
    let id: i64 = sqlx::query_scalar(
        "SELECT id FROM artifacts WHERE kind=?1 AND canonical_key=?2 AND project_id IS NULL",
    )
    .bind(a.kind.as_str())
    .bind(&a.canonical_key)
    .fetch_one(&store.reader)
    .await?;
    Ok(id)
}

async fn write_tool_results(store: &Store, p: &ParsedLine, session_id: &str) -> anyhow::Result<()> {
    for (tool_use_id, is_error, content) in tool_results(p) {
        // Session-scoped lookup: the same tool_use_id can legitimately
        // appear in multiple sessions (UNIQUE is on (tool_use_id, session_id)),
        // so attaching the result by id alone could land it on the wrong row.
        let tc_id: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM tool_calls WHERE tool_use_id = ?1 AND session_id = ?2",
        )
        .bind(&tool_use_id)
        .bind(session_id)
        .fetch_optional(&store.reader)
        .await?;
        let Some(tc_id) = tc_id else { continue };

        let started_at: Option<i64> = sqlx::query_scalar("SELECT ts FROM tool_calls WHERE id = ?1")
            .bind(tc_id)
            .fetch_optional(&store.reader)
            .await?;
        let duration_ms = started_at.map(|s| p.ts_ms - s);

        let output_bytes = content
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_default().len() as i64)
            .unwrap_or(0);

        let is_err_int = is_error.unwrap_or(false) as i32;
        sqlx::query(
            "INSERT OR IGNORE INTO tool_results(tool_call_id, ts, output_bytes, is_error) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(tc_id)
        .bind(p.ts_ms)
        .bind(output_bytes)
        .bind(is_err_int)
        .execute(&store.writer)
        .await?;
        sqlx::query(
            "UPDATE tool_calls SET duration_ms = ?1, success = ?2, error_kind = CASE WHEN ?3 = 1 THEN 'tool_error' ELSE NULL END WHERE id = ?4",
        )
        .bind(duration_ms)
        .bind((!is_error.unwrap_or(false)) as i32)
        .bind(is_err_int)
        .bind(tc_id)
        .execute(&store.writer)
        .await?;
    }
    Ok(())
}

async fn accumulate_session_tokens(
    store: &Store,
    p: &ParsedLine,
    session_id: &str,
) -> anyhow::Result<()> {
    let Some(u) = p.usage() else { return Ok(()) };
    sqlx::query(
        "UPDATE sessions SET
           total_input_tokens    = total_input_tokens    + COALESCE(?1, 0),
           total_output_tokens   = total_output_tokens   + COALESCE(?2, 0),
           total_cache_read      = total_cache_read      + COALESCE(?3, 0),
           total_cache_creation  = total_cache_creation  + COALESCE(?4, 0)
         WHERE id = ?5",
    )
    .bind(u.input_tokens)
    .bind(u.output_tokens)
    .bind(u.cache_read_input_tokens)
    .bind(u.cache_creation_input_tokens)
    .bind(session_id)
    .execute(&store.writer)
    .await?;
    Ok(())
}

async fn write_ai_title(store: &Store, p: &ParsedLine, session_id: &str) -> anyhow::Result<()> {
    if let Some(title) = p.transcript.ai_title.as_deref() {
        sqlx::query("UPDATE sessions SET ai_title = ?1 WHERE id = ?2")
            .bind(title)
            .bind(session_id)
            .execute(&store.writer)
            .await?;
    }
    Ok(())
}

#[allow(dead_code)]
fn parse_ts_ms_or_zero(s: Option<&str>) -> i64 {
    s.and_then(parse_ts_to_ms).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    async fn temp_db() -> (Store, tempdir::TempPath) {
        let tmp = tempdir::TempPath::new();
        let store = Store::open(&tmp.path).await.unwrap();
        (store, tmp)
    }

    fn assistant_line(uuid: &str, session_id: &str, ts: &str, tool_use_id: &str) -> String {
        format!(
            r#"{{"type":"assistant","uuid":"{uuid}","timestamp":"{ts}","sessionId":"{session_id}","message":{{"role":"assistant","model":"claude-test","content":[{{"type":"tool_use","id":"{tool_use_id}","name":"Bash","input":{{"command":"echo hi"}}}}],"usage":{{"input_tokens":1,"output_tokens":2}}}}}}"#
        )
    }

    fn tool_result_line(
        uuid: &str,
        parent_uuid: &str,
        session_id: &str,
        ts: &str,
        tool_use_id: &str,
    ) -> String {
        format!(
            r#"{{"type":"user","uuid":"{uuid}","parentUuid":"{parent_uuid}","timestamp":"{ts}","sessionId":"{session_id}","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"{tool_use_id}","content":"ok","is_error":false}}]}}}}"#
        )
    }

    async fn write_jsonl(path: &Path, lines: &[String]) {
        let mut f = tokio::fs::File::create(path).await.unwrap();
        for l in lines {
            f.write_all(l.as_bytes()).await.unwrap();
            f.write_all(b"\n").await.unwrap();
        }
        f.sync_all().await.unwrap();
    }

    #[tokio::test]
    async fn idempotent_reingest_does_not_double_count_tokens() {
        let (store, _db) = temp_db().await;
        let tmp = tempdir::TempPath::new();
        let p = tmp.path.with_extension("jsonl");
        write_jsonl(
            &p,
            &[assistant_line("u1", "s1", "2026-05-16T00:00:00Z", "t1")],
        )
        .await;

        let _ = ingest_path(&store, &p).await.unwrap();
        let _ = ingest_path(&store, &p).await.unwrap();

        let totals: (i64, i64) = sqlx::query_as(
            "SELECT total_input_tokens, total_output_tokens FROM sessions WHERE id='s1'",
        )
        .fetch_one(&store.reader)
        .await
        .unwrap();
        assert_eq!(totals, (1, 2));
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn rewrite_with_same_size_purges_and_reingests() {
        let (store, _db) = temp_db().await;
        let tmp = tempdir::TempPath::new();
        let p = tmp.path.with_extension("jsonl");

        // First ingest: one event in session s1.
        write_jsonl(
            &p,
            &[assistant_line("aaaa", "s1", "2026-05-16T00:00:00Z", "t1")],
        )
        .await;
        ingest_path(&store, &p).await.unwrap();

        // Same byte count, different content (different uuid + session).
        // The reviewer's bug: without sha-first detection this is a no-op.
        let line_old = assistant_line("aaaa", "s1", "2026-05-16T00:00:00Z", "t1");
        let line_new = assistant_line("bbbb", "s2", "2026-05-16T00:00:00Z", "t2");
        assert_eq!(line_old.len(), line_new.len(), "fixture must match length");
        write_jsonl(&p, &[line_new]).await;
        ingest_path(&store, &p).await.unwrap();

        // The old event must be gone, the new one present.
        let n_aaaa: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE uuid='aaaa'")
            .fetch_one(&store.reader)
            .await
            .unwrap();
        let n_bbbb: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE uuid='bbbb'")
            .fetch_one(&store.reader)
            .await
            .unwrap();
        assert_eq!(n_aaaa, 0, "old event should be purged");
        assert_eq!(n_bbbb, 1, "new event should be ingested");
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn truncation_to_smaller_size_resets_offset() {
        let (store, _db) = temp_db().await;
        let tmp = tempdir::TempPath::new();
        let p = tmp.path.with_extension("jsonl");

        // First: two events.
        write_jsonl(
            &p,
            &[
                assistant_line("u1", "s1", "2026-05-16T00:00:00Z", "t1"),
                assistant_line("u2", "s1", "2026-05-16T00:00:01Z", "t2"),
            ],
        )
        .await;
        ingest_path(&store, &p).await.unwrap();

        // Truncate-and-rewrite with one event.
        write_jsonl(
            &p,
            &[assistant_line("u3", "s1", "2026-05-16T00:00:02Z", "t3")],
        )
        .await;
        ingest_path(&store, &p).await.unwrap();

        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE source_path = ?1")
            .bind(p.to_string_lossy().to_string())
            .fetch_one(&store.reader)
            .await
            .unwrap();
        assert_eq!(n, 1, "old events should be purged after truncation");
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn tool_result_lookup_is_session_scoped() {
        let (store, _db) = temp_db().await;
        let tmp_a = tempdir::TempPath::new();
        let tmp_b = tempdir::TempPath::new();
        let pa = tmp_a.path.with_extension("jsonl");
        let pb = tmp_b.path.with_extension("jsonl");

        // Two sessions, same tool_use_id collides on different timestamps.
        // Session A: tool_use at t=0, tool_result at t=10 (10ms duration).
        write_jsonl(
            &pa,
            &[
                assistant_line("a1", "sA", "2026-05-16T00:00:00.000Z", "shared"),
                tool_result_line("a2", "a1", "sA", "2026-05-16T00:00:00.010Z", "shared"),
            ],
        )
        .await;
        // Session B: tool_use at t=0, tool_result at t=999 (999ms duration).
        write_jsonl(
            &pb,
            &[
                assistant_line("b1", "sB", "2026-05-16T00:00:00.000Z", "shared"),
                tool_result_line("b2", "b1", "sB", "2026-05-16T00:00:00.999Z", "shared"),
            ],
        )
        .await;
        ingest_path(&store, &pa).await.unwrap();
        ingest_path(&store, &pb).await.unwrap();

        let dur_a: Option<i64> = sqlx::query_scalar(
            "SELECT duration_ms FROM tool_calls WHERE session_id='sA' AND tool_use_id='shared'",
        )
        .fetch_one(&store.reader)
        .await
        .unwrap();
        let dur_b: Option<i64> = sqlx::query_scalar(
            "SELECT duration_ms FROM tool_calls WHERE session_id='sB' AND tool_use_id='shared'",
        )
        .fetch_one(&store.reader)
        .await
        .unwrap();
        assert_eq!(dur_a, Some(10), "session A tool_result must attach to A");
        assert_eq!(dur_b, Some(999), "session B tool_result must attach to B");
        let _ = std::fs::remove_file(&pa);
        let _ = std::fs::remove_file(&pb);
    }

    /// Tiny stand-in for the `tempfile` crate so we avoid a new dependency.
    /// Not robust — sufficient for these tests.
    mod tempdir {
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicU64, Ordering};

        static SEQ: AtomicU64 = AtomicU64::new(0);
        pub struct TempPath {
            pub path: PathBuf,
        }
        impl TempPath {
            pub fn new() -> Self {
                let n = SEQ.fetch_add(1, Ordering::SeqCst);
                let pid = std::process::id();
                let path = std::env::temp_dir().join(format!("an_test_{pid}_{n}"));
                Self { path }
            }
        }
        impl Drop for TempPath {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.path);
                // Best-effort cleanup of sqlite sidecars and jsonl alias.
                for ext in ["db-wal", "db-shm", "jsonl", "jsonl-wal", "jsonl-shm"] {
                    let _ = std::fs::remove_file(self.path.with_extension(ext));
                }
            }
        }
    }
}
