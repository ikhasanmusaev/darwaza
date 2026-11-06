import { invoke } from "@tauri-apps/api/core";

export type PortForwardKind = "local" | "remote" | "dynamic";

export interface PortForward {
  id: number;
  hostId: number;
  kind: PortForwardKind;
  label: string | null;
  /** Для local/dynamic — где слушаем; для remote — куда отдаём на нашей стороне. */
  localHost: string;
  localPort: number;
  /** Для local — куда идти со стороны сервера; для dynamic не используется. */
  remoteHost: string | null;
  remotePort: number | null;
  autoStart: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface PortForwardInput {
  hostId: number;
  kind: PortForwardKind;
  label?: string | null;
  localHost?: string | null;
  localPort: number;
  remoteHost?: string | null;
  remotePort?: number | null;
  autoStart?: boolean | null;
}

/** Активный проброс: правило + фактический адрес прослушки. */
export interface ForwardSnapshot {
  forwardId: number;
  sessionId: string;
  bound: string;
}

export const portForwardsApi = {
  list: (hostId?: number) =>
    invoke<PortForward[]>("port_forwards_list", { hostId: hostId ?? null }),
  create: (data: PortForwardInput) =>
    invoke<PortForward>("port_forwards_create", { data }),
  update: (id: number, data: PortForwardInput) =>
    invoke<PortForward>("port_forwards_update", { id, data }),
  remove: (id: number) => invoke<void>("port_forwards_remove", { id }),

  /** Возвращает фактический адрес прослушки (при localPort=0 порт выдаёт ОС). */
  start: (id: number, sessionId: string) =>
    invoke<string>("port_forward_start", { id, sessionId }),
  stop: (id: number) => invoke<boolean>("port_forward_stop", { id }),
  active: () => invoke<ForwardSnapshot[]>("port_forwards_active"),
};
