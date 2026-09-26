import { create } from "zustand";
import type { LoweringReplay, PassReplay, PipelineReport, ServerMessage } from "./types";

interface PipelineState {
  connected: boolean;
  connecting: boolean;
  watchedPath: string | null;
  sequence: number | null;
  report: PipelineReport | null;
  passReplay: PassReplay | null;
  loweringReplay: LoweringReplay | null;
  error: string | null;
  setConnected: (connected: boolean) => void;
  setConnecting: (connecting: boolean) => void;
  setWatchedPath: (path: string | null) => void;
  handleMessage: (msg: ServerMessage) => void;
  reset: () => void;
}

// A `sequence` guard: any replay event tagged with a sequence older than the
// current report's is discarded — the frontend keeps editing while a
// compile is in flight, and this keeps a late-arriving reply from a stale
// compile from overwriting what's on screen.
export const usePipelineStore = create<PipelineState>((set, get) => ({
  connected: false,
  connecting: false,
  watchedPath: null,
  sequence: null,
  report: null,
  passReplay: null,
  loweringReplay: null,
  error: null,

  setConnected: (connected) => set({ connected }),
  setConnecting: (connecting) => set({ connecting }),
  setWatchedPath: (watchedPath) => set({ watchedPath }),

  handleMessage: (msg) => {
    switch (msg.kind) {
      case "pipeline_report":
        set({
          sequence: msg.sequence,
          report: msg.report,
          error: null,
          passReplay: null,
          loweringReplay: null,
        });
        break;
      case "pass_replay":
        if (msg.sequence === get().sequence) set({ passReplay: msg.replay });
        break;
      case "lowering_replay":
        if (msg.sequence === get().sequence) set({ loweringReplay: msg.replay });
        break;
      case "compile_error":
        if (msg.sequence >= (get().sequence ?? -1)) {
          set({ error: msg.message });
        }
        break;
    }
  },

  reset: () =>
    set({
      report: null,
      passReplay: null,
      loweringReplay: null,
      error: null,
      sequence: null,
    }),
}));
