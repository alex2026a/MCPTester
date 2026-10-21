import { invoke } from "@tauri-apps/api/core";
import type {
  CapabilitySnapshot,
  ConnectionState,
  Handshake,
  ProbeResult,
  ServerConfig,
  ServerDraft,
  ServerSummary,
} from "../types/models";

/** 服务器列表（含运行期连接状态） */
export function listServers(): Promise<ServerSummary[]> {
  return invoke<ServerSummary[]>("list_servers");
}

/** 服务器详情（编辑用） */
export function getServer(id: string): Promise<ServerConfig> {
  return invoke<ServerConfig>("get_server", { id });
}

/** 新增服务器 */
export function addServer(draft: ServerDraft): Promise<ServerConfig> {
  return invoke<ServerConfig>("add_server", { draft });
}

/** 更新服务器（敏感值 keep/replace/clear） */
export function updateServer(id: string, draft: ServerDraft): Promise<ServerConfig> {
  return invoke<ServerConfig>("update_server", { id, draft });
}

/** 更新名称/启用状态 */
export function updateServerMeta(
  id: string,
  name?: string,
  enabled?: boolean,
): Promise<ServerConfig> {
  return invoke<ServerConfig>("update_server_meta", { id, name, enabled });
}

/** 删除服务器 */
export function deleteServer(id: string): Promise<void> {
  return invoke<void>("delete_server", { id });
}

/** 连接（失败时 reject 结构化错误 `{ kind, message, raw }`，用 toErrorInfo 归一） */
export function connectServer(id: string): Promise<ConnectionState> {
  return invoke<ConnectionState>("connect_server", { id });
}

/** 断开 */
export function disconnectServer(id: string): Promise<void> {
  return invoke<void>("disconnect_server", { id });
}

/** 重连（失败时 reject 结构化错误） */
export function reconnectServer(id: string): Promise<ConnectionState> {
  return invoke<ConnectionState>("reconnect_server", { id });
}

/** 健康探测 */
export function probeServer(id: string): Promise<ProbeResult> {
  return invoke<ProbeResult>("probe_server", { id });
}

/** 能力快照（缓存优先） */
export function getCapabilities(id: string): Promise<CapabilitySnapshot> {
  return invoke<CapabilitySnapshot>("get_capabilities", { id });
}

/** 强制刷新能力快照 */
export function refreshCapabilities(id: string): Promise<CapabilitySnapshot> {
  return invoke<CapabilitySnapshot>("refresh_capabilities", { id });
}

export interface ImportIssue {
  name: string;
  reason: string;
}

export interface ImportReport {
  added: number;
  updated: number;
  skipped: ImportIssue[];
  failed: ImportIssue[];
}

/** 导入 mcpServers / servers 配置（JSON 文本） */
export function importServers(json: string): Promise<ImportReport> {
  return invoke<ImportReport>("import_servers", { json });
}

/** 从文件导入（v0.1.1：原生文件选择后传路径） */
export function importServersFromFile(path: string): Promise<ImportReport> {
  return invoke<ImportReport>("import_servers_from_file", { path });
}

/** 导出配置（format: claude | vscode；返回文件路径） */
export function exportServers(format: "claude" | "vscode", path?: string): Promise<string> {
  return invoke<string>("export_servers", { format, path: path ?? null });
}

/** 导出预览（v0.1.1：返回 JSON 文本，不落盘） */
export function previewExportServers(format: "claude" | "vscode"): Promise<string> {
  return invoke<string>("preview_export_servers", { format });
}

/** 单服务器配置 JSON（Claude 格式，敏感值为占位符） */
export function getServerJson(id: string): Promise<string> {
  return invoke<string>("get_server_json", { id });
}

/** 测试连接步骤 */
export interface TestStep {
  name: string;
  ok: boolean;
  detail: string;
  elapsed_ms: number;
}

/** 测试连接报告（v0.1.1；支持未保存草稿） */
export interface ConnectionTestReport {
  ok: boolean;
  steps: TestStep[];
  handshake: Handshake | null;
  tool_count: number;
  resource_count: number;
  template_count: number;
  prompt_count: number;
  error: { kind: string; message?: string; raw?: string | null } | null;
}

/** 测试连接（不落库；id 用于编辑已有服务器时从 keyring 解析 keep 敏感值） */
export function testServerConnection(
  draft: ServerDraft,
  id?: string | null,
): Promise<ConnectionTestReport> {
  return invoke<ConnectionTestReport>("test_server_connection", { draft, id: id ?? null });
}
