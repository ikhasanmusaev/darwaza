import { invoke } from "@tauri-apps/api/core";


export interface Identity {
  id: number;
  label: string;
  kind: "password" | "private_key" | "agent";
  keyPath: string | null;
  keyringRef: string | null;
  hasPassphrase: boolean;
  hasTotp: boolean;
  publicKey: string | null;
  createdAt: string;
  updatedAt: string;
}

/** Запись списка: ключ и число хостов, которые им пользуются (#236). */
export interface IdentityListItem extends Identity {
  /**
   * Удаление ключа молча отвязывает эти хосты (`identity_id` обнуляется), так
   * что число нужно показать до подтверждения. Имена хостов —
   * `identitiesApi.usedBy`.
   */
  usedBy: number;
}

export interface IdentityInput {
  label: string;
  kind: "password" | "private_key" | "agent";
  keyPath?: string | null;
  keyringRef?: string | null;
  hasPassphrase?: boolean;
  publicKey?: string | null;
}

export const identitiesApi = {
  list: () => invoke<IdentityListItem[]>("identities_list"),
  /** Имена хостов, которые останутся без входа, если удалить ключ. */
  usedBy: (id: number) => invoke<string[]>("identities_used_by", { id }),
  get: (id: number) => invoke<Identity>("identities_get", { id }),
  create: (data: IdentityInput) => invoke<Identity>("identities_create", { data }),
  update: (id: number, data: IdentityInput) =>
    invoke<Identity>("identities_update", { id, data }),
  remove: (id: number) => invoke<void>("identities_remove", { id }),

  /** Сохранить пароль в keyring для identity. Чтение секрета доступно
   *  только из Rust-backend (resolve_auth) — намеренно не выставлено в UI. */
  setPassword: (id: number, password: string) =>
    invoke<void>("identity_set_password", { id, password }),

  /** Сохранить passphrase для зашифрованного приватного ключа.
   *  Чтение секрета — только из Rust-backend. */
  setPassphrase: (id: number, passphrase: string) =>
    invoke<void>("identity_set_passphrase", { id, passphrase }),
};
