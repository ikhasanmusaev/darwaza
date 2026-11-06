import { defineStore } from "pinia";
import { ref, computed } from "vue";
import {
  portForwardsApi,
  type ForwardSnapshot,
  type PortForward,
  type PortForwardInput,
} from "../api/port_forwards";

function errMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}

export const usePortForwardsStore = defineStore("portForwards", () => {
  const forwards = ref<PortForward[]>([]);
  const active = ref<ForwardSnapshot[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const initialized = ref(false);

  /** Быстрый доступ «правило → активно ли сейчас». */
  const activeById = computed(() => {
    const map: Record<number, ForwardSnapshot> = {};
    for (const s of active.value) map[s.forwardId] = s;
    return map;
  });

  async function fetchAll() {
    loading.value = true;
    error.value = null;
    try {
      forwards.value = await portForwardsApi.list();
      active.value = await portForwardsApi.active();
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

  /** Активные пробросы живут в памяти backend — состояние подтягиваем отдельно. */
  async function refreshActive() {
    try {
      active.value = await portForwardsApi.active();
    } catch (e) {
      error.value = errMessage(e);
    }
  }

  async function create(data: PortForwardInput): Promise<PortForward> {
    const created = await portForwardsApi.create(data);
    forwards.value = [...forwards.value, created];
    return created;
  }

  async function update(id: number, data: PortForwardInput): Promise<PortForward> {
    const updated = await portForwardsApi.update(id, data);
    forwards.value = forwards.value.map((f) => (f.id === id ? updated : f));
    await refreshActive();
    return updated;
  }

  async function remove(id: number): Promise<void> {
    await portForwardsApi.remove(id);
    forwards.value = forwards.value.filter((f) => f.id !== id);
    await refreshActive();
  }

  async function start(id: number, sessionId: string): Promise<string> {
    const bound = await portForwardsApi.start(id, sessionId);
    await refreshActive();
    return bound;
  }

  async function stop(id: number): Promise<void> {
    await portForwardsApi.stop(id);
    await refreshActive();
  }

  function byHost(hostId: number): PortForward[] {
    return forwards.value.filter((f) => f.hostId === hostId);
  }

  return {
    forwards,
    active,
    activeById,
    loading,
    error,
    initialized,
    fetchAll,
    ensureLoaded,
    refreshActive,
    create,
    update,
    remove,
    start,
    stop,
    byHost,
  };
});
