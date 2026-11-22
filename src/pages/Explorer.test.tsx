import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { Explorer } from "./Explorer";
import { useDraftStore } from "../stores/drafts";
import { useReplayStore } from "../stores/replay";
import { useToastStore } from "../stores/toast";
import type { CapabilitySnapshot, ServerSummary } from "../types/models";

const invokeMock = vi.mocked(invoke);

const server: ServerSummary = {
  id: "srv-1",
  name: "Mock Server",
  transport: "stdio",
  enabled: true,
  tags: [],
  last_connected_at: null,
  state: {
    state: "ready",
    handshake: {
      server_name: "mock",
      server_version: "1.0.0",
      protocol_version: "2025-06-18",
      lifecycle: "modern",
      capabilities: {},
      instructions: null,
    },
  },
};

const snapshot: CapabilitySnapshot = {
  server_id: "srv-1",
  tools: [
    {
      name: "echo",
      title: null,
      description: "回显文本",
      input_schema: {
        type: "object",
        properties: { text: { type: "string" } },
        required: ["text"],
      },
      output_schema: null,
      annotations: null,
    },
  ],
  resources: [],
  resource_templates: [
    {
      uri_template: "file:///{path}",
      name: "files",
      title: null,
      description: null,
      mime_type: null,
    },
  ],
  prompts: [
    {
      name: "greet",
      title: null,
      description: null,
      arguments: [{ name: "who", description: null, required: true }],
    },
  ],
  fetched_at: "2026-10-20T00:00:00Z",
  stale: false,
  warnings: [],
};

function renderExplorer() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={["/explorer"]}>
        <Explorer />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

const defaultInvoke = ((cmd: string) => {
  switch (cmd) {
    case "list_servers":
      return Promise.resolve([server]);
    case "get_app_info":
      return Promise.resolve({
        version: "0.1.3",
        data_dir: "C:\\data",
        log_dir: "C:\\logs",
        supported_protocol_versions: ["2026-07-28", "2025-06-18"],
        portable: false,
        data_dir_source: "appdata",
        config_path: null,
        config_status: "not_found",
        config_error: null,
        config_changed: false,
        default_server_id: null,
        auto_connect: false,
      });
    case "get_capabilities":
      return Promise.resolve(snapshot);
    case "preview_capability_export":
      return Promise.resolve(
        "server_name,server_id,category\r\nMock Server,srv-1,tool\r\n",
      );
    case "list_recent_call_params":
      return Promise.resolve([
        {
          id: "call-1",
          created_at: "2026-10-26T10:32:05Z",
          status: "ok",
          duration_ms: 320,
          origin: "manual",
          preview: 'text="loaded"',
          param_count: 1,
          repeat_count: 1,
          params_bytes: 32,
        },
      ]);
    case "get_call_params":
      return Promise.resolve({
        name: "echo",
        arguments: { text: "loaded" },
      });
    case "call_tool":
      return Promise.resolve({
        id: "call-1",
        server_id: "srv-1",
        server_name: "Mock Server",
        method: "tools_call",
        target: "echo",
        params: {},
        result: null,
        error: null,
        status: "ok",
        duration_ms: 1,
        protocol_version: "2026-07-28",
        transport: "stdio",
        truncated: false,
        size_bytes: 0,
        created_at: "2026-10-20T00:00:00Z",
      });
    default:
      return Promise.reject(new Error(`未 mock 的命令：${cmd}`));
  }
}) as typeof invoke;

function mockCapabilities(next: CapabilitySnapshot) {
  invokeMock.mockImplementation(((cmd: string) => {
    if (cmd === "get_capabilities") return Promise.resolve(next);
    return defaultInvoke(cmd);
  }) as typeof invoke);
}

beforeEach(() => {
  invokeMock.mockReset();
  useDraftStore.getState().clearAll();
  useReplayStore.getState().setReplay(null);
  useToastStore.setState({ toasts: [] });
  invokeMock.mockImplementation(defaultInvoke);
});

