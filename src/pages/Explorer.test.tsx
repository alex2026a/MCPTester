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

beforeEach(() => {
  invokeMock.mockReset();
  useDraftStore.getState().clearAll();
  useReplayStore.getState().setReplay(null);
  useToastStore.setState({ toasts: [] });
  invokeMock.mockImplementation(((cmd: string) => {
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
  }) as typeof invoke);
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
          return Promise.resolve([{ ...server, state: { state: "disconnected" } }]);
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
