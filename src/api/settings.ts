import { invoke } from "@tauri-apps/api/core";

export interface Settings {
  request_timeout_secs: number;
  connect_timeout_secs: number;
  history_retention_days: number;
  history_max_rows: number;
  stderr_buffer_lines: number;
  notification_buffer_lines: number;
  auto_reconnect: boolean;
  auto_refresh_on_list_changed: boolean;
}

export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export function updateSettings(settings: Settings): Promise<Settings> {
  return invoke<Settings>("update_settings", { settings });
}
