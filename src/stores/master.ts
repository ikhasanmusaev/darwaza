import { defineStore } from "pinia";
import { ref } from "vue";
import { masterApi, secretsApi, type MasterStatus, type SecretsPlace } from "../api/master";

function errMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}

export const useMasterStore = defineStore("master", () => {
  const status = ref<MasterStatus>({
    enabled: false,
    unlocked: false,
    lockoutSecondsLeft: 0,
    failedAttempts: 0,
    totpEnabled: false,
    recoveryKeySet: false,
    // 0 — «ещё не спрашивали»: число приходит из ядра (master_status.minLength).
    minLength: 0,
    // Нули — «ещё не спрашивали»: подпись про Argon2id показывать только
    // когда `memoryKib > 0`, а не вписывать число-заглушку.
    kdf: { memoryKib: 0, iterations: 0, parallelism: 0 },
  });
  /**
   * Где секреты лежат сейчас (#169, #171). `null` — ещё не спрашивали.
   *
   * Держится рядом со статусом мастер-пароля намеренно: включение и
   * выключение мастер-пароля меняет ответ, и разъехаться они не должны.
   */
  const place = ref<SecretsPlace | null>(null);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function refreshPlace() {
    try {
      place.value = await secretsApi.place();
    } catch (e) {
      error.value = errMessage(e);
    }
  }

  async function refresh() {
    loading.value = true;
    try {
      status.value = await masterApi.status();
      place.value = await secretsApi.place();
    } catch (e) {
      error.value = errMessage(e);
    } finally {
      loading.value = false;
    }
  }

  /**
   * Нужно ли показать экран разблокировки: пароль включён, но ключа в памяти
   * нет — значит секреты недоступны и подключаться бессмысленно.
   */
  function needsUnlock(): boolean {
    return status.value.enabled && !status.value.unlocked;
  }

  async function enable(password: string): Promise<number> {
    const moved = await masterApi.enable(password);
    await refresh();
    return moved;
  }

  async function disable(password: string): Promise<number> {
    const moved = await masterApi.disable(password);
    await refresh();
    return moved;
  }

  async function unlock(password: string, code?: string): Promise<void> {
    await masterApi.unlock(password, code);
    await refresh();
  }

  async function lock(): Promise<void> {
    await masterApi.lock();
    await refresh();
  }

  async function changePassword(
    oldPassword: string,
    newPassword: string,
  ): Promise<void> {
    await masterApi.changePassword(oldPassword, newPassword);
    await refresh();
  }

  /** Выпустить ключ восстановления. Возвращает его — показать один раз. */
  async function issueRecoveryKey(): Promise<string> {
    const key = await masterApi.recoveryIssue();
    await refresh();
    return key;
  }

  /** Восстановить доступ ключом. Возвращает новый ключ взамен использованного. */
  async function recover(recoveryKey: string, newPassword: string): Promise<string> {
    const replacement = await masterApi.recover(recoveryKey, newPassword);
    await refresh();
    return replacement;
  }

  return {
    issueRecoveryKey,
    recover,
    status,
    loading,
    error,
    refresh,
    needsUnlock,
    enable,
    disable,
    unlock,
    lock,
    changePassword,
    place,
    refreshPlace,
  };
});
