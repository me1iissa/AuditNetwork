-- Q09 — Time-to-first-edit per session.
-- How long Claude spent exploring before changing code. A high TTFE on
-- a fix task may indicate the model was hunting for the bug; a low TTFE
-- on a refactor task may mean it dove in without enough context. Either
-- reading needs the task description to interpret — use this in pairs.
SELECT s.id,
       s.ai_title,
       s.started_at,
       MIN(CASE WHEN tc.tool_name IN ('Edit', 'Write', 'MultiEdit') THEN tc.ts END)
           - s.started_at AS ms_to_first_edit
FROM sessions s
JOIN tool_calls tc ON tc.session_id = s.id
GROUP BY s.id
ORDER BY ms_to_first_edit ASC NULLS LAST;
