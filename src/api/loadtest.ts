import { invoke } from "@tauri-apps/api/core";
import type { Paged } from "../types/testing";
import type {
  ActiveLoadRun,
  LoadPreview,
  LoadProfile,
  LoadRun,
  LoadRunSummary,
} from "../types/loadtest";

export function previewLoadTest(
  serverId: string,
  suiteId: string,
  profile: LoadProfile,
): Promise<LoadPreview> {
  return invoke<LoadPreview>("preview_load_test", {
    serverId,
    suiteId,
    profile,
  });
}

export function startLoadTest(
  serverId: string,
  suiteId: string,
  profile: LoadProfile,
  confirmProduction: boolean,
): Promise<string> {
  return invoke<string>("start_load_test", {
    serverId,
    suiteId,
    profile,
    confirmProduction,
  });
}

export function cancelLoadTest(runId: string): Promise<boolean> {
  return invoke<boolean>("cancel_load_test", { runId });
}

export function getActiveLoadTest(): Promise<ActiveLoadRun | null> {
  return invoke<ActiveLoadRun | null>("get_active_load_test");
}

export function listLoadRuns(
  serverId: string | null,
  offset: number,
  limit: number,
): Promise<Paged<LoadRunSummary>> {
  return invoke<Paged<LoadRunSummary>>("list_load_runs", {
    serverId,
    offset,
    limit,
  });
}

export function getLoadRun(runId: string): Promise<LoadRun> {
  return invoke<LoadRun>("get_load_run", { runId });
}

export function exportLoadRun(runId: string, path?: string): Promise<string> {
  return invoke<string>("export_load_run", {
    runId,
    path: path ?? null,
  });
}
