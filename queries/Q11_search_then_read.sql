-- Q11 — Grep/Glob immediately followed by Read.
-- The healthy pattern: locate, then read. A session with a high count
-- here probably has a clean exploration loop. A session with zero is
-- usually one that cold-reads paths it already knows.
WITH seq AS (
    SELECT session_id,
           tool_name,
           LEAD(tool_name) OVER (PARTITION BY session_id ORDER BY ts) AS next_tool
    FROM tool_calls
)
SELECT session_id, COUNT(*) AS search_then_read
FROM seq
WHERE tool_name IN ('Grep', 'Glob')
  AND next_tool = 'Read'
GROUP BY session_id
ORDER BY search_then_read DESC;
