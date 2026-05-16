-- AuditNetwork SQLite schema, v1.
-- Two-tier rule: events is the lossless firehose (raw_json preserved).
-- Everything below it is a denormalised projection so analytical queries
-- never json_extract on the hot path.

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS projects (
    id            INTEGER PRIMARY KEY,
    slug          TEXT UNIQUE NOT NULL,
    cwd           TEXT NOT NULL,
    first_seen_ts INTEGER NOT NULL,
    last_seen_ts  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id                    TEXT PRIMARY KEY,
    project_id            INTEGER REFERENCES projects(id),
    started_at            INTEGER NOT NULL,
    ended_at              INTEGER,
    claude_version        TEXT,
    model                 TEXT,
    cwd                   TEXT,
    git_branch            TEXT,
    entrypoint            TEXT,
    ai_title              TEXT,
    exit_reason           TEXT,
    total_input_tokens    INTEGER NOT NULL DEFAULT 0,
    total_output_tokens   INTEGER NOT NULL DEFAULT 0,
    total_cache_read      INTEGER NOT NULL DEFAULT 0,
    total_cache_creation  INTEGER NOT NULL DEFAULT 0,
    source_file_sha256    TEXT
);
CREATE INDEX IF NOT EXISTS idx_sessions_project_started ON sessions(project_id, started_at);

