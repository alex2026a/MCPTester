import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ServerSelect } from "./ServerSelect";
import type { ConnectionState, ServerSummary } from "../types/models";

const ready: ConnectionState = {
  state: "ready",
  handshake: {
    server_name: "mock",
    server_version: "1.0.0",
    protocol_version: "2025-06-18",
    lifecycle: "legacy",
    capabilities: {},
    instructions: null,
  },
};

function server(
  id: string,
  name: string,
  state: ConnectionState,
  enabled = true,
): ServerSummary {
  return {
    id,
    name,
    transport: "stdio",
    enabled,
    tags: [],
    last_connected_at: null,
    state,
  };
}

describe("ServerSelect", () => {
  it("已连接分组在前，组内保持后端顺序", () => {
    const servers = [
      server("d1", "Disconnected", { state: "disconnected" }),
      server("r1", "Ready One", ready),
      server("r2", "Ready Two", ready),
    ];
    render(<ServerSelect servers={servers} value="r1" onChange={vi.fn()} />);

    const select = screen.getByRole("combobox", { name: "选择服务器" });
    const groups = select.querySelectorAll("optgroup");
    expect(groups).toHaveLength(2);
    expect(groups[0]).toHaveAttribute("label", "已连接（2）");
    expect(groups[1]).toHaveAttribute("label", "未连接（1）");

    const options = groups[0].querySelectorAll("option");
    expect(options).toHaveLength(2);
    expect(options[0]).toHaveTextContent("● Ready One");
    expect(options[1]).toHaveTextContent("● Ready Two");
  });

  it("支持空选项、不分组状态文本和停用选项", async () => {
    const onChange = vi.fn();
    const servers = [
      server("d1", "Disconnected", { state: "disconnected" }),
      server("x1", "Disabled", { state: "disconnected" }, false),
    ];
    render(
      <ServerSelect
        servers={servers}
        value=""
        onChange={onChange}
        placeholder="全部服务器"
        groupByStatus={false}
        ariaLabel="按服务器筛选"
      />,
    );

    const select = screen.getByRole("combobox", { name: "按服务器筛选" });
    expect(select.querySelectorAll("optgroup")).toHaveLength(0);
    expect(
      screen.getByRole("option", { name: "全部服务器" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: "○ Disconnected（未连接）" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: "× Disabled（已停用）" }),
    ).toBeDisabled();

    await userEvent.selectOptions(select, "d1");
    expect(onChange).toHaveBeenCalledWith("d1");
  });

  it("allowDisabled 时历史筛选仍可选择已停用服务器", () => {
    render(
      <ServerSelect
        servers={[server("x1", "Disabled", { state: "disconnected" }, false)]}
        value=""
        onChange={vi.fn()}
        placeholder="全部服务器"
        groupByStatus={false}
        allowDisabled
        ariaLabel="按服务器筛选"
      />,
    );

    expect(
      screen.getByRole("option", { name: "× Disabled（已停用）" }),
    ).not.toBeDisabled();
  });
});
