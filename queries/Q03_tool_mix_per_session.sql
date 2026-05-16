-- Q03 — Tool mix per session (percent).
-- A session that's 80% Bash is likely automation; 80% Read is
-- exploration; 80% Edit is implementation. Useful for classifying and
-- comparing sessions.
SELECT session_id,
       tool_name,
       COUNT(*) AS calls,
       ROUND(100.0 * COUNT(*) / SUM(COUNT(*)) OVER (PARTITION BY session_id), 1) AS pct
FROM tool_calls
GROUP BY session_id, tool_name
ORDER BY session_id, pct DESC;
