import { useEffect, useState } from "react";
import { api, type SessionSummary } from "../api";
import { useUi } from "../store";

function fmtTimestamp(ms: number): string {
  return new Date(ms).toISOString().replace("T", " ").slice(0, 19);
}

function fmtDuration(start: number, end: number | null): string {
  if (end == null) return "—";
  const secs = Math.round((end - start) / 1000);
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  const rs = secs % 60;
  return `${mins}m${rs.toString().padStart(2, "0")}s`;
}

export function Sidebar() {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const selected = useUi((s) => s.selectedSessionId);
  const setSession = useUi((s) => s.setSession);

  // Fetch sessions once on mount. Auto-selection is a separate effect so
  // clicking a session doesn't trigger a refetch.
  useEffect(() => {
    api
      .listSessions()
      .then(setSessions)
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    if (sessions.length && !selected) setSession(sessions[0].id);
  }, [sessions, selected, setSession]);

  return (
    <aside className="sidebar">
      <h1>AuditNetwork</h1>
      {error && <div className="error">{error}</div>}
      <ul className="session-list">
        {sessions.map((s) => (
          <li
            key={s.id}
            className={s.id === selected ? "selected" : ""}
            onClick={() => setSession(s.id)}
          >
            <div className="title">{s.ai_title ?? s.id.slice(0, 8)}</div>
            <div className="meta">
              <span>{fmtTimestamp(s.started_at)}</span>
              <span>·</span>
              <span>{fmtDuration(s.started_at, s.ended_at)}</span>
            </div>
            <div className="meta">
              <span>{s.tool_calls} calls</span>
              <span>·</span>
              <span>{s.artifacts_touched} artifacts</span>
              <span>·</span>
              <span>
                {(
                  (s.total_input_tokens + s.total_output_tokens) /
                  1000
                ).toFixed(1)}
                k tok
              </span>
            </div>
          </li>
        ))}
        {!sessions.length && !error && <li className="meta">No sessions ingested.</li>}
      </ul>
    </aside>
  );
}
