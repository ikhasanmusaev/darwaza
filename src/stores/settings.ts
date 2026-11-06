import { defineStore } from "pinia";
import { ref } from "vue";
import { settingsApi } from "../api/settings";

function errMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}

/**
 * Key-value store, mirroring `settings` SQLite table.
 * Значения хранятся в БД как строки. Если нужно положить число/булеан/объект —
 * сериализуйте сами (JSON.stringify) на стороне вызывающего.
 */
export const useSettingsStore = defineStore("settings", () => {
  const values = ref<Record<string, string>>({});
  const loading = ref(false);
  const error = ref<string | null>(null);
  const initialized = ref(false);

  async function fetchAll() {
    loading.value = true;
    error.value = null;
    try {
      values.value = await settingsApi.all();
      initialized.value = true;
    } catch (e) {
      error.value = errMessage(e);
    } finally {
      loading.value = false;
    }
  }

  /**
   * Загрузить один раз и дождаться загрузки, даже если её начал кто-то другой.
   *
   * Прежняя проверка `!loading.value` возвращала управление СРАЗУ, когда
   * запрос уже был в пути: вызывающий читал пустой список и решал, что данных
   * нет. Так пропадала выбранная идентификация в форме хоста и появлялось
   * «Host topilmadi» на существующем хосте (2026-08-28). Держим сам запрос и
   * ждём его.
   */
  let inflight: Promise<void> | null = null;
  async function ensureLoaded() {
    if (initialized.value) return;
    inflight ??= fetchAll().finally(() => {
      inflight = null;
    });
    await inflight;
  }

  function get(key: string): string | null {
    return values.value[key] ?? null;
  }

  async function set(key: string, value: string): Promise<void> {
    await settingsApi.set(key, value);
    values.value = { ...values.value, [key]: value };
  }

  async function remove(key: string): Promise<void> {
    await settingsApi.remove(key);
    const next = { ...values.value };
    delete next[key];
    values.value = next;
  }

  return {
    values,
    loading,
    error,
    initialized,
    fetchAll,
    ensureLoaded,
    get,
    set,
    remove,
  };
});
