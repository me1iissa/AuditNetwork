//! Walk a directory of Claude Code transcripts and load them into the
//! AuditNetwork store. Idempotent: re-running with no source changes is
//! a no-op; growing files are tail-read from the last ingested byte.

use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};
use store::Store;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, BufReader, SeekFrom};

use crate::denormalise::{project, ArtifactRef, ToolProjection};
use crate::parser::{parse_ts_to_ms, sha256_hex, text_and_thinking_chars, tool_results, tool_uses, ParsedLine};
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

    let (start_offset, prior_hash) = match &prior {
        Some((h, b)) if (*b as u64) <= size => (*b as u64, Some(h.clone())),
        _ => (0u64, None),
    };

    if start_offset == size {
        // Nothing new.
        return Ok(());
    }

    let mut file = tokio::fs::File::open(path).await?;
    // Full-file hash (cheap on these files; needed for provenance).
    let full_hash = file_sha256(&mut file).await?;
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

        write_event(store, &parsed, &session_id, &path_str, line_offset, &full_hash).await?;
        events_in_file += 1;
        stats.events_added += 1;

        // Per-line specific projections.
        write_message_row(store, &parsed, &session_id).await?;
        let tcs_added = write_tool_calls(store, &parsed, &session_id).await?;
        stats.tool_calls_added += tcs_added;
        write_tool_results(store, &parsed).await?;
        accumulate_session_tokens(store, &parsed, &session_id).await?;
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

    if prior_hash.as_deref() != Some(full_hash.as_str()) || events_in_file > 0 {
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
        if n == 0 { break; }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

async fn upsert_session(store: &Store, p: &ParsedLine, session_id: &str, file_sha: &str) -> anyhow::Result<()> {
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

async fn write_event(
    store: &Store,
    p: &ParsedLine,
    session_id: &str,
    source_path: &str,
    offset: u64,
    file_sha: &str,
) -> anyhow::Result<()> {
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

    sqlx::query(
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

    for h in &hits {
        sqlx::query("INSERT OR IGNORE INTO redactions(event_uuid, field_path, rule) VALUES (?1, ?2, ?3)")
            .bind(&uuid)
            .bind(&h.field_path)
            .bind(h.rule)
            .execute(&store.writer)
            .await?;
    }
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
    let Some(uuid) = p.transcript.uuid.as_deref() else { return Ok(()); };
    let Some(msg) = p.transcript.message.as_ref() else { return Ok(()); };
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
    let Some(uuid) = p.transcript.uuid.as_deref() else { return Ok(0); };
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

async fn write_tool_results(store: &Store, p: &ParsedLine) -> anyhow::Result<()> {
    for (tool_use_id, is_error, content) in tool_results(p) {
        let tc_id: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM tool_calls WHERE tool_use_id = ?1",
        )
        .bind(&tool_use_id)
        .fetch_optional(&store.reader)
        .await?;
        let Some(tc_id) = tc_id else { continue };

        let started_at: Option<i64> = sqlx::query_scalar(
            "SELECT ts FROM tool_calls WHERE id = ?1",
        )
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

async fn accumulate_session_tokens(store: &Store, p: &ParsedLine, session_id: &str) -> anyhow::Result<()> {
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
