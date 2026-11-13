import { invoke } from "@tauri-apps/api/core";

export type SftpEntryKind = "dir" | "file" | "symlink" | "other";

export interface SftpEntry {
  name: string;
  /** Полный абсолютный путь на сервере. */
  path: string;
  kind: SftpEntryKind;
  size: number;
  /** Unix-секунды; null если сервер не прислал атрибут. */
  modified: number | null;
  /** Unix-режим (например 0o644); null если атрибута нет. */
  permissions: number | null;
}

export const sftpApi = {
  /**
   * Соединение только для файлов (экран SFTP): вход и проверка ключа хоста те
   * же, что у терминала, но без оболочки и без вкладки. Возвращает id сразу;
   * готовность приходит событием `session:status`.
   */
  connect: (hostId: number) => invoke<{ sessionId: string }>("sftp_connect", { hostId }),

  /** Закрыть такое соединение целиком (не только SFTP-подсистему). */
  disconnect: (sessionId: string) => invoke<void>("ssh_disconnect", { sessionId }),

  /** Домашняя директория пользователя на сервере — точка входа браузера файлов. */
  home: (sessionId: string) => invoke<string>("sftp_home", { sessionId }),

  /** Список файлов: директории первыми, дальше по имени (сортирует backend). */
  list: (sessionId: string, path: string) =>
    invoke<SftpEntry[]>("sftp_list", { sessionId, path }),

  /** Путь на уровень выше. Разбор путей живёт в backend, чтобы не дублировать. */
  parent: (path: string) => invoke<string>("sftp_parent", { path }),

  mkdir: (sessionId: string, path: string) =>
    invoke<void>("sftp_mkdir", { sessionId, path }),

  /** Директория должна быть пустой — рекурсивного удаления в backend нет намеренно. */
  remove: (sessionId: string, path: string, isDir: boolean) =>
    invoke<void>("sftp_remove", { sessionId, path, isDir }),

  rename: (sessionId: string, from: string, to: string) =>
    invoke<void>("sftp_rename", { sessionId, from, to }),

  /** Возвращает число переданных байт. */
  download: (sessionId: string, remotePath: string, localPath: string) =>
    invoke<number>("sftp_download", { sessionId, remotePath, localPath }),

  /** Возвращает число переданных байт. */
  upload: (sessionId: string, localPath: string, remotePath: string) =>
    invoke<number>("sftp_upload", { sessionId, localPath, remotePath }),

  /** Положить картинку (PNG в base64) на сервер; возвращает путь к файлу. */
  putClipboardImage: (sessionId: string, data: string) =>
    invoke<string>("sftp_put_clipboard_image", { sessionId, data }),

  /** Закрыть SFTP-подсистему, не трогая терминал этой же сессии. */
  close: (sessionId: string) => invoke<void>("sftp_close", { sessionId }),
};
