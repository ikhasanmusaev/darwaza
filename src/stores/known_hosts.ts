import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { knownHostsApi, type KnownHost } from "../api/known_hosts";
import type { HostKeyPromptPayload } from "../lib/eventBus";

function errMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}

export const useKnownHostsStore = defineStore("knownHosts", () => {
  const knownHosts = ref<KnownHost[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const initialized = ref(false);

  /**
   * Очередь pending host-key prompt'ов. Один глобальный диалог в Layout берёт
   * первый из очереди и просит пользователя accept/reject. После ответа
   * элемент извлекается, диалог берёт следующий (если есть).
   */
  const prompts = ref<HostKeyPromptPayload[]>([]);
  const activePrompt = computed(() => prompts.value[0] ?? null);

  async function fetchAll() {
    loading.value = true;
    error.value = null;
    try {
      knownHosts.value = await knownHostsApi.list();
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

  async function remove(id: number): Promise<void> {
    await knownHostsApi.remove(id);
    knownHosts.value = knownHosts.value.filter((kh) => kh.id !== id);
  }

  async function clear(): Promise<void> {
    await knownHostsApi.clear();
    knownHosts.value = [];
  }

  // ── Prompt queue (TOFU) ──────────────────────────────────────────────

  /** Обработчик события `session:host_key_prompt` — кладёт в очередь. */
  function enqueuePrompt(p: HostKeyPromptPayload) {
    prompts.value = [...prompts.value, p];
  }

  /** Снять prompt из очереди (после accept/reject). */
  function dequeuePrompt(sessionId: string) {
    prompts.value = prompts.value.filter((p) => p.sessionId !== sessionId);
  }

  async function accept(sessionId: string) {
    await knownHostsApi.acceptHostKey(sessionId);
    dequeuePrompt(sessionId);
    // Обновим локальный кэш чтобы UI Settings → Known Hosts видел новую запись.
    if (initialized.value) await fetchAll();
  }

  async function reject(sessionId: string) {
    await knownHostsApi.rejectHostKey(sessionId);
    dequeuePrompt(sessionId);
  }

  return {
    knownHosts,
    loading,
    error,
    initialized,
    prompts,
    activePrompt,
    fetchAll,
    ensureLoaded,
    remove,
    clear,
    enqueuePrompt,
    dequeuePrompt,
    accept,
    reject,
  };
});
