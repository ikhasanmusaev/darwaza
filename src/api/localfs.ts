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
  /** Создать папку `name` внутри `dir`; возвращает путь новой папки. */
  mkdir: (dir: string, name: string) => invoke<string>("local_mkdir", { dir, name }),
  /** Переименовать запись в её же папке; существующую не затирает. */
  rename: (path: string, newName: string) =>
    invoke<string>("local_rename", { path, newName }),
  /** Файл или пустая папка; рекурсивного удаления нет. */
  remove: (path: string, isDir: boolean) => invoke<void>("local_remove", { path, isDir }),
};
