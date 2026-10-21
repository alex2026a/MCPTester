import { invoke } from "@tauri-apps/api/core";
import type { AppInfo } from "../types/models";

/** 获取应用信息（版本、数据目录、日志目录、启动配置、协议支持清单） */
export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("get_app_info");
}

/** 调起系统文件管理器打开实际数据目录 */
export function openDataDir(): Promise<void> {
  return invoke<void>("open_data_dir");
}

/** 调起系统文件管理器打开启动配置文件所在目录 */
export function openConfigDir(): Promise<void> {
  return invoke<void>("open_config_dir");
}

export interface SeedIssue {
  key: string;
  reason: string;
}

export interface SeedReport {
  status: "not_found" | "applied" | "partial" | "error" | string;
  path: string | null;
  added: number;
  skipped: number;
  failed: SeedIssue[];
  default_server_id: string | null;
  auto_connect: boolean;
}

/** 手动重新应用启动配置（只补缺失项，不覆盖已有服务器） */
export function reloadBootstrapConfig(): Promise<SeedReport> {
  return invoke<SeedReport>("reload_bootstrap_config");
}
