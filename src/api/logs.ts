import { invoke } from "@tauri-apps/api/core";

export interface AgentLogEntry {
  id: number;
  at: string;
  host: string | null;
  action: string;
  detail: string | null;
  outcome: string;
  exitCode: number | null;
  note: string | null;
}

export const logsApi = {
  dir: () => invoke<string>("logs_dir"),
  openDir: () => invoke<void>("logs_open_dir"),
  agentLog: (limit?: number) => invoke<AgentLogEntry[]>("agent_log_list", { limit }),
};