describe("Explorer 历史重放", () => {
  it("启动配置的默认服务器优先于列表第一项", async () => {
    const second: ServerSummary = {
      ...server,
      id: "srv-2",
      name: "Second Server",
    };
    invokeMock.mockImplementation(((cmd: string) => {
      switch (cmd) {
        case "list_servers":
          return Promise.resolve([server, second]);
        case "get_app_info":
          return Promise.resolve({
            version: "0.1.3",
            data_dir: "C:\\data",
            log_dir: "C:\\logs",
            supported_protocol_versions: ["2026-07-28"],
            portable: false,
            data_dir_source: "appdata",
            config_path: "C:\\mcptester.config.json",
            config_status: "applied",
            config_error: null,
            config_changed: false,
            default_server_id: "srv-2",
            auto_connect: false,
          });
        case "get_capabilities":
          return Promise.resolve({ ...snapshot, server_id: "srv-2" });
        default:
          return Promise.reject(new Error(`未 mock 的命令：${cmd}`));
      }
    }) as typeof invoke);

    renderExplorer();

    const select = await screen.findByRole("combobox");
    await waitFor(() => expect(select).toHaveValue("srv-2"));
  });

  it("工具重放定位到服务器、工具并回填参数", async () => {
    useReplayStore.getState().setReplay({
      serverId: "srv-1",
      method: "tools_call",
      target: "echo",
      params: { name: "echo", arguments: { text: "hello" } },
    });

    renderExplorer();

    const input = await screen.findByLabelText("text");
    expect(input).toHaveValue("hello");
    expect(
      screen.getByText("已载入历史重放：tools/call · echo"),
    ).toBeInTheDocument();
    await waitFor(() => expect(useReplayStore.getState().replay).toBeNull());

    // 参数变化不应导致表单重挂载（历史版本 key 含 JSON.stringify(toolArgs)，会逐字丢焦点）
    await userEvent.type(input, "!");
    expect(input).toHaveValue("hello!");
    expect(input).toHaveFocus();
  });

  it("Prompt 重放回填参数", async () => {
    useReplayStore.getState().setReplay({
      serverId: "srv-1",
      method: "prompts_get",
      target: "greet",
      params: { name: "greet", arguments: { who: "世界" } },
    });

    renderExplorer();

    expect(await screen.findByLabelText("who")).toHaveValue("世界");
    expect(
      screen.getByText("已载入历史重放：prompts/get · greet"),
    ).toBeInTheDocument();
  });

  it("资源重放自动匹配资源模板并回填变量", async () => {
    useReplayStore.getState().setReplay({
      serverId: "srv-1",
      method: "resources_read",
      target: "file:///docs/readme.md",
      params: { uri: "file:///docs/readme.md" },
    });

    renderExplorer();

    expect(await screen.findByLabelText("path")).toHaveValue("docs/readme.md");
    expect(
      screen.getByText(
        "已载入历史重放：resources/read · file:///docs/readme.md",
      ),
    ).toBeInTheDocument();
  });
});

describe("Explorer 能力降级与事件面板", () => {
  it("能力枚举降级提示展示在接口页顶部", async () => {
    invokeMock.mockImplementation(((cmd: string) => {
      switch (cmd) {
        case "list_servers":
          return Promise.resolve([server]);
        case "get_app_info":
          return Promise.resolve({
            version: "0.1.3",
            data_dir: "C:\\data",
            log_dir: "C:\\logs",
            supported_protocol_versions: ["2026-07-28"],
            portable: false,
            data_dir_source: "appdata",
            config_path: null,
            config_status: "not_found",
            config_error: null,
            config_changed: false,
            default_server_id: null,
            auto_connect: false,
          });
        case "get_capabilities":
          return Promise.resolve({
            ...snapshot,
            warnings: [
              "resources/templates/list 返回 -32601（Method not found），已按空列表处理",
            ],
          });
        default:
          return Promise.reject(new Error(`未 mock 的命令：${cmd}`));
      }
    }) as typeof invoke);

    renderExplorer();

    expect(
      await screen.findByText("部分能力未启用或枚举失败："),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/resources\/templates\/list 返回 -32601/),
    ).toBeInTheDocument();
  });

  it("服务器未连接时仍保留事件面板入口", async () => {
    invokeMock.mockImplementation(((cmd: string) => {
      switch (cmd) {
        case "list_servers":
          return Promise.resolve([
            { ...server, state: { state: "disconnected" } },
          ]);
        case "get_app_info":
          return Promise.resolve({
            version: "0.1.3",
            data_dir: "C:\\data",
            log_dir: "C:\\logs",
            supported_protocol_versions: ["2026-07-28"],
            portable: false,
            data_dir_source: "appdata",
            config_path: null,
            config_status: "not_found",
            config_error: null,
            config_changed: false,
            default_server_id: null,
            auto_connect: false,
          });
        default:
          return Promise.reject(new Error(`未 mock 的命令：${cmd}`));
      }
    }) as typeof invoke);

    renderExplorer();

    expect(await screen.findByText("服务器未连接")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /事件面板/ }),
    ).toBeInTheDocument();
  });
});

