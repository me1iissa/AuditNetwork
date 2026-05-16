-- Q22 — Tool mix shift across Claude versions.
-- How did the same project's tool usage change as Claude Code shipped
-- new versions? Rising Agent% over time suggests delegation became
-- more common; falling Read% might mean grep/glob got smarter. Mostly
-- useful for long-running projects with many sessions across versions.
SELECT s.claude_version,
       tc.tool_name,
       COUNT(*)                                                                                                         AS calls,
       ROUND(100.0 * COUNT(*) / SUM(COUNT(*)) OVER (PARTITION BY s.claude_version), 1)                                  AS pct_within_version
FROM sessions s
JOIN tool_calls tc ON tc.session_id = s.id
WHERE s.claude_version IS NOT NULL
GROUP BY s.claude_version, tc.tool_name
ORDER BY s.claude_version, pct_within_version DESC;
