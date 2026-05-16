import { create } from "zustand";
import type { GraphMode } from "./api";

type UiState = {
  selectedSessionId: string | null;
  graphMode: GraphMode;
  setSession: (id: string | null) => void;
  setGraphMode: (m: GraphMode) => void;
};

export const useUi = create<UiState>((set) => ({
  selectedSessionId: null,
  graphMode: "bipartite",
  setSession: (id) => set({ selectedSessionId: id }),
  setGraphMode: (m) => set({ graphMode: m }),
}));
