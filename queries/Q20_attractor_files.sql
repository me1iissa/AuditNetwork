-- Q20 — "Claude attractor" files touched in ≥ N distinct sessions.
-- Files that show up across many sessions: often config (.gitignore,
-- Cargo.toml), top-level docs (README.md), or the canonical entry point
-- of the project. Useful for ranking what "matters" structurally.
SELECT a.display,
       COUNT(DISTINCT tc.session_id) AS sessions
FROM artifacts a
JOIN tool_artifact_edges tae ON tae.artifact_id = a.id
JOIN tool_calls tc           ON tc.id           = tae.tool_call_id
WHERE a.kind = 'file'
GROUP BY a.id
HAVING sessions >= 2
ORDER BY sessions DESC, a.display
LIMIT 50;
