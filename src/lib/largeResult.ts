/** 大结果读取保护（v0.4.2） */

import { formatBytes } from "../components/json/format";
import type { CallRecord } from "../types/models";

/** 超过该大小的截断结果，读取全文前需要确认 */
export const LARGE_COPY_BYTES = 10 * 1024 * 1024;

/**
 * 读取完整结果前的确认。
 *
 * 仅对已截断且超过阈值的记录弹窗；返回 false 时调用方必须放弃读取，
 * 不能回退到截断的 `record.result`。
 */
export function confirmLargeResultRead(
  record: Pick<CallRecord, "truncated" | "size_bytes">,
): boolean {
  if (!record.truncated || record.size_bytes < LARGE_COPY_BYTES) return true;
  return window.confirm(
    `完整结果约 ${formatBytes(record.size_bytes)}，读取可能需要几秒，是否继续？`,
  );
}
