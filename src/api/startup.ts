import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** То же имя, что в `src-tauri/src/startup.rs`. */
export const STARTUP_FAILED_EVENT = "app:startup-failed";

export interface StartupFailure {
  /** Техническая причина как есть — показывается человеку целиком. */
  message: string;
  /** Каталог журнала: куда смотреть дальше. */
  logPath: string;
}

/**
 * Что случилось на старте. `null` — всё в порядке.
 *
 * Спрашиваем при монтировании, потому что отказ мог произойти до того, как
 * окно успело подписаться на событие: база открывается фоновой задачей.
 */
export async function getStartupFailure(): Promise<StartupFailure | null> {
  return await invoke<StartupFailure | null>("startup_failure");
}

/** Отказ, случившийся уже после того, как окно открылось. */
export async function listenStartupFailure(
  onFailure: (failure: StartupFailure) => void,
): Promise<UnlistenFn> {
  return await listen<StartupFailure>(STARTUP_FAILED_EVENT, (e) =>
    onFailure(e.payload),
  );
}
