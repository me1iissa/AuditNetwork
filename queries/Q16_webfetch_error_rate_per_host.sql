-- Q16 — WebFetch error rate per host.
-- Hosts with high 4xx/5xx are unreliable for Claude — either they
-- rate-limit, gate on auth, or serve JS-rendered pages with no useful
-- body. Pair with Q05 to find hosts that are both popular and broken.
SELECT host,
       COUNT(*)                                                            AS fetches,
       SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END)                 AS errs,
       ROUND(100.0 * SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END)
             / NULLIF(COUNT(*), 0), 1)                                     AS err_pct
FROM web_fetches
GROUP BY host
HAVING fetches >= 3
ORDER BY err_pct DESC, fetches DESC;
