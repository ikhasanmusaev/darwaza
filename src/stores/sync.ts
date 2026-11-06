import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { errorText } from "../lib/errors";

/** Что вышло за проход синхронизации. */
export interface SyncReport {
  received: number;
  sent: number;
  conflicts: number;
  failed: string[];
  cursor: number;
}

export interface AccountStatus {
  endpoint: string;
  email: string | null;
  signedIn: boolean;
  pending: number;
  lastError: string | null;
  /** Когда последний проход удался, Unix-секунды; `null` — ещё не было (#232). */
  lastSyncAt: number | null;
  cursor: number;
}

const STATE_EVENT = "sync://state";

/**
 * Аккаунт и синхронизация (#190-195).
 *
 * Всё здесь устроено так, чтобы **отсутствие аккаунта не мешало работать**.
 * Приложение было локальным и остаётся им: не вошёл — ничего не сломалось,
 * сервер недоступен — ничего не сломалось. Поэтому ошибки складываются в
 * поле, которое показывается ненавязчиво, и ни одна из них не перекрывает
 * работу с хостами.
 */
export const useSyncStore = defineStore("sync", () => {
  const status = ref<AccountStatus | null>(null);
  const busy = ref(false);
  const error = ref<string | null>(null);
  const lastReport = ref<SyncReport | null>(null);
  /** Синхронизация по расписанию. По умолчанию включена. */
  const auto = ref(true);

  /**
   * Следить за состоянием: Rust сообщает после каждого прохода, удался он или
   * нет, так что подвал не опрашивает его по таймеру (#232).
   */
  async function subscribe(): Promise<UnlistenFn> {
    await refresh();
    return listen<AccountStatus>(STATE_EVENT, (e) => {
      status.value = e.payload;
    });
  }

  async function refresh(): Promise<void> {
    try {
      status.value = await invoke<AccountStatus>("sync_status");
      auto.value = (await invoke<string | null>("settings_get", { key: "sync.auto" })) !== "0";
    } catch (e) {
      // Молча: недоступное состояние синхронизации — не повод ломать
      // настройки целиком.
      error.value = errorText(e);
    }
  }

  async function setAuto(value: boolean): Promise<void> {
    // Записываем "0"/"1", а не булево: настройки хранятся строками, и
    // отсутствие ключа должно означать «включено».
    await invoke("settings_set", { key: "sync.auto", value: value ? "1" : "0" });
    auto.value = value;
  }

  async function setEndpoint(url: string): Promise<void> {
    error.value = null;
    try {
      await invoke("sync_set_endpoint", { url });
      await refresh();
    } catch (e) {
      error.value = errorText(e);
    }
  }

  /**
   * Завести аккаунт. Возвращает ключ восстановления (#180c).
   *
   * Ключ выпускается **вместе с аккаунтом**, а не когда-нибудь потом:
   * отложенный ключ не выпускают никогда, а без него у забытого пароля нет
   * запасного входа вовсе — сброса по почте здесь не существует.
   */
  async function register(email: string, password: string): Promise<string | null> {
    if (busy.value) return null;
    busy.value = true;
    error.value = null;
    try {
      const key = await invoke<string>("sync_register", { email, password });
      await refresh();
      return key;
    } catch (e) {
      error.value = errorText(e);
      return null;
    } finally {
      busy.value = false;
    }
  }

  async function login(
    email: string,
    password: string,
    totpCode?: string,
  ): Promise<boolean> {
    return run(() =>
      invoke("sync_login", { email, password, totpCode: totpCode || null }),
    );
  }

  async function logout(): Promise<boolean> {
    return run(() => invoke("sync_logout"));
  }

  /**
   * Выпустить или перевыпустить ключ восстановления (#180c).
   *
   * Возвращает сам ключ — показать его человеку нужно **один раз**, второго
   * раза не будет: на сервере лежит только обёртка, открыть которую без
   * ключа нельзя. Перевыпуск отзывает прежнюю бумажку, поэтому и требует
   * пароль, а не только открытую сессию.
   */
  async function issueRecoveryKey(password: string): Promise<string | null> {
    if (busy.value) return null;
    busy.value = true;
    error.value = null;
    try {
      return await invoke<string>("sync_recovery_issue", { password });
    } catch (e) {
      error.value = errorText(e);
      return null;
    } finally {
      busy.value = false;
    }
  }

  /**
   * Восстановить доступ ключом с бумаги (#180c).
   *
   * Возвращает **новый** ключ: прежний перестаёт работать в тот же момент,
   * когда начинает работать новый пароль. Все сессии отозваны, поэтому после
   * этого нужно войти заново — это не сбой, а условие восстановления.
   */
  async function recover(
    email: string,
    recoveryKey: string,
    newPassword: string,
  ): Promise<string | null> {
    if (busy.value) return null;
    busy.value = true;
    error.value = null;
    try {
      const next = await invoke<string>("sync_recover", {
        email,
        recoveryKey,
        newPassword,
      });
      await refresh();
      return next;
    } catch (e) {
      error.value = errorText(e);
      return null;
    } finally {
      busy.value = false;
    }
  }

  async function syncNow(): Promise<boolean> {
    if (busy.value) return false;
    busy.value = true;
    error.value = null;
    try {
      lastReport.value = await invoke<SyncReport>("sync_now");
      await refresh();
      return true;
    } catch (e) {
      error.value = errorText(e);
      await refresh();
      return false;
    } finally {
      busy.value = false;
    }
  }

  /** Общая обёртка: занятость, ошибка, обновление состояния. */
  async function run(action: () => Promise<unknown>): Promise<boolean> {
    if (busy.value) return false;
    busy.value = true;
    error.value = null;
    try {
      await action();
      await refresh();
      return true;
    } catch (e) {
      error.value = errorText(e);
      return false;
    } finally {
      busy.value = false;
    }
  }

  return {
    status,
    auto,
    setAuto,
    busy,
    error,
    lastReport,
    refresh,
    subscribe,
    setEndpoint,
    register,
    login,
    logout,
    issueRecoveryKey,
    recover,
    syncNow,
  };
});
