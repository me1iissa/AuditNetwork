-- Q07 — Anthropic prompt-cache hit ratio per session.
-- High cache_read_tokens / tokens_in means the prompt prefix was reused
-- (cheap); a low ratio means the prefix was re-built each turn (expensive).
-- This is the most defensible cost metric the warehouse exposes.
SELECT session_id,
       SUM(tokens_in)             AS tokens_in,
       SUM(cache_read_tokens)     AS cache_read,
       SUM(cache_creation_tokens) AS cache_create,
       ROUND(100.0 * SUM(cache_read_tokens) / NULLIF(SUM(tokens_in), 0), 1) AS cache_hit_pct
FROM messages
GROUP BY session_id
ORDER BY cache_hit_pct ASC NULLS FIRST;
