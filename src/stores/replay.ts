import { create } from "zustand";
import type { CallMethod } from "../types/models";

/** 待重放的调用（历史页 → 接口页） */
export interface ReplayTarget {
  serverId: string;
  method: CallMethod;
  target: string;
  params: unknown;
}

interface ReplayState {
  replay: ReplayTarget | null;
  setReplay: (target: ReplayTarget | null) => void;
}

/** 重放状态：历史页写入，接口页消费后清空 */
export const useReplayStore = create<ReplayState>((set) => ({
  replay: null,
  setReplay: (replay) => set({ replay }),
}));
