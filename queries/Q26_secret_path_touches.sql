-- Q26 — Sessions that touched secret-like paths.
-- Audit primitive — surfaces every time Claude read/wrote a file whose
-- path matches a secret-y pattern. Not authoritative ("paths that
-- contain secrets" is not the same as "secrets") but useful for a quick
-- "did we look at the .env" sweep. The redaction pipeline catches the
-- content; this catches the location.
SELECT DISTINCT tc.session_id,
                a.display,
                tae.access_kind
FROM tool_artifact_edges tae
JOIN artifacts a   ON a.id  = tae.artifact_id
JOIN tool_calls tc ON tc.id = tae.tool_call_id
WHERE a.kind = 'file'
  AND (a.canonical_key LIKE '%.env%'
    OR a.canonical_key LIKE '%/secrets/%'
    OR a.canonical_key LIKE '%credentials%'
    OR a.canonical_key LIKE '%/.aws/%'
    OR a.canonical_key LIKE '%/.ssh/%'
    OR a.canonical_key LIKE '%.pem'
    OR a.canonical_key LIKE '%.key')
ORDER BY tc.session_id, a.display;
