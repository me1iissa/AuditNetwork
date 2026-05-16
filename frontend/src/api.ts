// Thin fetch wrappers around the Rust backend.

export type SessionSummary = {
  id: string;
  ai_title: string | null;
  started_at: number;
  ended_at: number | null;
  claude_version: string | null;
  model: string | null;
  cwd: string | null;
  git_branch: string | null;
  total_input_tokens: number;
  total_output_tokens: number;
  total_cache_read: number;
  total_cache_creation: number;
  tool_calls: number;
  artifacts_touched: number;
  events_total: number;
};

export type GraphMode = "bipartite" | "causal";

export type GraphNode = {
  id: string;
  kind: string;
  label: string;
  ts: number;
  sub?: string;
};

export type GraphEdge = {
  source: string;
  target: string;
  ts: number;
  label: string;
};

export type GraphResponse = {
  mode: string;
  nodes: GraphNode[];
  edges: GraphEdge[];
};

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(`${url} → HTTP ${res.status}`);
  }
  return (await res.json()) as T;
}

export type ToolCallDetail = {
  id: number;
  event_uuid: string;
  session_id: string;
  ts: number;
  tool_use_id: string;
  tool_name: string;
  input_json: string;
  duration_ms: number | null;
  success: number | null;
  error_kind: string | null;
  is_sidechain: number;
  agent_id: string | null;
  result_output_bytes: number | null;
  result_is_error: number | null;
  result_error_text: string | null;
  artifact_count: number;
};

export type ArtifactTouch = {
  tool_call_id: number;
  ts: number;
  tool_name: string;
  access_kind: string;
};

export type ArtifactDetail = {
  id: number;
  kind: string;
  canonical_key: string;
  display: string;
  first_seen_ts: number;
  last_seen_ts: number;
  touches: ArtifactTouch[];
};

export const api = {
  listSessions: () => getJson<SessionSummary[]>("/api/sessions"),
  sessionGraph: (id: string, mode: GraphMode) =>
    getJson<GraphResponse>(`/api/sessions/${id}/graph?mode=${mode}`),
  toolCall: (id: number) => getJson<ToolCallDetail>(`/api/tool_calls/${id}`),
  artifact: (id: number, sessionId: string) =>
    getJson<ArtifactDetail>(
      `/api/artifacts/${id}?session_id=${encodeURIComponent(sessionId)}`,
    ),
};