CREATE TABLE IF NOT EXISTS events (
    uuid          TEXT PRIMARY KEY,
    parent_uuid   TEXT,
    session_id    TEXT NOT NULL REFERENCES sessions(id),
    ts            INTEGER NOT NULL,
    kind          TEXT NOT NULL,
    is_sidechain  INTEGER NOT NULL DEFAULT 0,
    agent_id      TEXT,
    prompt_id     TEXT,
    request_id    TEXT,
    raw_json      TEXT NOT NULL,
    source_path   TEXT NOT NULL,
    source_offset INTEGER NOT NULL,
    source_sha256 TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_session_ts ON events(session_id, ts);
CREATE INDEX IF NOT EXISTS idx_events_parent     ON events(parent_uuid);
CREATE INDEX IF NOT EXISTS idx_events_kind_ts    ON events(kind, ts);

CREATE TABLE IF NOT EXISTS messages (
    id                    INTEGER PRIMARY KEY,
    event_uuid            TEXT UNIQUE NOT NULL REFERENCES events(uuid),
    session_id            TEXT NOT NULL,
    ts                    INTEGER NOT NULL,
    role                  TEXT NOT NULL,
    model                 TEXT,
    content_hash          TEXT NOT NULL,
    text_chars            INTEGER NOT NULL DEFAULT 0,
    thinking_chars        INTEGER NOT NULL DEFAULT 0,
    tokens_in             INTEGER,
    tokens_out            INTEGER,
    cache_read_tokens     INTEGER,
    cache_creation_tokens INTEGER,
    stop_reason           TEXT
);
CREATE INDEX IF NOT EXISTS idx_messages_session_ts ON messages(session_id, ts);

CREATE TABLE IF NOT EXISTS tool_calls (
    id            INTEGER PRIMARY KEY,
    event_uuid    TEXT NOT NULL REFERENCES events(uuid),
    session_id    TEXT NOT NULL,
    ts            INTEGER NOT NULL,
    tool_use_id   TEXT NOT NULL,
    tool_name     TEXT NOT NULL,
    input_json    TEXT NOT NULL,
    duration_ms   INTEGER,
    success       INTEGER,
    error_kind    TEXT,
    is_sidechain  INTEGER NOT NULL DEFAULT 0,
    agent_id      TEXT,
    UNIQUE(tool_use_id, session_id)
);
CREATE INDEX IF NOT EXISTS idx_tc_session_ts   ON tool_calls(session_id, ts);
CREATE INDEX IF NOT EXISTS idx_tc_tool_ts      ON tool_calls(tool_name, ts);
CREATE INDEX IF NOT EXISTS idx_tc_session_tool ON tool_calls(session_id, tool_name);

CREATE TABLE IF NOT EXISTS tool_results (
    tool_call_id INTEGER PRIMARY KEY REFERENCES tool_calls(id),
    ts           INTEGER NOT NULL,
    output_bytes INTEGER NOT NULL DEFAULT 0,
    output_lines INTEGER,
    truncated    INTEGER NOT NULL DEFAULT 0,
    is_error     INTEGER NOT NULL DEFAULT 0,
    error_text   TEXT
);

CREATE TABLE IF NOT EXISTS artifacts (
    id             INTEGER PRIMARY KEY,
    kind           TEXT NOT NULL,
    canonical_key  TEXT NOT NULL,
    display        TEXT NOT NULL,
    project_id     INTEGER REFERENCES projects(id),
    first_seen_ts  INTEGER NOT NULL,
    last_seen_ts   INTEGER NOT NULL,
    UNIQUE(kind, canonical_key, project_id)
);
CREATE INDEX IF NOT EXISTS idx_artifacts_kind_key ON artifacts(kind, canonical_key);

CREATE TABLE IF NOT EXISTS tool_artifact_edges (
    tool_call_id  INTEGER NOT NULL REFERENCES tool_calls(id),
    artifact_id   INTEGER NOT NULL REFERENCES artifacts(id),
    access_kind   TEXT NOT NULL,
    bytes         INTEGER,
    lines_changed INTEGER,
    PRIMARY KEY (tool_call_id, artifact_id, access_kind)
);
CREATE INDEX IF NOT EXISTS idx_tae_artifact ON tool_artifact_edges(artifact_id);

CREATE TABLE IF NOT EXISTS file_ops (
    tool_call_id        INTEGER PRIMARY KEY REFERENCES tool_calls(id),
    artifact_id         INTEGER NOT NULL REFERENCES artifacts(id),
    file_path           TEXT NOT NULL,
    op                  TEXT NOT NULL,
    offset_start        INTEGER,
    offset_end          INTEGER,
    bytes_before        INTEGER,
    bytes_after         INTEGER,
    lines_added         INTEGER,
    lines_removed       INTEGER,
    content_hash_before TEXT,
    content_hash_after  TEXT
);
CREATE INDEX IF NOT EXISTS idx_file_ops_artifact ON file_ops(artifact_id);

CREATE TABLE IF NOT EXISTS bash_commands (
    tool_call_id INTEGER PRIMARY KEY REFERENCES tool_calls(id),
    command      TEXT NOT NULL,
    argv0        TEXT NOT NULL,
    cwd          TEXT,
    exit_code    INTEGER,
    stdout_bytes INTEGER,
    stderr_bytes INTEGER,
    duration_ms  INTEGER,
    command_hash TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_bash_argv0 ON bash_commands(argv0);
CREATE INDEX IF NOT EXISTS idx_bash_hash  ON bash_commands(command_hash);

CREATE TABLE IF NOT EXISTS web_fetches (
    tool_call_id INTEGER PRIMARY KEY REFERENCES tool_calls(id),
    url          TEXT NOT NULL,
    host         TEXT NOT NULL,
    path         TEXT NOT NULL,
    method       TEXT NOT NULL DEFAULT 'GET',
    status_code  INTEGER,
    content_type TEXT,
    bytes        INTEGER,
    url_hash     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_wf_host ON web_fetches(host);
CREATE INDEX IF NOT EXISTS idx_wf_hash ON web_fetches(url_hash);

CREATE TABLE IF NOT EXISTS subagent_runs (
    id                   INTEGER PRIMARY KEY,
    parent_tool_call_id  INTEGER REFERENCES tool_calls(id),
    session_id           TEXT NOT NULL,
    agent_id             TEXT NOT NULL,
    agent_type           TEXT,
    prompt_hash          TEXT,
    prompt_chars         INTEGER NOT NULL DEFAULT 0,
    started_at           INTEGER NOT NULL,
    ended_at             INTEGER,
    duration_ms          INTEGER,
    tool_calls_count     INTEGER NOT NULL DEFAULT 0,
    tokens_in            INTEGER NOT NULL DEFAULT 0,
    tokens_out           INTEGER NOT NULL DEFAULT 0,
    result_chars         INTEGER,
    result_hash          TEXT,
    source_file_sha256   TEXT,
    UNIQUE(session_id, agent_id)
);
CREATE INDEX IF NOT EXISTS idx_sub_parent ON subagent_runs(parent_tool_call_id);

CREATE TABLE IF NOT EXISTS todos (
    id            INTEGER PRIMARY KEY,
    session_id    TEXT NOT NULL,
    ts            INTEGER NOT NULL,
    todo_hash     TEXT NOT NULL,
    content       TEXT NOT NULL,
    status        TEXT NOT NULL,
    superseded_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_todos_session ON todos(session_id, ts);

CREATE TABLE IF NOT EXISTS compactions (
    id            INTEGER PRIMARY KEY,
    session_id    TEXT NOT NULL,
    ts            INTEGER NOT NULL,
    tokens_before INTEGER,
    tokens_after  INTEGER
);

CREATE TABLE IF NOT EXISTS permission_events (
    id           INTEGER PRIMARY KEY,
    tool_call_id INTEGER REFERENCES tool_calls(id),
    ts           INTEGER NOT NULL,
    decision     TEXT NOT NULL,
    reason       TEXT
);

CREATE TABLE IF NOT EXISTS redactions (
    event_uuid TEXT NOT NULL,
    field_path TEXT NOT NULL,
    rule       TEXT NOT NULL,
    PRIMARY KEY(event_uuid, field_path)
);

CREATE TABLE IF NOT EXISTS ingest_log (
    source_path    TEXT PRIMARY KEY,
    source_sha256  TEXT NOT NULL,
    ingested_at    INTEGER NOT NULL,
    bytes_ingested INTEGER NOT NULL,
    events_added   INTEGER NOT NULL,
    schema_version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS saved_queries (
    id         INTEGER PRIMARY KEY,
    name       TEXT UNIQUE NOT NULL,
    sql        TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS recommendations (
    id             INTEGER PRIMARY KEY,
    session_id     TEXT NOT NULL,
    rule_id        TEXT NOT NULL,
    severity       TEXT NOT NULL,
    summary        TEXT NOT NULL,
    evidence_json  TEXT NOT NULL,
    estimated_save TEXT,
    created_at     INTEGER NOT NULL,
    dismissed_at   INTEGER
);
CREATE INDEX IF NOT EXISTS idx_recs_session ON recommendations(session_id);
