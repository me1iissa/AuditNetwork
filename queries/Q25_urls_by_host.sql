-- Q25 — URL fetches grouped by host.
-- A breakdown of which hosts Claude reaches out to and how reliably
-- they respond. Pair with Q05 (which URLs get hit repeatedly) and Q16
-- (per-host error rate) for the full picture.
SELECT host,
       COUNT(*)                                                  AS fetches,
       SUM(CASE WHEN status_code BETWEEN 200 AND 299 THEN 1 ELSE 0 END) AS ok,
       SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END)              AS errs,
       SUM(bytes)                                                AS total_bytes
FROM web_fetches
GROUP BY host
ORDER BY fetches DESC;
