import { invoke } from "@tauri-apps/api/core";

export type LocalEntryKind = "dir" | "file" | "symlink" | "other";

export interface LocalEntry {
  name: string;
  path: string;
  kind: LocalEntryKind;
  size: number;
  /** Unix-секунды; null если ФС не отдала время модификации. */
  modified: number | null;
}

/**
 * Локальная файловая система — левая колонка файлового браузера.
 *
 * Наружу отдаётся только листинг: содержимое файлов в UI не попадает,
 * чтением при выгрузке занимается backend (`sftp_upload`).
 */
export const localFsApi = {
  home: () => invoke<string>("local_home"),
  parent: (path: string) => invoke<string>("local_parent", { path }),
  list: (path: string) => invoke<LocalEntry[]>("local_list", { path }),
};
