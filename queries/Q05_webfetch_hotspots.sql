-- Q05 — WebFetch hotspots: same URL fetched repeatedly.
-- A URL that gets fetched 3+ times across sessions is a strong cache
-- candidate. Pair with Q16 (per-host error rate) to know if it's worth
-- caching or just being retried after a failure.
SELECT url_hash,
       MIN(url)                     AS url,
       COUNT(*)                     AS total_fetches,
       COUNT(DISTINCT tc.session_id) AS sessions
FROM web_fetches wf
JOIN tool_calls tc ON tc.id = wf.tool_call_id
GROUP BY url_hash
HAVING total_fetches > 1
ORDER BY total_fetches DESC
LIMIT 50;
