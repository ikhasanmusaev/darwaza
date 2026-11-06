import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { hostsApi, type Host, type HostInput } from "../api/hosts";
import { groupsApi, type Group, type GroupInput } from "../api/groups";

function errMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}

export const useHostsStore = defineStore("hosts", () => {
  const hosts = ref<Host[]>([]);
  const groups = ref<Group[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const initialized = ref(false);

  const groupsById = computed(() => {
    const map = new Map<number, Group>();
    for (const g of groups.value) map.set(g.id, g);
    return map;
  });

  function groupName(groupId: number | null): string | null {
    if (groupId == null) return null;
    return groupsById.value.get(groupId)?.name ?? null;
  }

  async function fetchAll() {
    loading.value = true;
    error.value = null;
    try {
      const [hostsList, groupsList] = await Promise.all([
        hostsApi.list(),
        groupsApi.list(),
      ]);
      hosts.value = hostsList;
      groups.value = groupsList;
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

  async function createHost(data: HostInput): Promise<Host> {
    const created = await hostsApi.create(data);
    hosts.value = [...hosts.value, created];
    return created;
  }

  async function updateHost(id: number, data: HostInput): Promise<Host> {
    const updated = await hostsApi.update(id, data);
    hosts.value = hosts.value.map((h) => (h.id === id ? updated : h));
    return updated;
  }

  async function removeHost(id: number): Promise<void> {
    await hostsApi.remove(id);
    hosts.value = hosts.value.filter((h) => h.id !== id);
  }

  function getHost(id: number): Host | undefined {
    return hosts.value.find((h) => h.id === id);
  }

  async function createGroup(data: GroupInput): Promise<Group> {
    const created = await groupsApi.create(data);
    groups.value = [...groups.value, created];
    return created;
  }

  async function updateGroup(id: number, data: GroupInput): Promise<Group> {
    const updated = await groupsApi.update(id, data);
    groups.value = groups.value.map((g) => (g.id === id ? updated : g));
    return updated;
  }

  async function removeGroup(id: number): Promise<void> {
    await groupsApi.remove(id);
    groups.value = groups.value.filter((g) => g.id !== id);
    // FK ON DELETE SET NULL — освобождаем groupId у локальных хостов
    hosts.value = hosts.value.map((h) =>
      h.groupId === id ? { ...h, groupId: null } : h,
    );
  }

  return {
    hosts,
    groups,
    loading,
    error,
    initialized,
    groupsById,
    groupName,
    fetchAll,
    ensureLoaded,
    createHost,
    updateHost,
    removeHost,
    getHost,
    createGroup,
    updateGroup,
    removeGroup,
  };
});
