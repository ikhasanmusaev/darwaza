import { invoke } from "@tauri-apps/api/core";

export type TransferKind = "upload" | "download" | "copy";
export type TransferStatus = "RUNNING" | "DONE" | "FAILED" | "CANCELLED";

export interface TransferProgress {
  id: string;
  kind: TransferKind;
  name: string;
  transferred: number;
  /** Полный размер, если сервер его сообщил. */
  total: number | null;
  status: TransferStatus;
  error?: string;
  localPath?: string;
  /** Файл внутри папки: копируется сейчас, а при ошибке — тот, на котором остановились. */
  currentPath?: string;
}

/** Сторона передачи для `transfer_tree`. */
export type TreeEndpoint = { kind: "local" } | { kind: "remote"; sessionId: string };

/**
 * Фоновые передачи (#124).
 *
 * Команды возвращают id сразу — сама передача идёт в фоне и отчитывается
 * событием `transfer:progress`.
 */
export const transfersApi = {
  download: (sessionId: string, remotePath: string, targetDir?: string) =>
    invoke<string>("transfer_download", { sessionId, remotePath, targetDir }),
  upload: (sessionId: string, localPath: string, remoteDir: string) =>
    invoke<string>("transfer_upload", { sessionId, localPath, remoteDir }),
  /**
   * Файл или папка целиком: компьютер ↔ сервер и сервер → сервер. Прогресс —
   * по общему размеру дерева.
   */
  tree: (
    source: TreeEndpoint,
    sourcePath: string,
    isDir: boolean,
    target: TreeEndpoint,
    targetDir: string,
  ) => invoke<string>("transfer_tree", { source, sourcePath, isDir, target, targetDir }),
  /** `false` — передача уже завершилась. */
  cancel: (id: string) => invoke<boolean>("transfer_cancel", { id }),
  downloadDir: () => invoke<string>("transfers_download_dir"),
};
