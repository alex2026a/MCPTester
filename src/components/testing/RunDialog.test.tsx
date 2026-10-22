import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { RunDialog } from "./RunDialog";

const invokeMock = vi.mocked(invoke);

function renderDialog() {
  const client = new QueryClient({
    defaultOptions: { mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <RunDialog
        mode="suite"
        suiteId="suite-1"
        serverId="srv-1"
        caseIds={[]}
        caseCount={0}
        onClose={() => {}}
        onStarted={() => {}}
      />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue("run-1");
});

describe("RunDialog", () => {
  it("默认不授权 write/destructive/unknown，并启动运行", async () => {
    renderDialog();
    await userEvent.click(screen.getByRole("button", { name: "开始运行" }));
    expect(invokeMock).toHaveBeenCalledWith("run_test_suite", {
      suiteId: "suite-1",
      options: expect.objectContaining({
        allow_write: false,
        allow_destructive: false,
        allow_unknown: false,
        stop_on_failure: false,
        retry_count: 0,
      }),
    });
  });
});
