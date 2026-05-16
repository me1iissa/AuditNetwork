-- Q19 — Permission denials by tool.
-- Where the user has been declining auto-permission prompts. High counts
-- on a given tool usually mean the permission allowlist needs an entry
-- or the tool's invocation pattern is risky and being intercepted.
--
-- Limitation: permission_events is wired but not yet populated.
SELECT tc.tool_name,
       COUNT(*) AS denials
FROM permission_events pe
JOIN tool_calls tc ON tc.id = pe.tool_call_id
WHERE pe.decision = 'deny'
GROUP BY tc.tool_name
ORDER BY denials DESC;
