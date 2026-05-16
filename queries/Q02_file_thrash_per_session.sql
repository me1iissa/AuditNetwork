-- Q02 — Per-session thrash: same file accessed > 5 times.
-- High thrash signals that something didn't stick the first time —
-- either Claude couldn't find what it needed, or the file kept being
-- edited and re-read across many turns. Either way, candidate for
-- caching / refactoring.
SELECT fo.file_path,
       tc.session_id,
       COUNT(*) AS hits
FROM file_ops fo
JOIN tool_calls tc ON tc.id = fo.tool_call_id
GROUP BY tc.session_id, fo.file_path
HAVING hits > 5
ORDER BY hits DESC
LIMIT 100;
