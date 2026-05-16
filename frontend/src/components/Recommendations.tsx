import { useEffect, useState } from "react";
import { api, type Recommendation } from "../api";
import { useUi } from "../store";

function severityColor(s: string): string {
  switch (s) {
    case "warn":
      return "#e8a33d";
    case "opportunity":
      return "#58a6ff";
    default:
      return "#8b949e";
  }
}

/// Right-panel content when nothing is selected: a list of optimisation
/// hits the detectors produced for the current session.
export function Recommendations() {
  const session = useUi((s) => s.selectedSessionId);
  const [recs, setRecs] = useState<Recommendation[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setRecs(null);
    setError(null);
    if (!session) return;
    api
      .recommendations(session)
      .then(setRecs)
      .catch((e) => setError(String(e)));
  }, [session]);

  if (!session) {
    return (
      <div className="recs">
        <div className="muted">Select a session.</div>
      </div>
    );
  }
  if (error) return <div className="error">{error}</div>;
  if (recs == null) return <div className="muted">loading…</div>;
  if (recs.length === 0) {
    return (
      <div className="recs empty">
        <h3>No recommendations</h3>
        <p className="muted">
          Run <code>auditnetwork recommend {session.slice(0, 8)}</code> (or{" "}
          <code>recommend-all</code>) to score this session.
        </p>
      </div>
    );
  }
  return (
    <div className="recs">
      <h3>{recs.length} recommendations</h3>
      <ul>
        {recs.map((r) => (
          <li key={r.id}>
            <div className="rec-head">
              <span
                className="rec-rule"
                style={{ color: severityColor(r.severity) }}
              >
                {r.rule_id}
              </span>
              {r.estimated_save && (
                <span className="rec-save">{r.estimated_save}</span>
              )}
            </div>
            <div className="rec-summary">{r.summary}</div>
          </li>
        ))}
      </ul>
    </div>
  );
}
