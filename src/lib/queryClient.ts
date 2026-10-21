import { QueryClient } from "@tanstack/react-query";

/** 全局 QueryClient（本地桌面应用：窗口失焦不刷新、失败重试 1 次） */
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
});
