import { defineStore } from "pinia";
import { ref } from "vue";
import { recordingsApi, type RecordingFile } from "../api/recordings";

function errMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}

export const useRecordingsStore = defineStore("recordings", () => {
  const files = ref<RecordingFile[]>([]);
  const dir = ref<string>("");
  const loading = ref(false);
  const error = ref<string | null>(null);

  /**
   * Какие сессии сейчас пишутся: sessionId → путь к файлу.
   *
   * Состояние живёт в backend (рекордер открыт внутри задачи сессии), здесь
   * держим отражение, чтобы кнопка знала, что показывать.
   */
  const active = ref<Record<string, string>>({});

  async function refresh() {
    loading.value = true;
    error.value = null;
    try {
      files.value = await recordingsApi.list();
      dir.value = await recordingsApi.dir();
    } catch (e) {
      error.value = errMessage(e);
    } finally {
      loading.value = false;
    }
  }

  function isRecording(sessionId: string): boolean {
    return !!active.value[sessionId];
  }

  async function start(sessionId: string): Promise<string> {
    const path = await recordingsApi.start(sessionId);
    active.value = { ...active.value, [sessionId]: path };
    return path;
  }

  async function stop(sessionId: string): Promise<string> {
    const path = await recordingsApi.stop(sessionId);
    const next = { ...active.value };
    delete next[sessionId];
    active.value = next;
    await refresh();
    return path;
  }

  async function remove(path: string): Promise<void> {
    await recordingsApi.remove(path);
    files.value = files.value.filter((f) => f.path !== path);
  }

  /** Сессия закрылась — запись backend закрыл сам, снимаем отметку. */
  function forget(sessionId: string): void {
    if (!active.value[sessionId]) return;
    const next = { ...active.value };
    delete next[sessionId];
    active.value = next;
  }

  return {
    files,
    dir,
    active,
    loading,
    error,
    refresh,
    isRecording,
    start,
    stop,
    remove,
    forget,
  };
});
