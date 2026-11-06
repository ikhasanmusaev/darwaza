import { invoke } from "@tauri-apps/api/core";

/**
 * Первый запуск (#235).
 *
 * `needed` — только для действительно нового приложения: у обновившегося с
 * прежней версии уже есть хосты, ключи или аккаунт, и мастер ему не нужен.
 */
export const onboardingApi = {
  needed: () => invoke<boolean>("onboarding_needed"),
  /** Отметить, что мастер пройден или пропущен. */
  done: () => invoke<void>("onboarding_done"),
  /**
   * Сохранить ключ восстановления в файл с правами 0600. Путь выбирает
   * человек; существующий файл не затирается (`errors.fileExists`).
   */
  saveRecoveryKey: (path: string, key: string) =>
    invoke<void>("recovery_key_save", { path, key }),
};
