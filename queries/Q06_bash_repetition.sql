-- Q06 — Bash command repetition within a session.
-- Same command run ≥ 3 times in one session usually means a polling
-- loop, a flaky operation Claude is retrying, or a check Claude doesn't
-- realise it already did. The command itself is shown for the top hit
-- via MIN() — argv0 alone (q.v. v_bash_failures) is easier to scan.
SELECT tc.session_id,
       bc.command_hash,
       MIN(bc.command) AS cmd,
       COUNT(*)        AS n
FROM bash_commands bc
JOIN tool_calls tc ON tc.id = bc.tool_call_id
GROUP BY tc.session_id, bc.command_hash
HAVING n >= 3
ORDER BY n DESC
LIMIT 50;
