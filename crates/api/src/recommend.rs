//! Optimisation detectors. Each rule produces a row in `recommendations`
//! tying its `rule_id` to a session, a short summary, JSON evidence
//! (specific tool_call ids), and an estimated saving.
//!
//! v1 rule set (in priority order — most actionable first):
//!
//! 1. `reread_unchanged_file` — a Read of a file whose content hasn't
//!    changed since the prior touch. Requires `file_ops.content_hash_*`,
//!    which the M1 ingest populates only when explicit bytes are
//!    available; until M5 backfills more aggressively this rule is
//!    conservative.
//!
//! 2. `redundant_bash` — same `command_hash` executed ≥ 2× within a
//!    session with no intervening file write.
//!
//! 3. `redundant_webfetch` — same `url_hash` fetched ≥ 2× within a
//!    session.
//!
//! 4. `pattern_repetition` — same `Grep` / `Glob` `pattern` fires ≥ 3×
//!    in a session — usually a sign Claude is hunting and should have
//!    cached the result.
//!
//! 5. `cross_session_webfetch_dupe` — same `url_hash` seen in ≥ 2
//!    distinct sessions within the last 7 days. Project-level cache
//!    candidate.

use store::Store;

#[derive(Debug, Clone)]
pub struct Recommendation {
    pub session_id: String,
    pub rule_id: &'static str,
    pub severity: &'static str, // 'info' | 'warn' | 'opportunity'
    pub summary: String,
    pub evidence_json: serde_json::Value,
    pub estimated_save: Option<String>,
}

/// Compute every rule against the warehouse for a single session and
/// upsert into the `recommendations` table. Returns the rows it wrote.
///
/// Idempotency: replaces every non-dismissed row for this session in a
/// single transaction so re-running doesn't double-count and a panicking
/// detector never leaves the session in an empty state. Dismissals
/// (`dismissed_at IS NOT NULL`) are preserved across runs.
pub async fn analyse_session(
    store: &Store,
    session_id: &str,
) -> anyhow::Result<Vec<Recommendation>> {
    let now_ms = time::OffsetDateTime::now_utc().unix_timestamp() * 1000;
    let mut out = Vec::new();
    out.extend(reread_unchanged_file(store, session_id).await?);
    out.extend(redundant_bash(store, session_id).await?);
    out.extend(redundant_webfetch(store, session_id).await?);
    out.extend(pattern_repetition(store, session_id).await?);
    out.extend(cross_session_webfetch_dupe(store, session_id, now_ms).await?);
    persist(store, session_id, &out, now_ms).await?;
    Ok(out)
}

async fn persist(
    store: &Store,
    session_id: &str,
    recs: &[Recommendation],
    now_ms: i64,
) -> anyhow::Result<()> {
    let mut tx = store.writer.begin().await?;
    sqlx::query("DELETE FROM recommendations WHERE session_id = ?1 AND dismissed_at IS NULL")
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
    for r in recs {
        sqlx::query(
            "INSERT INTO recommendations(session_id, rule_id, severity, summary, evidence_json, estimated_save, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(&r.session_id)
        .bind(r.rule_id)
        .bind(r.severity)
        .bind(&r.summary)
        .bind(serde_json::to_string(&r.evidence_json)?)
        .bind(r.estimated_save.as_deref())
        .bind(now_ms)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Rule 1 — a Read of a file that hasn't changed since the prior touch.
///
/// Two shapes are caught:
/// - The current Read's `content_hash_after` equals the prior op's
///   `content_hash_after` (works when Reads populate `content_hash_after`).
/// - The current Read's `content_hash_before` equals the prior op's
///   `content_hash_after` (the more common shape — works when Reads
///   only record the pre-read state).
///
/// Either signal means the bytes Claude just read are bytes we already
/// have in this session; the Read tool call is redundant.
async fn reread_unchanged_file(
    store: &Store,
    session_id: &str,
) -> anyhow::Result<Vec<Recommendation>> {
    let rows: Vec<(i64, String, Option<String>)> = sqlx::query_as(
        r#"
        WITH ord AS (
            SELECT fo.tool_call_id, fo.artifact_id, fo.op,
                   fo.content_hash_before,
                   fo.content_hash_after,
                   LAG(fo.content_hash_after) OVER (PARTITION BY fo.artifact_id ORDER BY tc.ts) AS prev_after,
                   fo.file_path
            FROM file_ops fo
            JOIN tool_calls tc ON tc.id = fo.tool_call_id
            WHERE tc.session_id = ?1
        )
        SELECT tool_call_id, file_path, prev_after
        FROM ord
        WHERE op = 'read'
          AND prev_after IS NOT NULL
          AND (
               (content_hash_after  IS NOT NULL AND content_hash_after  = prev_after)
            OR (content_hash_before IS NOT NULL AND content_hash_before = prev_after)
          )
        "#,
    )
    .bind(session_id)
    .fetch_all(&store.reader)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(tc_id, path, _prev)| Recommendation {
            session_id: session_id.to_string(),
            rule_id: "reread_unchanged_file",
            severity: "opportunity",
            summary: format!(
                "Re-read of {path} where the content hadn't changed since the last touch"
            ),
            evidence_json: serde_json::json!({ "tool_call_id": tc_id, "file_path": path }),
            estimated_save: Some("1 Read tool call per occurrence".into()),
        })
        .collect())
}

/// Rule 2 — same Bash command_hash executed twice in one session.
async fn redundant_bash(store: &Store, session_id: &str) -> anyhow::Result<Vec<Recommendation>> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        r#"
        SELECT bc.command_hash, MIN(bc.command), COUNT(*)
        FROM bash_commands bc
        JOIN tool_calls tc ON tc.id = bc.tool_call_id
        WHERE tc.session_id = ?1
        GROUP BY bc.command_hash
        HAVING COUNT(*) >= 2
        "#,
    )
    .bind(session_id)
    .fetch_all(&store.reader)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(hash, cmd, n)| Recommendation {
            session_id: session_id.to_string(),
            rule_id: "redundant_bash",
            severity: "opportunity",
            summary: format!("Bash `{}` ran {n}×", trim(&cmd, 60)),
            evidence_json: serde_json::json!({ "command_hash": hash, "count": n, "command": cmd }),
            estimated_save: Some(format!("{} Bash tool calls", n - 1)),
        })
        .collect())
}

