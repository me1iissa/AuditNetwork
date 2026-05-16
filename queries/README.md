# AuditNetwork — Canonical Queries

26 SQL queries that ship with AuditNetwork and form the spine of its
audit / optimisation product. They run against the SQLite warehouse
populated by `auditnetwork ingest …`.

## Running

From the repo:

```
auditnetwork query "$(cat queries/Q01_hot_files.sql)"
auditnetwork query "$(cat queries/Q07_cache_hit_ratio.sql)" 2>/dev/null
```

Or via the HTTP API:

```
curl -sS -X POST http://localhost:8080/api/query \
  -H "content-type: application/json" \
  -d '{"sql":"select count(*) from tool_calls"}'
```

Or open the DB directly with [datasette](https://datasette.io/):

```
datasette ~/.local/share/auditnetwork/audit.db
```

## Themes

| File | Theme |
|---|---|
| `Q01`–`Q08` | Cost / tokens / cache efficiency |
| `Q09`–`Q13` | Behavioural patterns |
| `Q14`–`Q19` | Failure / inefficiency signatures |
| `Q20`–`Q22` | Cross-session research |
| `Q23`–`Q26` | Per-project audit |

The `views/` subdirectory holds 11 `CREATE VIEW v_*` statements applied
by the second migration (`migrations/0002_views.sql`). The dashboard
sidebar maps 1:1 to those views.

## Parameters

`?N` placeholders are the SQLite convention. The HTTP `POST /api/query`
endpoint accepts an optional `"params": [...]` array bound positionally
to those placeholders. The CLI does not yet support parameters; bake
literal values into the SQL or use the HTTP endpoint.
