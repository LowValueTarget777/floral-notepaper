import { invoke } from "@tauri-apps/api/core";
import type { SyncStatus } from "./types";

function normalizeSyncStatus(value: unknown): SyncStatus {
  const status = value as {
    enabled?: boolean;
    configured?: boolean;
    lastRevision?: number | string;
    lastSyncAt?: string | null;
    lastError?: string | null;
  };

  return {
    enabled: Boolean(status.enabled),
    configured: Boolean(status.configured),
    lastRevision: String(status.lastRevision ?? "0"),
    lastSyncAt: status.lastSyncAt ?? null,
    lastError: status.lastError ?? null,
  };
}

export async function getSyncStatus(): Promise<SyncStatus> {
  return normalizeSyncStatus(await invoke("sync_status"));
}

export async function syncNow(): Promise<SyncStatus> {
  return normalizeSyncStatus(await invoke("sync_now"));
}

export async function testSyncConnection(): Promise<SyncStatus> {
  return normalizeSyncStatus(await invoke("sync_test_connection"));
}
