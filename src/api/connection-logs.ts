import { invoke } from "@tauri-apps/api/core";

export interface ConnectionLogEntry {
  id: number;
  hostId: number;
  sessionId: string;
  startedAt: string;
  endedAt: string | null;
  status: string;
  error: string | null;
}

export const connectionLogsApi = {
  list: (hostId?: number, limit?: number) =>
    invoke<ConnectionLogEntry[]>("connection_logs_list", { hostId, limit }),
};
