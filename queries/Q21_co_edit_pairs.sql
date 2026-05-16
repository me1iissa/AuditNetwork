-- Q21 — Co-edit graph: pairs of files edited together in the same session.
-- Strong dependency signal. When file X is always edited alongside Y,
-- they're coupled; this is the raw input to dependency-discovery
-- analyses and refactor candidacy scoring.
SELECT a1.display AS file_a,
       a2.display AS file_b,
       COUNT(DISTINCT tc1.session_id) AS sessions_together
FROM file_ops fo1
JOIN tool_calls tc1 ON tc1.id = fo1.tool_call_id
JOIN file_ops fo2   ON fo2.tool_call_id IN (SELECT id FROM tool_calls WHERE session_id = tc1.session_id)
JOIN tool_calls tc2 ON tc2.id = fo2.tool_call_id
JOIN artifacts a1   ON a1.id = fo1.artifact_id
JOIN artifacts a2   ON a2.id = fo2.artifact_id
WHERE fo1.op IN ('edit', 'write')
  AND fo2.op IN ('edit', 'write')
  AND a1.id < a2.id                          -- de-dupe symmetric pairs
GROUP BY a1.id, a2.id
HAVING sessions_together >= 2
ORDER BY sessions_together DESC, file_a
LIMIT 50;
