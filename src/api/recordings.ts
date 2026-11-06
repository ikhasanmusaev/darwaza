import { invoke } from "@tauri-apps/api/core";

export interface RecordingFile {
  name: string;
  path: string;
  size: number;
  /** Unix-секунды. */
  modified: number | null;
  /** «имя хоста (адрес)» из заголовка записи (#230). */
  title: string | null;
  /** Когда запись началась, Unix-секунды. */
  startedAt: number | null;
  /** Секунды; у идущей записи — с начала до сейчас. */
  durationSeconds: number | null;
  /** Пишется сейчас: показывать «стоп», не «проиграть»; удалить нельзя. */
  active: boolean;
}

/** Кусок записи для проигрывателя (#230). */
export interface RecordingChunk {
  /**
   * Целые строки файла как есть: первая — заголовок asciinema, остальные —
   * события `[время, "o", текст]` в JSON. Разбирает проигрыватель.
   */
  lines: string[];
  nextOffset: number;
  /** Дочитали до конца того, что есть; у идущей записи позже будет больше. */
  eof: boolean;
}

/**
 * Запись сессии (#144).
 *
 * Формат — asciinema v2 (`.cast`), поэтому записи проигрываются стандартным
 * `asciinema play`, а не только внутри приложения. Пишется только вывод
 * сервера: ввод не записывается, чтобы в файл не попали пароли.
 */
export const recordingsApi = {
  /** Начать запись активной сессии. Возвращает путь к файлу. */
  start: (sessionId: string) =>
    invoke<string>("session_recording_start", { sessionId }),
  /** Остановить запись. Возвращает путь к закрытому файлу. */
  stop: (sessionId: string) =>
    invoke<string>("session_recording_stop", { sessionId }),

  list: () => invoke<RecordingFile[]>("recordings_list"),
  remove: (path: string) => invoke<void>("recordings_remove", { path }),
  /** Читать с `offset = 0`, дальше — `nextOffset`, пока не `eof`. */
  read: (path: string, offset: number, maxBytes?: number) =>
    invoke<RecordingChunk>("recording_read", { path, offset, maxBytes }),
  dir: () => invoke<string>("recordings_dir"),
};
