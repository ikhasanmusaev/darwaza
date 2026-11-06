import { invoke } from "@tauri-apps/api/core";

// ────────────────────────────────────────────────────────────────────
// ssh_config (#018)
// ────────────────────────────────────────────────────────────────────

export interface SshConfigEntry {
  alias: string;
  hostname: string | null;
  user: string | null;
  port: number | null;
  identityFile: string | null;
  proxyJump: string | null;
}

export interface SshConfigPreview {
  path: string;
  entries: SshConfigEntry[];
}

// ────────────────────────────────────────────────────────────────────
// SSH sessions (#020-#02B + events)
// ────────────────────────────────────────────────────────────────────

/** Статусы — точно по `SessionStatus` enum в Rust (SCREAMING_SNAKE_CASE). */
export type SessionStatus =
  | "CONNECTING"
  | "AUTHENTICATING"
  | "PTY_OPENING"
  | "CONNECTED"
  | "DISCONNECTING"
  | "CLOSED"
  | "FAILED";

export interface ConnectResponse {
  sessionId: string;
}

export interface SessionSnapshot {
  sessionId: string;
  hostId: number;
  status: SessionStatus;
}

// Payloads событий от backend (см. `SessionEvent` в src-tauri/src/ssh/session.rs)

export interface SessionStatusPayload {
  event: "status";
  sessionId: string;
  status: SessionStatus;
  error?: string;
}

export interface SessionDataPayload {
  event: "data";
  sessionId: string;
  /** Сейчас сериализуется как number[] (Vec<u8> в Rust). TODO: оптимизировать через base64 при необходимости. */
  chunk: number[];
}

export interface SessionClosedPayload {
  event: "closed";
  sessionId: string;
  exitCode?: number;
}

export const sshApi = {
  configPreview: () => invoke<SshConfigPreview>("ssh_config_preview"),

  connect: (hostId: number) =>
    invoke<ConnectResponse>("ssh_connect", { hostId }),

  send: (sessionId: string, data: Uint8Array) =>
    invoke<void>("ssh_send", { sessionId, data: Array.from(data) }),

  resize: (sessionId: string, cols: number, rows: number) =>
    invoke<void>("ssh_resize", { sessionId, cols, rows }),

  /**
   * Задержка до сервера, мс (#229). Спрашивать, пока вкладка на виду, раз в
   * несколько секунд; ошибка — «не удалось измерить», а не обрыв связи.
   */
  latency: (sessionId: string) => invoke<number>("ssh_latency", { sessionId }),

  disconnect: (sessionId: string) =>
    invoke<void>("ssh_disconnect", { sessionId }),

  activeSessions: () => invoke<SessionSnapshot[]>("ssh_active_sessions"),
};
