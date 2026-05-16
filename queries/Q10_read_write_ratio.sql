-- Q10 — Read:Write ratio per session.
-- Reads / (Writes + Edits + MultiEdits). A very high ratio (>20) is an
-- exploration-heavy session; <2 is a write-heavy / generation session.
-- Useful for spotting sessions where Claude over-read or under-explored.
SELECT session_id,
       SUM(CASE WHEN tool_name = 'Read' THEN 1 ELSE 0 END)                                                          AS reads,
       SUM(CASE WHEN tool_name IN ('Write', 'Edit', 'MultiEdit') THEN 1 ELSE 0 END)                                 AS writes,
       ROUND(1.0 * SUM(CASE WHEN tool_name = 'Read' THEN 1 ELSE 0 END)
             / NULLIF(SUM(CASE WHEN tool_name IN ('Write', 'Edit', 'MultiEdit') THEN 1 ELSE 0 END), 0), 2)          AS rw_ratio
FROM tool_calls
GROUP BY session_id
ORDER BY rw_ratio DESC NULLS LAST;
