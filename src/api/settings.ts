import { invoke } from "@tauri-apps/api/core";

export const settingsApi = {
  get: (key: string) => invoke<string | null>("settings_get", { key }),
  set: (key: string, value: string) =>
    invoke<void>("settings_set", { key, value }),
  remove: (key: string) => invoke<void>("settings_remove", { key }),
  all: () => invoke<Record<string, string>>("settings_all"),
};

/**
 * Установленные моноширинные шрифты (#233). Пустой список — `fc-list` нет или
 * он не сработал: шрифт тогда вводят вручную, ошибкой это не считается.
 */
export const fontsApi = {
  list: () => invoke<string[]>("system_fonts"),
};
