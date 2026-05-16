import { create } from "zustand";
import type { GraphMode } from "./api";

export type SelectedNode =
  | { kind: "tool_call"; id: number }
  | { kind: "artifact"; id: number }
  | { kind: "event"; uuid: string }
  | null;

type UiState = {
  selectedSessionId: string | null;
  graphMode: GraphMode;

  // Replay state (driven by the WS client).
  cursor: number | null;
  fromTs: number | null;
  toTs: number | null;
  playing: boolean;
  speed: number;
  replayError: string | null;

  // Right-panel selection.
  selectedNode: SelectedNode;

  setSession: (id: string | null) => void;
  setGraphMode: (m: GraphMode) => void;
  setBounds: (from: number, to: number) => void;
  setCursor: (ts: number) => void;
  setPlaying: (p: boolean) => void;
  setSpeed: (s: number) => void;
  setSelectedNode: (n: SelectedNode) => void;
};

export const useUi = create<UiState>((set) => ({
  selectedSessionId: null,
  graphMode: "bipartite",
  cursor: null,
  fromTs: null,
  toTs: null,
  playing: false,
  speed: 1,
  replayError: null,
  selectedNode: null,
  setSession: (id) => set({ selectedSessionId: id, selectedNode: null, replayError: null }),
  setGraphMode: (m) => set({ graphMode: m }),
  setBounds: (from, to) => set({ fromTs: from, toTs: to, cursor: from }),
  setCursor: (ts) => set({ cursor: ts }),
  setPlaying: (p) => set({ playing: p }),
  setSpeed: (s) => set({ speed: s }),
  setSelectedNode: (n) => set({ selectedNode: n }),
}));
