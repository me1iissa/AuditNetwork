-- Q18 — Sessions affected by context compaction.
-- Compactions are forced by hitting the context window. A session with
-- N compactions probably has fragmented memory and is worth examining
-- as a "ran long enough to break" exemplar.
--
-- Limitation: the compactions table is wired but not yet populated by
-- the M1 ingest pipeline (compaction events are detectable from the
-- transcript but not yet projected). Returns zero rows until then.
SELECT session_id, COUNT(*) AS compactions
FROM compactions
GROUP BY session_id
ORDER BY compactions DESC;
