import { defineStore } from "pinia";
import { ref } from "vue";
import {
  identitiesApi,
  type Identity,
  type IdentityInput,
} from "../api/identities";

function errMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}

export const useIdentitiesStore = defineStore("identities", () => {
  const identities = ref<Identity[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const initialized = ref(false);

  async function fetchAll() {
    loading.value = true;
    error.value = null;
    try {
      identities.value = await identitiesApi.list();
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

  async function create(data: IdentityInput): Promise<Identity> {
    const created = await identitiesApi.create(data);
    identities.value = [...identities.value, created];
    return created;
  }

  async function update(id: number, data: IdentityInput): Promise<Identity> {
    const updated = await identitiesApi.update(id, data);
    identities.value = identities.value.map((i) =>
      i.id === id ? updated : i,
    );
    return updated;
  }

  async function remove(id: number): Promise<void> {
    await identitiesApi.remove(id);
    identities.value = identities.value.filter((i) => i.id !== id);
  }

  function getById(id: number): Identity | undefined {
    return identities.value.find((i) => i.id === id);
  }

  /** Установить пароль в keyring для существующей identity. */
  async function setPassword(id: number, password: string): Promise<void> {
    await identitiesApi.setPassword(id, password);
  }

  /** Установить passphrase для зашифрованного private_key identity. */
  async function setPassphrase(id: number, passphrase: string): Promise<void> {
    await identitiesApi.setPassphrase(id, passphrase);
  }

  return {
    identities,
    loading,
    error,
    initialized,
    fetchAll,
    ensureLoaded,
    create,
    update,
    remove,
    getById,
    setPassword,
    setPassphrase,
  };
});
