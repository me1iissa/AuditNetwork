-- Q14 — Repeated failed bash commands (same hash, exit != 0, ≥ 2x).
-- Strong "stuck retry" signal. The same command failing twice in a row
-- with no intervening fix means Claude is hoping for a different result;
-- worth surfacing.
SELECT tc.session_id,
       bc.command_hash,
       MIN(bc.command) AS cmd,
       COUNT(*)        AS fails
FROM bash_commands bc
JOIN tool_calls tc ON tc.id = bc.tool_call_id
WHERE bc.exit_code IS NOT NULL AND bc.exit_code <> 0
GROUP BY tc.session_id, bc.command_hash
HAVING fails >= 2
ORDER BY fails DESC;
