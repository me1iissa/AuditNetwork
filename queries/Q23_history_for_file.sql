-- Q23 — Everything Claude did to a specific file.
-- Audit primitive: bind ?1 to a `canonical_key` (usually the absolute
-- path) and ?2 to a lower-bound ms-since-epoch (use 0 for all-time).
-- Returns every read/edit/write/grep ordered by time.
SELECT tc.ts,
       tc.session_id,
       tc.tool_name,
       fo.op,
       fo.lines_added,
       fo.lines_removed
FROM file_ops fo
JOIN tool_calls tc ON tc.id = fo.tool_call_id
JOIN artifacts a   ON a.id  = fo.artifact_id
WHERE a.canonical_key = ?1
  AND tc.ts >= ?2
ORDER BY tc.ts;
