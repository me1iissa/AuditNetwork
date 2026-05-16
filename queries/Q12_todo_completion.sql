-- Q12 — TODO completion rate per session.
-- Of the unique TODO items written via TodoWrite, what fraction landed
-- in a `completed` state by the last snapshot? Best read as a hint, not
-- a score — Claude often closes a TODO by moving the work to a new one.
--
-- Limitation: M1's ingest pipeline doesn't yet populate `todos`; rows
-- arrive when M5 starts handling TodoWrite tool inputs. Returns zero
-- rows until then.
SELECT session_id,
       COUNT(DISTINCT todo_hash)                                       AS todos_total,
       SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END)           AS todos_done,
       ROUND(1.0 * SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END)
             / NULLIF(COUNT(DISTINCT todo_hash), 0), 2)                AS completion_rate
FROM todos
WHERE superseded_at IS NULL
GROUP BY session_id
ORDER BY completion_rate DESC NULLS LAST;
