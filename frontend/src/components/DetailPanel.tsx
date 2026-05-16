import { useEffect, useState } from "react";
import { api, type ArtifactDetail, type ToolCallDetail } from "../api";
import { useUi } from "../store";

type Detail =
  | { kind: "tool_call"; data: ToolCallDetail }
  | { kind: "artifact"; data: ArtifactDetail }
  | { kind: "event"; uuid: string }
  | null;

function fmtMs(ms: number | null): string {
  if (ms == null) return "—";
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toFixed(2)} s`;
}

export function DetailPanel() {
  const selected = useUi((s) => s.selectedNode);
  const session = useUi((s) => s.selectedSessionId);
  const [detail, setDetail] = useState<Detail>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDetail(null);
    setError(null);
    if (!selected) return;
    if (selected.kind === "tool_call") {
      api
        .toolCall(selected.id)
        .then((d) => setDetail({ kind: "tool_call", data: d }))
        .catch((e) => setError(String(e)));
    } else if (selected.kind === "artifact" && session) {
      api
        .artifact(selected.id, session)
        .then((d) => setDetail({ kind: "artifact", data: d }))
        .catch((e) => setError(String(e)));
    } else if (selected.kind === "event") {
      setDetail({ kind: "event", uuid: selected.uuid });
    }
  }, [selected, session]);

  if (!selected) {
    return (
      <aside className="detail-panel">
        <div className="muted">Click a node to inspect it.</div>
      </aside>
    );
  }
  if (error) {
    return (
      <aside className="detail-panel">
        <div className="error">{error}</div>
      </aside>
    );
  }
  if (!detail) {
    return (
      <aside className="detail-panel">
        <div className="muted">loading…</div>
      </aside>
    );
  }
  if (detail.kind === "tool_call") {
    const d = detail.data;
    let prettyInput = d.input_json;
    try {
      prettyInput = JSON.stringify(JSON.parse(d.input_json), null, 2);
    } catch {
      /* leave as-is */
    }
    return (
      <aside className="detail-panel">
        <h2>{d.tool_name}</h2>
        <dl className="kv">
          <dt>id</dt>
          <dd>{d.id}</dd>
          <dt>tool_use_id</dt>
          <dd className="mono">{d.tool_use_id}</dd>
          <dt>duration</dt>
          <dd>{fmtMs(d.duration_ms)}</dd>
          <dt>status</dt>
          <dd>
            {d.success === 1
              ? "ok"
              : d.success === 0
                ? `error: ${d.error_kind ?? "(unknown)"}`
                : "in-flight"}
          </dd>
          <dt>sidechain</dt>
          <dd>{d.is_sidechain ? "yes" : "no"}</dd>
          {d.agent_id && (
            <>
              <dt>agent</dt>
              <dd className="mono">{d.agent_id}</dd>
            </>
          )}
          <dt>artifacts</dt>
          <dd>{d.artifact_count}</dd>
          <dt>result bytes</dt>
          <dd>{d.result_output_bytes ?? "—"}</dd>
        </dl>
        <h3>input</h3>
        <pre className="json">{prettyInput}</pre>
        {d.result_error_text && (
          <>
            <h3>result error</h3>
            <pre className="json">{d.result_error_text}</pre>
          </>
        )}
      </aside>
    );
  }
  if (detail.kind === "artifact") {
    const d = detail.data;
    return (
      <aside className="detail-panel">
        <h2>{d.display}</h2>
        <dl className="kv">
          <dt>kind</dt>
          <dd>{d.kind}</dd>
          <dt>canonical_key</dt>
          <dd className="mono break">{d.canonical_key}</dd>
          <dt>touches</dt>
          <dd>{d.touches.length}</dd>
        </dl>
        <h3>this session</h3>
        <table className="touches">
          <thead>
            <tr>
              <th>ts</th>
              <th>tool</th>
              <th>kind</th>
            </tr>
          </thead>
          <tbody>
            {d.touches.map((t, i) => (
              <tr key={i}>
                <td className="mono">{new Date(t.ts).toISOString().slice(11, 19)}</td>
                <td>{t.tool_name}</td>
                <td>{t.access_kind}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </aside>
    );
  }
  return (
    <aside className="detail-panel">
      <h2>event</h2>
      <dl className="kv">
        <dt>uuid</dt>
        <dd className="mono break">{detail.uuid}</dd>
      </dl>
    </aside>
  );
}
