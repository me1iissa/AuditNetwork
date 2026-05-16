-- Q04 — Subagent ROI by agent type.
-- Subagents cost tokens. This query asks: do they produce proportionally
-- more output? Higher result_chars / token suggests the agent type is
-- efficient; lower suggests it's chewing through context for thin output.
--
-- Limitation: M1's ingest pipeline doesn't yet populate subagent_runs;
-- the table is wired but empty until M5 backfills it from the
-- subagent JSONL files. Until then this returns zero rows.
SELECT agent_type,
       COUNT(*)                                                AS runs,
       AVG(tokens_in + tokens_out)                             AS avg_tokens,
       AVG(result_chars)                                       AS avg_result_chars,
       AVG(1.0 * result_chars / NULLIF(tokens_in + tokens_out, 0)) AS chars_per_token
FROM subagent_runs
GROUP BY agent_type
ORDER BY chars_per_token DESC NULLS LAST;
