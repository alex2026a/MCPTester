import { invoke } from "@tauri-apps/api/core";
import type { CallRecord, CallRecordSummary } from "../types/models";

export interface CallFilterInput {
  server_id?: string | null;
  method?: string | null;
  status?: string | null;
  keyword?: string | null;
  keyword_full?: boolean;
  from?: string | null;
  to?: string | null;
}

export interface PagedCallRecords {
  items: CallRecordSummary[];
  total: number;
  offset: number;
  limit: number;
}

/** 历史列表（筛选 + 分页） */
export function listCallRecords(
  filter: CallFilterInput,
  offset: number,
  limit: number,
): Promise<PagedCallRecords> {
  return invoke<PagedCallRecords>("list_call_records", { filter, offset, limit });
}

/** 历史详情（完整参数与结果） */
export function getCallRecord(recordId: string): Promise<CallRecord> {
  return invoke<CallRecord>("get_call_record", { recordId });
}

/** 大结果全文路径（v0.1.3；未落盘时为 null） */
export function getFullResultPath(recordId: string): Promise<string | null> {
  return invoke<string | null>("get_full_result_path", { recordId });
}

/** 读取大结果全文（v0.1.3） */
export function readFullResult(recordId: string): Promise<string> {
  return invoke<string>("read_full_result", { recordId });
}

/** 删除记录（同步清理落盘文件） */
export function deleteCallRecords(ids: string[]): Promise<number> {
  return invoke<number>("delete_call_records", { ids });
}

/** 清空（按筛选条件） */
export function clearCallRecords(filter: CallFilterInput): Promise<number> {
  return invoke<number>("clear_call_records", { filter });
}

/** 导出（返回写入路径） */
export function exportCallRecords(
  filter: CallFilterInput,
  path?: string,
): Promise<string> {
  return invoke<string>("export_call_records", { filter, path: path ?? null });
}

/** 导出预览（v0.1.1：返回 JSON 文本与条数，不落盘） */
export function previewExportCallRecords(
  filter: CallFilterInput,
): Promise<{ json: string; count: number }> {
  return invoke<{ json: string; count: number }>("preview_export_call_records", {
    filter,
  });
}
