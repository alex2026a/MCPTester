import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { save } from "@tauri-apps/plugin-dialog";
import { ExportPreviewDialog } from "./ExportPreviewDialog";

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
  revealItemInDir: vi.fn(),
}));

const saveMock = vi.mocked(save);

describe("ExportPreviewDialog（v0.6.1）", () => {
  it("自定义 filters 传给保存对话框", async () => {
    saveMock.mockResolvedValue("C:\\tmp\\out.csv");
    const onSave = vi.fn().mockResolvedValue("C:\\tmp\\out.csv");
    const onClose = vi.fn();

    render(
      <ExportPreviewDialog
        title="导出接口清单"
        defaultFilename="out.csv"
        load={() => Promise.resolve("a,b\r\n1,2\r\n")}
        onSave={onSave}
        onClose={onClose}
        filters={[{ name: "CSV", extensions: ["csv"] }]}
      />,
    );

    await userEvent.click(
      await screen.findByRole("button", { name: "保存到…" }),
    );
    await waitFor(() =>
      expect(saveMock).toHaveBeenCalledWith(
        expect.objectContaining({
          filters: [{ name: "CSV", extensions: ["csv"] }],
          defaultPath: "out.csv",
        }),
      ),
    );
    expect(onSave).toHaveBeenCalledWith("C:\\tmp\\out.csv");
    expect(onClose).toHaveBeenCalled();
  });

  it("超长预览截断并提示保存仍完整", async () => {
    render(
      <ExportPreviewDialog
        title="预览"
        defaultFilename="out.txt"
        previewLimit={10}
        load={() => Promise.resolve("x".repeat(20))}
        onSave={() => Promise.resolve("")}
        onClose={() => {}}
      />,
    );

    expect(await screen.findByText(/预览已截断/)).toBeInTheDocument();
  });
});
