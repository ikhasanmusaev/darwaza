import { invoke } from "@tauri-apps/api/core";

export interface CrashReport {
  name: string;
  path: string;
  size: number;
  modified: number | null;
}

/**
 * Отчёты о сбоях (#171).
 *
 * Никуда не отправляются: это файлы на диске, которые можно приложить к
 * багрепорту вручную. Сбор выключен по умолчанию.
 */
export const crashApi = {
  enabled: () => invoke<boolean>("crash_reports_enabled"),
  setEnabled: (enabled: boolean) =>
    invoke<void>("crash_reports_set_enabled", { enabled }),
  list: () => invoke<CrashReport[]>("crash_reports_list"),
  read: (path: string) => invoke<string>("crash_report_read", { path }),
  remove: (path: string) => invoke<void>("crash_report_remove", { path }),
  dir: () => invoke<string>("crash_reports_dir"),

  /**
   * Адрес для отправки (#171a). Пусто — отправка выключена.
   * Своего сервера у приложения нет: сюда вписывают собственный.
   */
  endpointGet: () => invoke<string | null>("crash_endpoint_get"),
  endpointSet: (url: string) => invoke<void>("crash_endpoint_set", { url }),
  /** Отправить отчёт. Возвращает HTTP-код ответа. */
  send: (path: string) => invoke<number>("crash_report_send", { path }),
  /** Сохранить копию в каталог загрузок. Возвращает путь. */
  export: (path: string) => invoke<string>("crash_report_export", { path }),
};
