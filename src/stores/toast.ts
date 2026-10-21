/**
 * Toast 轻量通知体系（v0.1.1）
 *
 * - 成功/信息：4 秒自动消失；
 * - 失败：常驻（手动关闭），可带「详情」打开结构化错误弹窗；
 * - 支持附加动作（如导出后的「打开所在文件夹」）。
 */

import { create } from "zustand";
import { toErrorInfo, type ErrorInfo } from "../lib/errors";

export interface ToastAction {
  label: string;
  run: () => void;
}

export interface ToastItem {
  id: number;
  level: "success" | "error" | "info";
  title: string;
  message?: string;
  /** 结构化错误（失败 + 详情按钮） */
  detail?: ErrorInfo;
  action?: ToastAction;
  /** 常驻（失败默认 true） */
  sticky?: boolean;
  createdAt: number;
}

interface ToastState {
  toasts: ToastItem[];
  push: (toast: Omit<ToastItem, "id" | "createdAt">) => number;
  dismiss: (id: number) => void;
}

let nextToastId = 1;

export const useToastStore = create<ToastState>((set) => ({
  toasts: [],
  push: (toast) => {
    const id = nextToastId++;
    set((state) => ({
      toasts: [...state.toasts, { ...toast, id, createdAt: Date.now() }].slice(-5),
    }));
    return id;
  },
  dismiss: (id) => {
    set((state) => ({ toasts: state.toasts.filter((item) => item.id !== id) }));
  },
}));

/** 命令式辅助（组件外可用） */
export const toast = {
  success(title: string, options?: { message?: string; action?: ToastAction }): number {
    return useToastStore.getState().push({ level: "success", title, ...options });
  },
  info(title: string, options?: { message?: string; action?: ToastAction }): number {
    return useToastStore.getState().push({ level: "info", title, ...options });
  },
  error(
    title: string,
    options?: { message?: string; detail?: ErrorInfo; action?: ToastAction; sticky?: boolean },
  ): number {
    return useToastStore
      .getState()
      .push({ level: "error", title, sticky: options?.sticky ?? true, ...options });
  },
  /** 任意错误 → 失败 toast（结构化错误自动带详情） */
  fromError(error: unknown, fallbackTitle?: string): ErrorInfo {
    const info = toErrorInfo(error);
    useToastStore.getState().push({
      level: "error",
      title: fallbackTitle ?? info.title,
      message: info.message,
      detail: info.kind ? info : undefined,
      sticky: true,
    });
    return info;
  },
};
