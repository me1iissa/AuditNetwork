-- Q17 — Orphan tool calls: no child event within the message tree.
-- Reads as "Claude ran this tool but never observed/followed up on the
-- result within the session". Often a sign of a crash, timeout, or
-- compaction that dropped the tail.
SELECT tc.id, tc.session_id, tc.tool_name, tc.ts, tc.event_uuid
FROM tool_calls tc
LEFT JOIN events child ON child.parent_uuid = tc.event_uuid
WHERE child.uuid IS NULL
ORDER BY tc.ts DESC
LIMIT 50;
