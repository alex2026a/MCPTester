import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { CaseEditor } from "./CaseEditor";

const invokeMock = vi.mocked(invoke);

function renderEditor() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <CaseEditor
        suiteId="suite-1"
        onClose={() => {}}
        onSaved={() => {}}
      />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((() => Promise.resolve(null)) as never);
});

describe("CaseEditor", () => {
  it("底栏展示保存禁用原因，必填补齐后可保存", async () => {
    renderEditor();

    const save = screen.getByRole("button", { name: "保存" });
    expect(save).toBeDisabled();
    expect(screen.getByText("名称不能为空")).toBeInTheDocument();

    await userEvent.type(screen.getByLabelText("名称"), "echo 用例");
    await userEvent.type(screen.getByLabelText("目标"), "echo");
    expect(save).toBeEnabled();
    expect(screen.getByText("参数与断言校验通过")).toBeInTheDocument();
  });

  it("参数与断言分页签编辑，切换后保留输入并标记非法 JSON", async () => {
    renderEditor();

    const args = screen.getByLabelText("参数（JSON）");
    await userEvent.clear(args);
    await userEvent.type(args, "123");

    await userEvent.click(
      screen.getByRole("button", { name: "断言（JSON 数组）" }),
    );
    const assertions = screen.getByLabelText("断言（JSON 数组）");
    await userEvent.clear(assertions);
    await userEvent.type(assertions, "not-json");

    expect(
      screen.getAllByText("断言必须是合法 JSON 数组").length,
    ).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();

    await userEvent.click(
      screen.getByRole("button", { name: /参数（JSON）/ }),
    );
    expect(screen.getByLabelText("参数（JSON）")).toHaveValue("123");
  });
});
