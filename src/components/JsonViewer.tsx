/**
 * 兼容壳（v0.1.3）：统一收敛到 MessageViewer。
 *
 * 旧调用点（工具 Schema、历史参数、导出预览、服务器配置 JSON）保持不变，
 * 自动获得搜索 / 折叠 / 复制 / 全屏能力。
 */

import { MessageViewer } from "./json/MessageViewer";

export type JsonViewerMode = "tree" | "text";

export function JsonViewer({
  value,
  text,
  defaultMode = "tree",
  showToolbar = true,
  copyLabel,
  contentClassName = "max-h-80 overflow-auto",
}: {
  /** 结构化值（与 text 二选一） */
  value?: unknown;
  /** 预序列化的 JSON 文本（优先于 value） */
  text?: string;
  defaultMode?: JsonViewerMode;
  showToolbar?: boolean;
  /** 工具栏复制按钮文案；默认“复制全部” */
  copyLabel?: string;
  /** 内容区样式（高度/滚动） */
  contentClassName?: string;
}) {
  return (
    <MessageViewer
      value={value}
      text={text}
      defaultTab={defaultMode === "text" ? "raw" : "tree"}
      tabs={["tree", "raw"]}
      showToolbar={showToolbar}
      copyLabel={copyLabel}
      contentClassName={contentClassName}
    />
  );
}
