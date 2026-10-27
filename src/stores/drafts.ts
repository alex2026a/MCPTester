import { create } from "zustand";
import type { CallMethod } from "../types/models";

type DraftEntry = {
  args: Record<string, unknown>;
  updatedAt: number;
};

interface DraftState {
  drafts: Record<string, DraftEntry>;
  save: (key: string, args: Record<string, unknown>) => void;
  get: (key: string) => Record<string, unknown> | null;
  clear: (key: string) => void;
  clearAll: () => void;
}

/** 按接口保留未发送参数（仅内存，退出应用即丢弃）。 */
export const useDraftStore = create<DraftState>((set, get) => ({
  drafts: {},
  save: (key, args) => {
    if (Object.keys(args).length === 0) {
      set((state) => {
        if (!(key in state.drafts)) return state;
        const next = { ...state.drafts };
        delete next[key];
        return { drafts: next };
      });
      return;
    }
    set((state) => ({
      drafts: {
        ...state.drafts,
        [key]: { args: { ...args }, updatedAt: Date.now() },
      },
    }));
  },
  get: (key) => {
    const entry = get().drafts[key];
    return entry ? { ...entry.args } : null;
  },
  clear: (key) =>
    set((state) => {
      if (!(key in state.drafts)) return state;
      const next = { ...state.drafts };
      delete next[key];
      return { drafts: next };
    }),
  clearAll: () => set({ drafts: {} }),
}));

/** 草稿键：server + method + target。 */
export function draftKey(
  serverId: string,
  method: CallMethod,
  target: string,
): string {
  return `${serverId}:${method}:${target}`;
}
