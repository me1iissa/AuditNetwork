-- Q15 — Edit-then-revert: a file's content_hash returns to a prior state.
-- A strong "wasted work" signal — Claude wrote something, then a couple
-- of edits later wrote it back. Requires content_hash_before/after on
-- file_ops, which the M1 ingest pipeline records when available.
WITH ord AS (
    SELECT fo.*,
           ROW_NUMBER() OVER (PARTITION BY fo.artifact_id ORDER BY fo.tool_call_id) AS rn
    FROM file_ops fo
    WHERE fo.op IN ('edit', 'write')
)
SELECT a.id    AS artifact_id,
       a.display,
       o1.tool_call_id AS first_edit,
       o2.tool_call_id AS reverting_edit
FROM ord o1
JOIN ord o2
  ON o1.artifact_id = o2.artifact_id
 AND o2.rn          = o1.rn + 2
 AND o1.content_hash_before IS NOT NULL
 AND o1.content_hash_before = o2.content_hash_after
JOIN artifacts a ON a.id = o1.artifact_id
ORDER BY a.id, o1.tool_call_id;
