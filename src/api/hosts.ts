import { invoke } from "@tauri-apps/api/core";

export interface Host {
  id: number;
  groupId: number | null;
  name: string;
  hostname: string;
  port: number;
  username: string;
  identityId: number | null;
  authMethod: "password" | "publickey" | "agent";
  color: string | null;
  icon: string | null;
  jumpHostId: number | null;
  /** Пробрасывать ли ssh-agent на удалённый хост (#150a). */
  forwardAgent: boolean;
  /** Разрешён ли доступ агента к этому хосту (#201). */
  agentAccess: boolean;
  /** Читающие команды с sudo, разрешённые агенту (#214). По одной в строке. */
  agentSudoRead: string | null;
  /** Заметки помощника о сервере (#217). Пишет агент, читает человек. */
  agentNotes: string | null;
  keepAliveInterval: number;
  description: string | null;
  lastConnectedAt: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface HostInput {
  groupId?: number | null;
  name: string;
  hostname: string;
  port: number;
  username: string;
  identityId?: number | null;
  authMethod: "password" | "publickey" | "agent";
  color?: string | null;
  icon?: string | null;
  jumpHostId?: number | null;
  forwardAgent?: boolean | null;
  /** Разрешён ли доступ агента к этому хосту (#201). */
  agentAccess?: boolean | null;
  agentSudoRead?: string | null;
  agentNotes?: string | null;
  keepAliveInterval?: number;
  description?: string | null;
  password?: string | null;
}

/**
 * Доступность хоста (#228). `unknown` — НЕ «не в сети»: не проверяли (проба
 * выключена, хост за промежуточным, адрес не разобрать). Рисовать отдельно.
 */
export type Reachability = "reachable" | "unreachable" | "unknown";

export interface HostReachability {
  hostId: number;
  state: Reachability;
}

export const hostsApi = {
  /**
   * Проверить доступность хостов, которые сейчас на виду. Проба включается
   * настройкой `hosts.probe = "1"` и по умолчанию выключена: пока она
   * выключена, Rust не делает ни одного сетевого вызова и отвечает `unknown`.
   */
  probe: (hostIds: number[]) =>
    invoke<HostReachability[]>("hosts_probe", { hostIds }),

  list: () => invoke<Host[]>("hosts_list"),
  get: (id: number) => invoke<Host>("hosts_get", { id }),
  create: (data: HostInput) => invoke<Host>("hosts_create", { data }),
  update: (id: number, data: HostInput) =>
    invoke<Host>("hosts_update", { id, data }),
  remove: (id: number) => invoke<void>("hosts_remove", { id }),
};