describe("Explorer 参数校验", () => {
  it("必填缺失时拦截发送，仍要发送可强制调用", async () => {
    renderExplorer();

    await userEvent.click(await screen.findByRole("button", { name: /echo/ }));
    expect(await screen.findByLabelText("text")).toBeInTheDocument();
    expect(screen.queryByText("缺少必填参数「text」")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "发送调用" }));

    expect(screen.getByText("缺少必填参数「text」")).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("call_tool", expect.anything());

    await userEvent.click(screen.getByRole("button", { name: "仍要发送" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "call_tool",
        expect.objectContaining({ id: "srv-1", name: "echo", arguments: {} }),
      ),
    );
  });

  it("填写必填参数后正常发送", async () => {
    renderExplorer();

    await userEvent.click(await screen.findByRole("button", { name: /echo/ }));
    const input = await screen.findByLabelText("text");
    await userEvent.type(input, "hello");
    await userEvent.click(screen.getByRole("button", { name: "发送调用" }));

    expect(
      screen.queryByRole("button", { name: "仍要发送" }),
    ).not.toBeInTheDocument();
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "call_tool",
        expect.objectContaining({
          id: "srv-1",
          name: "echo",
          arguments: { text: "hello" },
        }),
      ),
    );
  });

  it("Prompt 必填缺失时拦截获取", async () => {
    renderExplorer();

    await userEvent.click(
      await screen.findByRole("button", { name: /^Prompts/ }),
    );
    await userEvent.click(await screen.findByRole("button", { name: /greet/ }));
    await userEvent.click(screen.getByRole("button", { name: "获取提示词" }));

    expect(screen.getByText("缺少必填参数：who")).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith(
      "get_prompt",
      expect.anything(),
    );
    expect(
      screen.getByRole("button", { name: "仍要发送" }),
    ).toBeInTheDocument();
  });

  it("历史参数可回填、可撤销，且不触发调用", async () => {
    renderExplorer();

    await userEvent.click(await screen.findByRole("button", { name: /echo/ }));
    const input = await screen.findByLabelText("text");
    await userEvent.click(screen.getByRole("button", { name: /历史参数/ }));
    await userEvent.click(await screen.findByText('text="loaded"'));

    await waitFor(() => expect(input).toHaveValue("loaded"));
    expect(screen.getByText(/已载入/)).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("call_tool", expect.anything());

    await userEvent.click(screen.getByRole("button", { name: "撤销" }));
    expect(input).toHaveValue("");
  });

  it("Ctrl+Enter 发送工具调用，输入法组合态不发送", async () => {
    renderExplorer();

    await userEvent.click(await screen.findByRole("button", { name: /echo/ }));
    const input = await screen.findByLabelText("text");
    await userEvent.type(input, "hello");

    fireEvent.keyDown(input, {
      key: "Enter",
      ctrlKey: true,
      isComposing: true,
    });
    expect(invokeMock).not.toHaveBeenCalledWith("call_tool", expect.anything());

    await userEvent.keyboard("{Control>}{Enter}{/Control}");
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "call_tool",
        expect.objectContaining({
          name: "echo",
          arguments: { text: "hello" },
        }),
      ),
    );
  });

  it("切换页签后回到工具，恢复未发送草稿；清空后不恢复", async () => {
    renderExplorer();

    await userEvent.click(await screen.findByRole("button", { name: /echo/ }));
    const input = await screen.findByLabelText("text");
    await userEvent.type(input, "draft-value");

    await userEvent.click(
      await screen.findByRole("button", { name: /^Prompts/ }),
    );
    await userEvent.click(screen.getByRole("button", { name: /^Tools/ }));
    await userEvent.click(screen.getByRole("button", { name: /echo/ }));
    expect(await screen.findByLabelText("text")).toHaveValue("draft-value");

    await userEvent.click(screen.getByTitle("清空当前参数"));
    await userEvent.click(
      await screen.findByRole("button", { name: /^Prompts/ }),
    );
    await userEvent.click(screen.getByRole("button", { name: /^Tools/ }));
    await userEvent.click(screen.getByRole("button", { name: /echo/ }));
    expect(await screen.findByLabelText("text")).toHaveValue("");
  });

  it("切页签保留结果并提示旧结果归属，切回后恢复", async () => {
    renderExplorer();

    await userEvent.click(await screen.findByRole("button", { name: /echo/ }));
    await userEvent.type(await screen.findByLabelText("text"), "hello");
    await userEvent.click(screen.getByRole("button", { name: "发送调用" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("call_tool", expect.anything()),
    );

    await userEvent.click(
      await screen.findByRole("button", { name: /^Prompts/ }),
    );
    expect(
      await screen.findByText(/上一次调用结果属于 工具「echo」/),
    ).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /^Tools/ }));
    await userEvent.click(screen.getByRole("button", { name: /echo/ }));
    await waitFor(() =>
      expect(
        screen.queryByText(/上一次调用结果属于 工具「echo」/),
      ).not.toBeInTheDocument(),
    );
  });

  it("JSON 解析失败时不发送，也不提供仍要发送", async () => {
    renderExplorer();

    await userEvent.click(await screen.findByRole("button", { name: /echo/ }));
    await screen.findByLabelText("text");
    await userEvent.click(screen.getByRole("button", { name: "JSON 模式" }));
    const textarea = screen.getByLabelText("参数 JSON");
    await userEvent.clear(textarea);
    await userEvent.type(textarea, "{{");

    await userEvent.click(screen.getByRole("button", { name: "发送调用" }));

    expect(screen.getByText(/JSON 解析失败/)).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "仍要发送" }),
    ).not.toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("call_tool", expect.anything());
  });
});

