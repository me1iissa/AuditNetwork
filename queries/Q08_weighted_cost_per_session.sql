-- Q08 — Weighted token cost per session.
-- Anthropic's prompt-cache pricing (as of writing): cache_creation costs
-- 1.25× the base input rate, cache_read costs 0.1× the base, output
-- costs 5×. The "cost units" column here is a rough comparator — useful
-- for ranking sessions by spend without committing to absolute dollars.
-- Adjust the multipliers if pricing changes.
SELECT s.id,
       s.ai_title,
       SUM(m.tokens_in)             AS tokens_in,
       SUM(m.tokens_out)            AS tokens_out,
       SUM(m.cache_read_tokens)     AS cache_read,
       SUM(m.cache_creation_tokens) AS cache_create,
       ROUND(
           SUM(m.tokens_in) +
           SUM(m.cache_creation_tokens) * 1.25 +
           SUM(m.cache_read_tokens)     * 0.10 +
           SUM(m.tokens_out)            * 5.00,
           1
       ) AS cost_units
FROM sessions s
JOIN messages m ON m.session_id = s.id
GROUP BY s.id
ORDER BY cost_units DESC;
