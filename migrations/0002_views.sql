-- AuditNetwork view layer.
-- 1:1 with the dashboard sidebar — each view powers a card. Heavy
-- joins (co_edit_pairs, attractors) will be materialised into _cache_*
-- tables when M5 introduces session-close events; until then they're
-- live views and may be slow on large warehouses.

DROP VIEW IF EXISTS v_session_summary;
CREATE VIEW v_session_summary AS
SELECT
    s.id,
    s.ai_title,
    s.started_at,
    s.ended_at,
    s.ended_at - s.started_at                 AS duration_ms,
    s.claude_version,
    s.model,
    (SELECT COUNT(*) FROM tool_calls tc WHERE tc.session_id = s.id) AS tool_calls,
    (SELECT COUNT(DISTINCT tae.artifact_id)
       FROM tool_artifact_edges tae
       JOIN tool_calls tc ON tc.id = tae.tool_call_id
      WHERE tc.session_id = s.id)              AS artifacts_touched,
    s.total_input_tokens + s.total_output_tokens AS total_tokens
FROM sessions s;

DROP VIEW IF EXISTS v_hot_files;
CREATE VIEW v_hot_files AS
SELECT a.id, a.display AS file,
       COUNT(*) AS touches,
       SUM(CASE WHEN tae.access_kind = 'read'  THEN 1 ELSE 0 END) AS reads,
       SUM(CASE WHEN tae.access_kind = 'edit'  THEN 1 ELSE 0 END) AS edits,
       SUM(CASE WHEN tae.access_kind = 'write' THEN 1 ELSE 0 END) AS writes
FROM tool_artifact_edges tae
JOIN tool_calls tc ON tc.id = tae.tool_call_id
JOIN artifacts  a  ON a.id  = tae.artifact_id
WHERE a.kind = 'file'
GROUP BY a.id;

DROP VIEW IF EXISTS v_file_thrash;
CREATE VIEW v_file_thrash AS
SELECT fo.file_path,
       tc.session_id,
       COUNT(*) AS hits
FROM file_ops fo
JOIN tool_calls tc ON tc.id = fo.tool_call_id
GROUP BY tc.session_id, fo.file_path
HAVING hits > 5;

DROP VIEW IF EXISTS v_bash_failures;
CREATE VIEW v_bash_failures AS
SELECT tc.session_id,
       bc.command_hash,
       MIN(bc.command) AS cmd,
       COUNT(*)        AS fails
FROM bash_commands bc
JOIN tool_calls tc ON tc.id = bc.tool_call_id
WHERE bc.exit_code IS NOT NULL AND bc.exit_code <> 0
GROUP BY tc.session_id, bc.command_hash
HAVING fails >= 2;

DROP VIEW IF EXISTS v_webfetch_dupes;
CREATE VIEW v_webfetch_dupes AS
SELECT url_hash,
       MIN(url) AS url,
       COUNT(*) AS fetches,
       COUNT(DISTINCT tc.session_id) AS sessions
FROM web_fetches wf
JOIN tool_calls tc ON tc.id = wf.tool_call_id
GROUP BY url_hash
HAVING fetches > 1;

DROP VIEW IF EXISTS v_subagent_costs;
CREATE VIEW v_subagent_costs AS
SELECT agent_type,
       COUNT(*) AS runs,
       AVG(tokens_in + tokens_out)                                 AS avg_tokens,
       AVG(result_chars)                                           AS avg_result_chars,
       AVG(1.0 * result_chars / NULLIF(tokens_in + tokens_out, 0)) AS chars_per_token
FROM subagent_runs
GROUP BY agent_type;

DROP VIEW IF EXISTS v_attractors;
CREATE VIEW v_attractors AS
SELECT a.id, a.display,
       COUNT(DISTINCT tc.session_id) AS sessions
FROM artifacts a
JOIN tool_artifact_edges tae ON tae.artifact_id = a.id
JOIN tool_calls tc           ON tc.id           = tae.tool_call_id
WHERE a.kind = 'file'
GROUP BY a.id
HAVING sessions >= 2;

DROP VIEW IF EXISTS v_cache_efficiency;
CREATE VIEW v_cache_efficiency AS
SELECT session_id,
       SUM(tokens_in)             AS tokens_in,
       SUM(cache_read_tokens)     AS cache_read,
       SUM(cache_creation_tokens) AS cache_create,
       ROUND(100.0 * SUM(cache_read_tokens) / NULLIF(SUM(tokens_in), 0), 1) AS cache_hit_pct
FROM messages
GROUP BY session_id;

DROP VIEW IF EXISTS v_permission_denials;
CREATE VIEW v_permission_denials AS
SELECT tc.tool_name,
       COUNT(*) AS denials
FROM permission_events pe
JOIN tool_calls tc ON tc.id = pe.tool_call_id
WHERE pe.decision = 'deny'
GROUP BY tc.tool_name;

DROP VIEW IF EXISTS v_orphan_calls;
CREATE VIEW v_orphan_calls AS
SELECT tc.id, tc.session_id, tc.tool_name, tc.ts, tc.event_uuid
FROM tool_calls tc
LEFT JOIN events child ON child.parent_uuid = tc.event_uuid
WHERE child.uuid IS NULL;

DROP VIEW IF EXISTS v_secret_path_touches;
CREATE VIEW v_secret_path_touches AS
SELECT DISTINCT tc.session_id,
                a.display,
                tae.access_kind
FROM tool_artifact_edges tae
JOIN artifacts a   ON a.id  = tae.artifact_id
JOIN tool_calls tc ON tc.id = tae.tool_call_id
WHERE a.kind = 'file'
  AND (a.canonical_key LIKE '%.env%'
    OR a.canonical_key LIKE '%/secrets/%'
    OR a.canonical_key LIKE '%credentials%'
    OR a.canonical_key LIKE '%/.aws/%'
    OR a.canonical_key LIKE '%/.ssh/%'
    OR a.canonical_key LIKE '%.pem'
    OR a.canonical_key LIKE '%.key');