describe("Explorer 接口搜索（v0.7.1）", () => {
  it("默认只搜索名称，切换名称+内容后显示描述命中", async () => {
    renderExplorer();

    await screen.findByRole("button", { name: /echo/ });
    const search = screen.getByRole("textbox", { name: "搜索接口" });
    await userEvent.type(search, "回显");

    expect(screen.getByText("没有匹配的接口")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /echo/ }),
    ).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "搜索全部内容" }));

    expect(
      await screen.findByRole("button", { name: /echo/ }),
    ).toBeInTheDocument();
    expect(screen.getByText("描述")).toBeInTheDocument();
    expect(screen.getByText("1 / 1")).toBeInTheDocument();
  });

  it("名称+内容可命中 Tool Schema 字段名", async () => {
    renderExplorer();

    await screen.findByRole("button", { name: /echo/ });
    const search = screen.getByRole("textbox", { name: "搜索接口" });
    await userEvent.type(search, "text");
    expect(screen.getByText("没有匹配的接口")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "名称+内容" }));

    expect(
      await screen.findByRole("button", { name: /echo/ }),
    ).toBeInTheDocument();
    expect(screen.getByText("字段")).toBeInTheDocument();
  });

  it("名称命中与内容命中分组展示", async () => {
    mockCapabilities({
      ...snapshot,
      tools: [
        snapshot.tools[0],
        {
          name: "search_files",
          title: null,
          description: "在项目中搜索文件",
          input_schema: null,
          output_schema: null,
          annotations: null,
        },
        {
          name: "weather",
          title: null,
          description: "查询天气。search 关键词只应作为内容命中",
          input_schema: null,
          output_schema: null,
          annotations: null,
        },
      ],
    });
    renderExplorer();

    const search = await screen.findByRole("textbox", { name: "搜索接口" });
    await userEvent.type(search, "search");
    await userEvent.click(screen.getByRole("button", { name: "名称+内容" }));

    expect(screen.getByText("名称匹配")).toBeInTheDocument();
    expect(screen.getByText("内容匹配")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /search_files/ }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /weather/ })).toBeInTheDocument();
    expect(screen.getByText("2 / 3")).toBeInTheDocument();
  });

  it("Prompts 参数名在名称+内容范围可搜索", async () => {
    renderExplorer();

    await userEvent.click(
      await screen.findByRole("button", { name: /^Prompts/ }),
    );
    const search = screen.getByRole("textbox", { name: "搜索接口" });
    await userEvent.type(search, "who");
    expect(screen.getByText("没有匹配的接口")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "名称+内容" }));

    expect(
      await screen.findByRole("button", { name: /greet/ }),
    ).toBeInTheDocument();
    expect(screen.getByText("参数")).toBeInTheDocument();
  });

  it("选中项被搜索过滤时保留选择并提示", async () => {
    renderExplorer();

    await userEvent.click(await screen.findByRole("button", { name: /echo/ }));
    const search = screen.getByRole("textbox", { name: "搜索接口" });
    await userEvent.type(search, "greet");

    expect(screen.getByText("当前选中项不在筛选结果中")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "清空搜索" }));
    expect(
      screen.queryByText("当前选中项不在筛选结果中"),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /echo/ })).toBeInTheDocument();
  });

  it("Esc 清空搜索，Ctrl+F 聚焦搜索框", async () => {
    renderExplorer();

    await screen.findByRole("button", { name: /echo/ });
    const search = screen.getByRole("textbox", { name: "搜索接口" });
    await userEvent.type(search, "echo");
    expect(search).toHaveValue("echo");

    await userEvent.keyboard("{Escape}");
    expect(search).toHaveValue("");
    expect(screen.getByRole("button", { name: /echo/ })).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /echo/ }));
    await userEvent.keyboard("{Control>}f{/Control}");
    expect(search).toHaveFocus();
  });
});

