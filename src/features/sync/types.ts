export interface SyncStatus {
  enabled: boolean;
  configured: boolean;
  lastRevision: number;
  lastSyncAt: string | null;
  lastError: string | null;
}
