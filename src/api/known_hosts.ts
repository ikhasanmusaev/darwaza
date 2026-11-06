import { invoke } from "@tauri-apps/api/core";

export interface KnownHost {
  id: number;
  hostname: string;
  keyType: string;
  fingerprint: string;
  acceptedAt: string;
}

export const knownHostsApi = {
  list: () => invoke<KnownHost[]>("known_hosts_list"),
  remove: (id: number) => invoke<void>("known_hosts_remove", { id }),
  clear: () => invoke<void>("known_hosts_clear"),

  /** Подтвердить fingerprint, разблокировать handshake. */
  acceptHostKey: (sessionId: string) =>
    invoke<void>("ssh_accept_host_key", { sessionId }),

  /** Отвергнуть fingerprint — сессия упадёт в FAILED. */
  rejectHostKey: (sessionId: string) =>
    invoke<void>("ssh_reject_host_key", { sessionId }),
};

/** Запись из ~/.ssh/known_hosts вместе с тем, что о ней уже известно (#175). */
export interface KnownHostsImportItem {
  hostname: string;
  keyType: string;
  fingerprint: string;
  /** Такая же запись уже есть, fingerprint совпадает — импортировать нечего. */
  alreadyKnown: boolean;
  /**
   * Запись есть, но fingerprint другой. Молча перезаписать её значило бы
   * обойти собственный TOFU, поэтому такие показываем отдельно.
   */
  conflicts: boolean;
  currentFingerprint?: string;
}

export interface KnownHostsImportPreview {
  path: string;
  items: KnownHostsImportItem[];
  skippedHashed: number;
  skippedWildcard: number;
  skippedMarker: number;
  skippedUnparsable: number;
}

export interface KnownHostImportEntry {
  hostname: string;
  keyType: string;
  fingerprint: string;
}

export const knownHostsImportApi = {
  preview: () => invoke<KnownHostsImportPreview>("known_hosts_import_preview"),
  /** Возвращает число добавленных записей. */
  run: (entries: KnownHostImportEntry[]) =>
    invoke<number>("known_hosts_import", { entries }),
};
