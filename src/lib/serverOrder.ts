import type { ServerSummary } from "../types/models";

/** 服务器选择列表的状态分组键 */
export type ServerGroupKey =
  "connected" | "connecting" | "disconnected" | "error" | "disabled";

export interface ServerGroup {
  key: ServerGroupKey;
  label: string;
  servers: ServerSummary[];
}

export interface ServerStatusMeta {
  group: ServerGroupKey;
  /** 选项文本前缀（单色字形，避免依赖 emoji 颜色） */
  glyph: string;
  /** 中文状态名 */
  label: string;
  /** 分组标题已表达状态时仍需补充的细节（如“降级”） */
  suffix: string | null;
}

const GROUP_LABEL: Record<ServerGroupKey, string> = {
  connected: "已连接",
  connecting: "连接中",
  disconnected: "未连接",
  error: "异常",
  disabled: "已停用",
};

const GROUP_ORDER: ServerGroupKey[] = [
  "connected",
  "connecting",
  "disconnected",
  "error",
  "disabled",
];

/** 服务器所属状态分组（已停用优先于连接状态） */
export function serverGroupKey(server: ServerSummary): ServerGroupKey {
  if (!server.enabled) return "disabled";
  switch (server.state.state) {
    case "ready":
    case "degraded":
      return "connected";
    case "connecting":
      return "connecting";
    case "disconnected":
      return "disconnected";
    case "lost":
    case "error":
      return "error";
    default:
      return "error";
  }
}

/** 选项展示元信息 */
export function serverStatusMeta(server: ServerSummary): ServerStatusMeta {
  if (!server.enabled) {
    return {
      group: "disabled",
      glyph: "×",
      label: "已停用",
      suffix: "已停用",
    };
  }
  switch (server.state.state) {
    case "ready":
      return { group: "connected", glyph: "●", label: "已连接", suffix: null };
    case "degraded":
      return { group: "connected", glyph: "◐", label: "降级", suffix: "降级" };
    case "connecting":
      return { group: "connecting", glyph: "◌", label: "连接中", suffix: null };
    case "disconnected":
      return {
        group: "disconnected",
        glyph: "○",
        label: "未连接",
        suffix: null,
      };
    case "lost":
      return { group: "error", glyph: "!", label: "掉线", suffix: "掉线" };
    case "error":
      return { group: "error", glyph: "!", label: "错误", suffix: "错误" };
    default:
      return { group: "error", glyph: "!", label: "错误", suffix: "错误" };
  }
}

/**
 * 选项文本：状态字形前置。
 *
 * 分组模式下组标题已经表达大类状态，只有需要补充细节时才追加括号；
 * 不分组模式（如历史筛选）追加完整中文状态。
 */
export function serverOptionLabel(
  server: ServerSummary,
  grouped: boolean,
): string {
  const meta = serverStatusMeta(server);
  const suffix = meta.suffix ?? (grouped ? "" : meta.label);
  return `${meta.glyph} ${server.name}${suffix ? `（${suffix}）` : ""}`;
}

/** 按状态分组；组内保持传入顺序（即后端的 sort_order） */
export function groupServers(servers: ServerSummary[]): ServerGroup[] {
  return GROUP_ORDER.map((key) => ({
    key,
    label: GROUP_LABEL[key],
    servers: servers.filter((server) => serverGroupKey(server) === key),
  })).filter((group) => group.servers.length > 0);
}

/** 默认服务器：显式首选 > 第一个可用（ready）> 第一个 */
export function pickDefaultServerId(
  servers: ServerSummary[],
  preferredId?: string | null,
): string {
  if (preferredId && servers.some((server) => server.id === preferredId)) {
    return preferredId;
  }
  const ready = servers.find(
    (server) => server.enabled && server.state.state === "ready",
  );
  return ready?.id ?? servers[0]?.id ?? "";
}

/** 移动数组元素；`to` 是移除源元素后的目标下标 */
export function moveItem<T>(items: T[], from: number, to: number): T[] {
  if (
    from === to ||
    from < 0 ||
    from >= items.length ||
    to < 0 ||
    to >= items.length
  ) {
    return items;
  }
  const next = [...items];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}
