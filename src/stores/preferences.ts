import { create } from "zustand";

interface PreferenceState {
  /** 表单「必填优先」开关：默认关闭，保持服务端原始声明顺序 */
  requiredFirst: boolean;
  setRequiredFirst: (value: boolean) => void;
}

/** 界面偏好（仅内存，退出应用后恢复默认）。 */
export const usePreferenceStore = create<PreferenceState>((set) => ({
  requiredFirst: false,
  setRequiredFirst: (requiredFirst) => set({ requiredFirst }),
}));
