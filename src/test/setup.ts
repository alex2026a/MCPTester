import "@testing-library/jest-dom/vitest";
import { configure } from "@testing-library/react";
import { vi } from "vitest";

// 本机 jsdom 环境初始化较慢，放宽异步断言等待（避免并发跑测时假超时）
configure({ asyncUtilTimeout: 5000 });

// Tauri IPC 全局 mock（各测试按需注入实现）
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

// Tauri 事件模块全局 mock（组件挂载时不会真实调用 IPC）
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
