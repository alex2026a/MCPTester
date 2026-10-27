import { createRef } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SchemaForm, type SchemaFormHandle } from "./SchemaForm";
import { validateAgainstSchema } from "../lib/schema";

const schema = {
  type: "object",
  properties: {
    text: { type: "string", description: "要回显的文本" },
    count: { type: "number" },
    flag: { type: "boolean" },
    mode: { type: "string", enum: ["fast", "slow"] },
  },
  required: ["text"],
};

describe("SchemaForm", () => {
  it("按 Schema 生成基础控件", () => {
    render(<SchemaForm schema={schema} onChange={() => {}} />);
    expect(screen.getByLabelText("text")).toBeInTheDocument();
    expect(screen.getByLabelText("count")).toHaveAttribute("type", "number");
    expect(screen.getByLabelText("flag")).toHaveAttribute("type", "checkbox");
    expect(screen.getByLabelText("mode").tagName).toBe("SELECT");
  });

  it("未操作时不提示必填，提交后给出错误", async () => {
    const ref = createRef<SchemaFormHandle>();
    render(<SchemaForm ref={ref} schema={schema} onChange={() => {}} />);

    expect(screen.queryByText("缺少必填参数「text」")).not.toBeInTheDocument();

    let result: ReturnType<SchemaFormHandle["validate"]> | undefined;
    await act(async () => {
      result = ref.current?.validate();
    });

    expect(result?.errors).toEqual(["缺少必填参数「text」"]);
    expect(await screen.findByText("缺少必填参数「text」")).toBeInTheDocument();
  });

  it("输入后回调最新的参数值", async () => {
    const onChange = vi.fn();
    render(<SchemaForm schema={schema} onChange={onChange} />);
    await userEvent.type(screen.getByLabelText("text"), "hi");
    expect(onChange).toHaveBeenLastCalledWith({ text: "hi" });
  });

  it("JSON 模式可切换并解析参数", async () => {
    const onChange = vi.fn();
    render(<SchemaForm schema={schema} onChange={onChange} />);
    await userEvent.click(screen.getByRole("button", { name: "JSON 模式" }));
    const textarea = screen.getByLabelText("参数 JSON");
    await userEvent.clear(textarea);
    await userEvent.type(textarea, '{{"text": "json-mode"}');
    expect(onChange).toHaveBeenLastCalledWith({ text: "json-mode" });
  });

  it("JSON 模式按当前文本校验，解析失败时不回退旧参数", async () => {
    const ref = createRef<SchemaFormHandle>();
    render(<SchemaForm ref={ref} schema={schema} onChange={() => {}} />);

    await userEvent.click(screen.getByRole("button", { name: "JSON 模式" }));
    const textarea = screen.getByLabelText("参数 JSON");
    await userEvent.clear(textarea);
    await userEvent.type(textarea, '{{"text": "json-mode", "count": 2}');

    const ok = ref.current?.readValue();
    expect(ok?.parseError).toBeNull();
    expect(ok?.value).toEqual({ text: "json-mode", count: 2 });
    expect(ok?.errors).toEqual([]);

    await userEvent.clear(textarea);
    await userEvent.type(textarea, "{{");
    const broken = ref.current?.readValue();
    expect(broken?.parseError).toBeTruthy();
    expect(broken?.value).toEqual({});
  });

  it("无参数时展示空态", () => {
    render(
      <SchemaForm
        schema={{ type: "object", properties: {}, required: [] }}
        onChange={() => {}}
      />,
    );
    expect(screen.getByText("该接口无参数")).toBeInTheDocument();
  });

  it("必填字段显示标记，失焦后才出现红框", async () => {
    render(<SchemaForm schema={schema} onChange={() => {}} />);
    expect(screen.getAllByText("必填").length).toBeGreaterThan(0);

    const text = screen.getByLabelText("text");
    expect(text).toHaveAttribute("aria-required", "true");
    expect(text).not.toHaveClass("border-red-300");
    expect(screen.queryByText("此参数必填")).not.toBeInTheDocument();
    expect(screen.getByLabelText("count")).not.toHaveAttribute("aria-required");
    expect(screen.getByLabelText("count")).not.toHaveClass("border-red-300");

    await userEvent.click(text);
    await userEvent.tab();

    expect(text).toHaveClass("border-red-300");
    expect(text).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText("此参数必填")).toBeInTheDocument();
  });

  it("兼容字段级 required: true", () => {
    const ref = createRef<SchemaFormHandle>();
    render(
      <SchemaForm
        ref={ref}
        schema={{
          type: "object",
          properties: { query: { type: "string", required: true } },
        }}
        onChange={() => {}}
      />,
    );
    expect(screen.getByLabelText("query")).toHaveAttribute(
      "aria-required",
      "true",
    );
    expect(screen.queryByText("缺少必填参数「query」")).not.toBeInTheDocument();

    act(() => {
      ref.current?.validate();
    });
    expect(screen.getByText("缺少必填参数「query」")).toBeInTheDocument();
  });

  it("apply 可回填表单与 JSON 模式，clear 清空并通知", async () => {
    const ref = createRef<SchemaFormHandle>();
    const onChange = vi.fn();
    render(<SchemaForm ref={ref} schema={schema} onChange={onChange} />);

    act(() => ref.current?.apply({ text: "loaded", count: 2 }));
    expect(screen.getByLabelText("text")).toHaveValue("loaded");
    expect(screen.getByLabelText("count")).toHaveValue(2);
    expect(onChange).toHaveBeenLastCalledWith({ text: "loaded", count: 2 });

    await userEvent.click(screen.getByRole("button", { name: "JSON 模式" }));
    act(() => ref.current?.apply({ text: "json", count: 3 }));
    expect(screen.getByLabelText("参数 JSON")).toHaveValue(
      JSON.stringify({ text: "json", count: 3 }, null, 2),
    );

    act(() => ref.current?.clear());
    expect(screen.getByLabelText("参数 JSON")).toHaveValue("{}");
    expect(onChange).toHaveBeenLastCalledWith({});
  });

  it("snapshot/restore 可恢复 JSON 非法文本，并只触发一次 onChange", async () => {
    const ref = createRef<SchemaFormHandle>();
    const onChange = vi.fn();
    render(<SchemaForm ref={ref} schema={schema} onChange={onChange} />);

    await userEvent.click(screen.getByRole("button", { name: "JSON 模式" }));
    const textarea = screen.getByLabelText("参数 JSON");
    fireEvent.change(textarea, { target: { value: "{{" } });

    let snapshot: ReturnType<SchemaFormHandle["snapshot"]> | undefined;
    act(() => {
      snapshot = ref.current?.snapshot();
    });
    expect(snapshot?.jsonMode).toBe(true);
    expect(snapshot?.jsonText).toBe("{{");

    onChange.mockClear();
    act(() => {
      if (snapshot) ref.current?.restore(snapshot);
    });
    expect(screen.getByLabelText("参数 JSON")).toHaveValue("{{");
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it("清空按钮清空当前参数", async () => {
    const onChange = vi.fn();
    render(<SchemaForm schema={schema} onChange={onChange} />);
    await userEvent.type(screen.getByLabelText("text"), "hello");

    await userEvent.click(screen.getByRole("button", { name: "清空" }));
    expect(screen.getByLabelText("text")).toHaveValue("");
    expect(onChange).toHaveBeenLastCalledWith({});
  });

  it("JSON 非法输入也触发 onDirty", async () => {
    const onDirty = vi.fn();
    render(<SchemaForm schema={schema} onChange={() => {}} onDirty={onDirty} />);

    await userEvent.click(screen.getByRole("button", { name: "JSON 模式" }));
    const textarea = screen.getByLabelText("参数 JSON");
    await userEvent.clear(textarea);
    await userEvent.type(textarea, "{{");
    expect(onDirty).toHaveBeenCalled();
  });

  it("校验函数覆盖必填/类型/枚举", () => {
    expect(validateAgainstSchema(schema, { text: "x" })).toEqual([]);
    expect(validateAgainstSchema(schema, { text: "x", count: "no" })).toEqual([
      "参数「count」应为数字",
    ]);
    expect(validateAgainstSchema(schema, { text: "x", mode: "other" })).toEqual(
      ["参数「mode」必须是以下值之一：fast / slow"],
    );
  });
});
