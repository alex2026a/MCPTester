/**
 * 交互输入待处理队列（v0.6.0）
 *
 * 事件由 `InputRequestHost` 写入；Dialog 只读 store，避免在 effect 中 setState。
 */

import { create } from "zustand";
import type { InputContext, InputEvent, InputRequestView } from "../types/input";

export interface PendingInput {
  requestId: string;
  context: InputContext;
  requests: InputRequestView[];
  expiresAt: string;
  minimized: boolean;
}

interface InputRequestsState {
  pending: PendingInput[];
  activeId: string | null;
  open: (event: Extract<InputEvent, { type: "opened" }>) => void;
  close: (requestId: string, _reason?: string) => void;
  minimize: (requestId: string) => void;
  restore: (requestId: string) => void;
  setActive: (requestId: string | null) => void;
  reset: () => void;
}

function firstVisible(pending: PendingInput[], exclude?: string): string | null {
  return (
    pending.find((item) => item.requestId !== exclude && !item.minimized)?.requestId ??
    null
  );
}

export const useInputRequests = create<InputRequestsState>((set) => ({
  pending: [],
  activeId: null,
  open: (event) => {
    set((state) => {
      const next: PendingInput = {
        requestId: event.request_id,
        context: event.context,
        requests: event.requests,
        expiresAt: event.expires_at,
        minimized: false,
      };
      const exists = state.pending.some(
        (item) => item.requestId === event.request_id,
      );
      const pending = exists
        ? state.pending.map((item) =>
            item.requestId === event.request_id ? next : item,
          )
        : [...state.pending, next];
      return { pending, activeId: event.request_id };
    });
  },
  close: (requestId) => {
    set((state) => {
      const pending = state.pending.filter(
        (item) => item.requestId !== requestId,
      );
      const activeId =
        state.activeId === requestId
          ? firstVisible(pending)
          : state.activeId;
      return { pending, activeId };
    });
  },
  minimize: (requestId) => {
    set((state) => {
      const pending = state.pending.map((item) =>
        item.requestId === requestId ? { ...item, minimized: true } : item,
      );
      return {
        pending,
        activeId:
          state.activeId === requestId
            ? firstVisible(pending, requestId)
            : state.activeId,
      };
    });
  },
  restore: (requestId) => {
    set((state) => ({
      pending: state.pending.map((item) =>
        item.requestId === requestId ? { ...item, minimized: false } : item,
      ),
      activeId: requestId,
    }));
  },
  setActive: (requestId) => {
    set({ activeId: requestId });
  },
  reset: () => {
    set({ pending: [], activeId: null });
  },
}));
