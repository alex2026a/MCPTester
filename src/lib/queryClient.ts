import { QueryClient, type DefaultOptions } from "@tanstack/react-query";

/**
 * 本地桌面应用：所有数据都经 Tauri IPC 读写本地，真实网络访问在 Rust 侧。
 * 因此查询/变更不能被浏览器的在线状态暂停（默认 online 模式在断网时会把
 * invoke 查询挂起，表现为“详情加载失败”）。
 */
const LOCAL_DEFAULTS: DefaultOptions = {
  queries: {
    retry: 1,
    refetchOnWindowFocus: false,
    networkMode: "always",
  },
  mutations: {
    networkMode: "always",
  },
};

/** 按本地应用默认值创建 QueryClient（测试可覆盖部分默认项） */
export function createQueryClient(overrides?: DefaultOptions): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { ...LOCAL_DEFAULTS.queries, ...overrides?.queries },
      mutations: { ...LOCAL_DEFAULTS.mutations, ...overrides?.mutations },
    },
  });
}

/** 全局 QueryClient（窗口失焦不刷新、失败重试 1 次） */
export const queryClient = createQueryClient();
