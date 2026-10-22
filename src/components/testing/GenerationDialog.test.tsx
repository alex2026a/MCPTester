import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { GenerationDialog } from "./GenerationDialog";
import type { GenerationPreview } from "../../types/testing";

const invokeMock = vi.mocked(invoke);

const preview: GenerationPreview = {
  server_id: "srv-1",
  capability_hash: "a".repeat(32),
  capability_snapshot: { server_id: "srv-1", tools: [] },
  warnings: ["示例提示"],
  stats: {
    total: 1,
    positive: 1,
    boundary: 0,
    negative: 0,
    resources: 0,
    prompts: 0,
    read_only: 1,
    write: 0,
    destructive: 0,
    unknown: 0,
  },
  cases: [
    {
      name: "echo · 正向基线",
      description: "",
      method: "tools_call",
      target: "echo",
      arguments: { text: "hi" },
      expect: { outcome: "success", accept_as_warning: true },
      assertions: [],
      tags: ["positive"],
      priority: "p0",
      risk: "read_only",
      source: "schema",
      generation_note: "required text",
      enabled: true,
      timeout_secs: null,
      retry_count: 0,
    },
  ],
};

function renderDialog() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <GenerationDialog
        suiteId="suite-1"
        serverId="srv-1"
        onClose={() => {}}
        onSaved={() => {}}
      />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(((cmd: string) => {
    if (cmd === "generate_test_cases") return Promise.resolve(preview);
    if (cmd === "save_generated_cases") {
      return Promise.resolve([{ ...preview.cases[0], id: "case-1", suite_id: "suite-1" }]);
    }
    return Promise.resolve(null);
  }) as never);
});

describe("GenerationDialog", () => {
  it("生成预览并保存所选用例", async () => {
    renderDialog();
    await userEvent.click(screen.getByRole("button", { name: "生成预览" }));

    expect(await screen.findByText("echo · 正向基线")).toBeInTheDocument();
    expect(screen.getByText(/共 1 条/)).toBeInTheDocument();
    expect(screen.getByText("示例提示")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "接受所选" }));
    expect(invokeMock).toHaveBeenCalledWith(
      "save_generated_cases",
      expect.objectContaining({
        suiteId: "suite-1",
        capabilityHash: preview.capability_hash,
      }),
    );
  });
});
