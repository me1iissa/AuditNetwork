-- Q24 — Every bash command run with cwd = ?1.
-- Useful for auditing actions taken inside a specific project directory.
-- Bind ?1 to the cwd, e.g. '/home/user/AuditNetwork'.
SELECT tc.ts,
       tc.session_id,
       bc.command,
       bc.exit_code
FROM bash_commands bc
JOIN tool_calls tc ON tc.id = bc.tool_call_id
WHERE bc.cwd = ?1
ORDER BY tc.ts DESC
LIMIT 500;