describe("Explorer 接口导出（v0.6.1）", () => {
  it("导出接口按钮调用预览命令，并支持切换格式", async () => {
    renderExplorer();

    const exportButton = await screen.findByRole("button", {
      name: "导出接口",
    });
    await waitFor(() => expect(exportButton).toBeEnabled());
    await userEvent.click(exportButton);

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("preview_capability_export", {
        id: "srv-1",
        format: "csv",
        includeSchema: false,
      }),
    );

    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "导出格式" }),
      "json",
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("preview_capability_export", {
        id: "srv-1",
        format: "json",
        includeSchema: false,
      }),
    );

    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "导出格式" }),
      "csv_params",
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("preview_capability_export", {
        id: "srv-1",
        format: "csv_params",
        includeSchema: false,
      }),
    );
  });

  it("未连接时导出按钮禁用", async () => {
    const disconnected: ServerSummary = {
      ...server,
      state: { state: "disconnected" },
    };
    invokeMock.mockImplementation(((cmd: string) => {
      switch (cmd) {
        case "list_servers":
          return Promise.resolve([disconnected]);
        case "get_app_info":
          return Promise.resolve(null);
        default:
          return Promise.resolve(null);
      }
    }) as typeof invoke);

    renderExplorer();
    const exportButton = await screen.findByRole("button", {
      name: "导出接口",
    });
    expect(exportButton).toBeDisabled();
  });

  it("现代 Streamable HTTP 工具显示 x-mcp-header 提示", async () => {
    const baseHandshake =
      server.state.state === "ready"
        ? server.state.handshake
        : {
            server_name: "mock",
            server_version: "1.0.0",
            protocol_version: "2026-07-28",
            lifecycle: "modern" as const,
            capabilities: {},
            instructions: null,
          };
    const modernServer: ServerSummary = {
      ...server,
      transport: "streamable_http",
      state: {
        state: "ready",
        handshake: {
          ...baseHandshake,
          protocol_version: "2026-07-28",
          lifecycle: "modern",
        },
      },
    };
    const modernSnapshot: CapabilitySnapshot = {
      ...snapshot,
      tools: [
        {
          ...snapshot.tools[0],
          input_schema: {
            type: "object",
            properties: {
              text: { type: "string" },
              region: { type: "string", "x-mcp-header": "Region" },
            },
            required: ["text"],
          },
        },
      ],
    };
    invokeMock.mockImplementation(((cmd: string) => {
      switch (cmd) {
        case "list_servers":
          return Promise.resolve([modernServer]);
        case "get_app_info":
          return Promise.resolve(null);
        case "get_capabilities":
          return Promise.resolve(modernSnapshot);
        case "list_recent_call_params":
          return Promise.resolve([]);
        default:
          return Promise.resolve(null);
      }
    }) as typeof invoke);

    renderExplorer();
    await userEvent.click(await screen.findByRole("button", { name: /echo/ }));
    expect(
      await screen.findByText(/该工具会将 1 个参数作为/),
    ).toBeInTheDocument();
    expect(screen.getByText("HTTP 头")).toBeInTheDocument();
  });
});
