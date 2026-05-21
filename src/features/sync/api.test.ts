import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { getSyncStatus, syncNow, testSyncConnection } from "./api";
import type { SyncStatus } from "./types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const mockedInvoke = vi.mocked(invoke);

describe("sync api", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  test("gets sync status through Rust", async () => {
    const status = {
      enabled: true,
      configured: true,
      lastRevision: 12,
      lastSyncAt: "2026-05-18T08:00:00Z",
      lastError: null,
    };
    mockedInvoke.mockResolvedValue(status);

    const expected: SyncStatus = {
      enabled: true,
      configured: true,
      lastRevision: "12",
      lastSyncAt: "2026-05-18T08:00:00Z",
      lastError: null,
    };

    await expect(getSyncStatus()).resolves.toEqual(expected);

    expect(invoke).toHaveBeenCalledWith("sync_status");
  });

  test("runs manual sync through Rust", async () => {
    mockedInvoke.mockResolvedValue({
      enabled: false,
      configured: true,
      lastRevision: 13,
      lastSyncAt: "2026-05-18T08:01:00Z",
      lastError: null,
    });

    await expect(syncNow()).resolves.toEqual({
      enabled: false,
      configured: true,
      lastRevision: "13",
      lastSyncAt: "2026-05-18T08:01:00Z",
      lastError: null,
    });

    expect(invoke).toHaveBeenCalledWith("sync_now");
  });

  test("tests the configured server connection through Rust", async () => {
    mockedInvoke.mockResolvedValue({
      enabled: false,
      configured: true,
      lastRevision: 0,
      lastSyncAt: null,
      lastError: null,
    });

    await expect(testSyncConnection()).resolves.toEqual({
      enabled: false,
      configured: true,
      lastRevision: "0",
      lastSyncAt: null,
      lastError: null,
    });

    expect(invoke).toHaveBeenCalledWith("sync_test_connection");
  });
});
