import { useEffect, useRef, useState } from "react";
import cytoscape, { type Core, type ElementDefinition } from "cytoscape";
import fcose from "cytoscape-fcose";
import dagre from "cytoscape-dagre";
import { api, type GraphResponse } from "../api";
import { useUi } from "../store";

cytoscape.use(fcose);
cytoscape.use(dagre);

const TOOL_COLOR: Record<string, string> = {
  Bash: "#e8a33d",
  Read: "#5b9bd5",
  Write: "#70ad47",
  Edit: "#70ad47",
  WebFetch: "#9c27b0",
  WebSearch: "#9c27b0",
  Agent: "#d33682",
  Grep: "#268bd2",
  Glob: "#268bd2",
};

const ARTIFACT_COLOR: Record<string, string> = {
  file: "#cccccc",
  url: "#9c27b0",
  command: "#e8a33d",
  glob_pattern: "#268bd2",
  mcp_resource: "#2aa198",
  agent: "#d33682",
};

function buildElements(g: GraphResponse): ElementDefinition[] {
  const els: ElementDefinition[] = [];
  for (const n of g.nodes) {
    const palette = n.kind === "tool_call" ? TOOL_COLOR : ARTIFACT_COLOR;
    const color = palette[n.sub ?? ""] ?? (n.kind === "tool_call" ? "#888" : "#bbb");
    els.push({
      data: {
        id: n.id,
        label: n.label,
        kind: n.kind,
        sub: n.sub ?? "",
        ts: n.ts,
        color,
      },
    });
  }
  for (const e of g.edges) {
    els.push({
      data: {
        id: `${e.source}->${e.target}-${e.label}`,
        source: e.source,
        target: e.target,
        label: e.label,
        ts: e.ts,
      },
    });
  }
  return els;
}

function parseSelected(elId: string): { kind: "tool_call" | "artifact"; id: number } | null {
  // bipartite IDs are "tc:123" or "art:5"
  const m = elId.match(/^(tc|art):(\d+)$/);
  if (!m) return null;
  return { kind: m[1] === "tc" ? "tool_call" : "artifact", id: parseInt(m[2], 10) };
}

export function GraphCanvas() {
  const ref = useRef<HTMLDivElement>(null);
  const cyRef = useRef<Core | null>(null);
  const sessionId = useUi((s) => s.selectedSessionId);
  const mode = useUi((s) => s.graphMode);
  const setMode = useUi((s) => s.setGraphMode);
  const setSelectedNode = useUi((s) => s.setSelectedNode);
  const cursor = useUi((s) => s.cursor);
  const fromTs = useUi((s) => s.fromTs);
  const toTs = useUi((s) => s.toTs);
  const [stats, setStats] = useState<{ nodes: number; edges: number } | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!sessionId || !ref.current) return;
    let cancelled = false;
    setError(null);
    api
      .sessionGraph(sessionId, mode)
      .then((g) => {
        if (cancelled) return;
        setStats({ nodes: g.nodes.length, edges: g.edges.length });
        if (cyRef.current) {
          cyRef.current.destroy();
          cyRef.current = null;
        }
        const cy = cytoscape({
          container: ref.current!,
          elements: buildElements(g),
          style: [
            {
              selector: "node",
              style: {
                "background-color": "data(color)",
                label: "data(label)",
                color: "#ddd",
                "font-size": 9,
                "text-valign": "center",
                "text-halign": "center",
                "text-margin-y": -10,
                "text-wrap": "ellipsis",
                "text-max-width": "120",
                width: 14,
                height: 14,
              } as cytoscape.Css.Node,
            },
            {
              selector: 'node[kind = "artifact"]',
              style: { shape: "round-rectangle", width: 16, height: 10 } as cytoscape.Css.Node,
            },
            {
              selector: 'node[kind = "event"]',
              style: { width: 8, height: 8 } as cytoscape.Css.Node,
            },
            {
              selector: ".faded",
              style: { opacity: 0.08 } as cytoscape.Css.Node,
            },
            {
              selector: "edge",
              style: {
                "curve-style": "bezier",
                "line-color": "#444",
                width: 1,
                "target-arrow-shape": mode === "causal" ? "triangle" : "none",
                "target-arrow-color": "#444",
                "arrow-scale": 0.6,
                opacity: 0.55,
              } as cytoscape.Css.Edge,
            },
            {
              selector: "edge.faded",
              style: { opacity: 0.04 } as cytoscape.Css.Edge,
            },
            {
              selector: ":selected",
              style: { "border-width": 2, "border-color": "#fff" } as cytoscape.Css.Node,
            },
          ],
          layout:
            mode === "causal"
              ? ({ name: "dagre", rankDir: "TB", nodeSep: 6, rankSep: 14 } as cytoscape.LayoutOptions)
              : ({
                  name: "fcose",
                  animate: false,
                  randomize: true,
                  nodeSeparation: 60,
                } as cytoscape.LayoutOptions),
          wheelSensitivity: 0.25,
        });
        cy.on("tap", "node", (evt) => {
          const id = evt.target.id() as string;
          if (mode === "causal") {
            setSelectedNode({ kind: "event", uuid: id });
          } else {
            const sel = parseSelected(id);
            if (sel) setSelectedNode(sel);
          }
        });
        cy.on("tap", (evt) => {
          if (evt.target === cy) setSelectedNode(null);
        });
        cyRef.current = cy;
      })
      .catch((e) => setError(String(e)));
    return () => {
      cancelled = true;
    };
  }, [sessionId, mode, setSelectedNode]);

  // Cursor → opacity: any element with ts > cursor gets `.faded`.
  // When cursor is null OR before any bounds exist, show everything.
  //
  // `stats` is in the deps deliberately: it changes once per new graph
  // load (session switch / mode toggle) and the new Cytoscape `cy`
  // instance hasn't been seen by the previous effect run, so we need to
  // re-apply the fade pass to the fresh element set.
  useEffect(() => {
    const cy = cyRef.current;
    if (!cy) return;
    if (cursor == null || fromTs == null || toTs == null) {
      cy.elements().removeClass("faded");
      return;
    }
    cy.batch(() => {
      cy.elements().forEach((el) => {
        const ts = el.data("ts") as number | undefined;
        const faded = ts != null && ts > cursor && ts > 0;
        el.toggleClass("faded", faded);
      });
    });
  }, [cursor, fromTs, toTs, stats]);

  useEffect(
    () => () => {
      if (cyRef.current) cyRef.current.destroy();
    },
    [],
  );

  return (
    <main className="canvas-pane">
      <div className="canvas-header">
        <div className="mode-toggle">
          <button
            className={mode === "bipartite" ? "active" : ""}
            onClick={() => setMode("bipartite")}
          >
            Bipartite
          </button>
          <button
            className={mode === "causal" ? "active" : ""}
            onClick={() => setMode("causal")}
          >
            Causal
          </button>
        </div>
        <div className="stats">
          {stats
            ? `${stats.nodes} nodes · ${stats.edges} edges`
            : sessionId
              ? "loading…"
              : "no session selected"}
        </div>
      </div>
      {error && <div className="error">{error}</div>}
      <div ref={ref} className="cy" />
    </main>
  );
}
