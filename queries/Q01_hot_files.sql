-- Q01 — Hottest files across all sessions.
-- Surfaces files Claude keeps returning to. Big numbers mean either real
-- centrality (a core module) or thrash (re-reading because the answer
-- never lands). Pair with Q02 to disambiguate.
SELECT a.display AS file,
       COUNT(*)                                                 AS touches,
       SUM(CASE WHEN tae.access_kind = 'read'  THEN 1 ELSE 0 END) AS reads,
       SUM(CASE WHEN tae.access_kind = 'edit'  THEN 1 ELSE 0 END) AS edits,
       SUM(CASE WHEN tae.access_kind = 'write' THEN 1 ELSE 0 END) AS writes
FROM tool_artifact_edges tae
JOIN tool_calls tc ON tc.id = tae.tool_call_id
JOIN artifacts  a  ON a.id  = tae.artifact_id
WHERE a.kind = 'file'
GROUP BY a.id
ORDER BY touches DESC
LIMIT 50;
