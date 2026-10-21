/** 中文文案集中管理（预留多语言结构） */
export const zh = {
  appName: "MCP Tester",
  appSlogan: "MCP 服务接口测试工具",
  nav: {
    servers: "服务器",
    explorer: "接口",
    history: "历史",
    settings: "设置",
  },
  servers: {
    title: "服务器",
    placeholder: "服务器管理将在 M3 提供：添加 stdio / HTTP / SSE 服务器并连接。",
  },
  explorer: {
    title: "接口",
    placeholder:
      "接口浏览与调用台将在 M3 提供：枚举 Tools / Resources / Prompts 并发起调用。",
  },
  history: {
    title: "历史",
    placeholder: "调用历史与重放将在 M4 提供。",
  },
  settings: {
    title: "设置",
    placeholder: "设置项将在 M4 提供。",
    version: "版本",
    dataDir: "数据目录",
    logDir: "日志目录",
    protocolVersions: "协议支持",
    openDataDir: "打开数据目录",
  },
  common: {
    loading: "加载中…",
    loadFailed: "加载失败",
  },
} as const;
