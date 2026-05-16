-- Q13 — Plan → implementation latency.
-- First time the session opens a plan-mode artefact (ExitPlanMode tool
-- call) vs. the first subsequent Edit/Write. A long delta suggests
-- planning happened but execution stalled; a near-zero delta suggests
-- planning was perfunctory.
WITH first_plan AS (
    SELECT session_id, MIN(ts) AS plan_ts
    FROM tool_calls
    WHERE tool_name = 'ExitPlanMode'
    GROUP BY session_id
),
first_edit_after AS (
    SELECT fp.session_id, MIN(tc.ts) AS edit_ts
    FROM first_plan fp
    JOIN tool_calls tc
      ON tc.session_id = fp.session_id
     AND tc.tool_name IN ('Edit', 'Write', 'MultiEdit')
     AND tc.ts >= fp.plan_ts
    GROUP BY fp.session_id
)
SELECT fp.session_id,
       fp.plan_ts,
       fea.edit_ts,
       fea.edit_ts - fp.plan_ts AS ms_plan_to_impl
FROM first_plan fp
LEFT JOIN first_edit_after fea ON fea.session_id = fp.session_id
ORDER BY ms_plan_to_impl ASC NULLS LAST;
