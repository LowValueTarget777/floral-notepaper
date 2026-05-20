import { invoke } from "@tauri-apps/api/core";
import type { SyncStatus } from "./types";

export function getSyncStatus(): Promise<SyncStatus> {
  return invoke("sync_status");
}

export function syncNow(): Promise<SyncStatus> {
  return invoke("sync_now");
}

export function testSyncConnection(): Promise<SyncStatus> {
  return invoke("sync_test_connection");
}
