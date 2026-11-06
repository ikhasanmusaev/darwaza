import { defineStore } from "pinia";
import { ref } from "vue";
import { snippetsApi, type Snippet, type SnippetInput } from "../api/snippets";
import { expandSnippet, type SnippetContext } from "../lib/snippet-vars";
import { useSessionsStore } from "./sessions";
import { useHostsStore } from "./hosts";

function errMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}

export const useSnippetsStore = defineStore("snippets", () => {
  const snippets = ref<Snippet[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const initialized = ref(false);

  async function fetchAll() {
    loading.value = true;
    error.value = null;
    try {
      snippets.value = await snippetsApi.list();
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

  async function create(data: SnippetInput): Promise<Snippet> {
    const created = await snippetsApi.create(data);
    snippets.value = [...snippets.value, created];
    return created;
  }

  async function update(id: number, data: SnippetInput): Promise<Snippet> {
    const updated = await snippetsApi.update(id, data);
    snippets.value = snippets.value.map((s) => (s.id === id ? updated : s));
    return updated;
  }

  async function remove(id: number): Promise<void> {
    await snippetsApi.remove(id);
    snippets.value = snippets.value.filter((s) => s.id !== id);
  }

  function getById(id: number): Snippet | undefined {
    return snippets.value.find((s) => s.id === id);
  }

  /** Контекст подстановки по хосту активной сессии. null — если сессии нет. */
  function activeContext(): SnippetContext | null {
    const sessions = useSessionsStore();
    const hosts = useHostsStore();
    const active = sessions.activeSession;
    if (!active) return null;
    const host = hosts.hosts.find((h) => h.id === active.hostId);
    if (!host) return null;
    return {
      host: host.hostname,
      user: host.username,
      port: host.port,
      name: host.name,
    };
  }

  /**
   * Отправить сниппет в активную сессию: подставить переменные и дописать `\n`,
   * чтобы шелл выполнил команду.
   *
   * Возвращает `false`, если активной сессии нет или она не в состоянии
   * CONNECTED — писать stdin в мёртвый канал бессмысленно.
   */
  async function runOnActiveSession(snippet: Snippet): Promise<boolean> {
    const sessions = useSessionsStore();
    const active = sessions.activeSession;
    if (!active || active.status !== "CONNECTED") return false;

    const command = expandSnippet(snippet.command, activeContext());
    const payload = new TextEncoder().encode(`${command}\n`);
    await sessions.send(active.sessionId, payload);
    return true;
  }

  return {
    snippets,
    loading,
    error,
    initialized,
    fetchAll,
    ensureLoaded,
    create,
    update,
    remove,
    getById,
    activeContext,
    runOnActiveSession,
  };
});