/// Rule 3 — same URL fetched twice within a session.
async fn redundant_webfetch(
    store: &Store,
    session_id: &str,
) -> anyhow::Result<Vec<Recommendation>> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        r#"
        SELECT wf.url_hash, MIN(wf.url), COUNT(*)
        FROM web_fetches wf
        JOIN tool_calls tc ON tc.id = wf.tool_call_id
        WHERE tc.session_id = ?1
        GROUP BY wf.url_hash
        HAVING COUNT(*) >= 2
        "#,
    )
    .bind(session_id)
    .fetch_all(&store.reader)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(hash, url, n)| Recommendation {
            session_id: session_id.to_string(),
            rule_id: "redundant_webfetch",
            severity: "opportunity",
            summary: format!("WebFetch of {} ran {n}×", trim(&url, 70)),
            evidence_json: serde_json::json!({ "url_hash": hash, "count": n, "url": url }),
            estimated_save: Some(format!("{} WebFetch tool calls", n - 1)),
        })
        .collect())
}

/// Rule 4 — same Grep/Glob pattern fires ≥ 3× in a session.
async fn pattern_repetition(
    store: &Store,
    session_id: &str,
) -> anyhow::Result<Vec<Recommendation>> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        r#"
        SELECT tc.tool_name,
               json_extract(tc.input_json, '$.pattern') AS pattern,
               COUNT(*) AS n
        FROM tool_calls tc
        WHERE tc.session_id = ?1
          AND tc.tool_name IN ('Grep', 'Glob')
          AND pattern IS NOT NULL
        GROUP BY tc.tool_name, pattern
        HAVING n >= 3
        "#,
    )
    .bind(session_id)
    .fetch_all(&store.reader)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(tool, pat, n)| Recommendation {
            session_id: session_id.to_string(),
            rule_id: "pattern_repetition",
            severity: "opportunity",
            summary: format!("{tool} pattern `{}` ran {n}×", trim(&pat, 60)),
            evidence_json: serde_json::json!({ "tool": tool, "pattern": pat, "count": n }),
            estimated_save: Some(format!("{} search calls; cache the result", n - 1)),
        })
        .collect())
}

/// Rule 5 — same URL fetched across ≥ 2 sessions within 7 days of now.
///
/// The window is anchored on `now_ms` (the detector run time), not on
/// the analysed session's max ts — otherwise ingesting old sessions
/// would silently miss recent project-wide duplicates of URLs they hit.
async fn cross_session_webfetch_dupe(
    store: &Store,
    session_id: &str,
    now_ms: i64,
) -> anyhow::Result<Vec<Recommendation>> {
    let seven_days_ms: i64 = 7 * 24 * 3600 * 1000;
    let window_start = now_ms - seven_days_ms;
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        r#"
        SELECT wf.url_hash,
               MIN(wf.url) AS url,
               COUNT(DISTINCT tc.session_id) AS sessions
        FROM web_fetches wf
        JOIN tool_calls tc ON tc.id = wf.tool_call_id
        WHERE tc.ts >= ?2
          AND wf.url_hash IN (
            SELECT wf2.url_hash
            FROM web_fetches wf2
            JOIN tool_calls tc2 ON tc2.id = wf2.tool_call_id
            WHERE tc2.session_id = ?1
          )
        GROUP BY wf.url_hash
        HAVING sessions >= 2
        "#,
    )
    .bind(session_id)
    .bind(window_start)
    .fetch_all(&store.reader)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(hash, url, sessions)| Recommendation {
            session_id: session_id.to_string(),
            rule_id: "cross_session_webfetch_dupe",
            severity: "info",
            summary: format!(
                "URL {} hit by {sessions} sessions in the last 7 days; cache candidate",
                trim(&url, 70)
            ),
            evidence_json: serde_json::json!({ "url_hash": hash, "sessions": sessions, "url": url }),
            estimated_save: None,
        })
        .collect())
}

fn trim(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    }
}
