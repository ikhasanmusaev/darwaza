import { invoke } from "@tauri-apps/api/core";

export interface MasterStatus {
  /** Включён ли мастер-пароль. */
  enabled: boolean;
  /** Разблокирован ли сейчас (ключ в памяти backend). */
  unlocked: boolean;
  /** Включён ли второй фактор на вход (#153b). */
  totpEnabled: boolean;
  /** Сколько секунд осталось до конца паузы после неудачных попыток. */
  lockoutSecondsLeft: number;
  failedAttempts: number;
  /** Выпущен ли ключ восстановления (#180b). */
  recoveryKeySet: boolean;
  /** Наименьшая длина нового пароля в знаках; число в разметку не вписывать. */
  minLength: number;
  /**
   * Параметры Argon2id: записанные, если мастер-пароль включён, иначе те, с
   * которыми он включится. Подпись под полем пароля берётся отсюда, а не из
   * разметки (#235).
   */
  kdf: { memoryKib: number; iterations: number; parallelism: number };
}

/**
 * Мастер-пароль (#110–#113).
 *
 * Сам пароль никогда не хранится: из него выводится ключ, который живёт только
 * в памяти backend до блокировки. UI передаёт пароль ровно на время операции.
 */
export const masterApi = {
  status: () => invoke<MasterStatus>("master_status"),

  /** Включить. Возвращает число секретов, перенесённых из keyring в хранилище. */
  enable: (password: string) => invoke<number>("master_enable", { password }),

  /** Выключить. Возвращает число секретов, возвращённых в keyring. */
  disable: (password: string) => invoke<number>("master_disable", { password }),

  unlock: (password: string, code?: string) =>
    invoke<void>("master_unlock", { password, code }),

  /**
   * Второй фактор на вход в аккаунт (#153b).
   *
   * `begin` только выдаёт секрет, но не включает: сперва человек должен
   * доказать кодом, что аутентификатор настроен, иначе он запрёт себя снаружи.
   */
  totpBegin: () => invoke<{ secret: string; uri: string }>("master_totp_begin"),
  totpConfirm: (secret: string, code: string) =>
    invoke<void>("master_totp_confirm", { secret, code }),
  totpDisable: (code: string) => invoke<void>("master_totp_disable", { code }),
  lock: () => invoke<void>("master_lock"),

  changePassword: (oldPassword: string, newPassword: string) =>
    invoke<void>("master_change_password", { oldPassword, newPassword }),

  /**
   * Выпустить ключ восстановления (#180b). Возвращает его один раз — узнать
   * заново неоткуда. Прежний ключ этим же вызовом перестаёт работать.
   */
  recoveryIssue: () => invoke<string>("master_recovery_issue"),

  /**
   * Восстановить доступ ключом и задать новый пароль. Возвращает новый ключ
   * взамен использованного — его тоже показывают один раз.
   */
  recover: (recoveryKey: string, newPassword: string) =>
    invoke<string>("master_recover", { recoveryKey, newPassword }),
};

/**
 * Где секреты лежат прямо сейчас (#169, #171).
 *
 * Спрашивается при открытии формы, а не при сохранении: человек должен знать,
 * есть ли куда класть пароль, до того как его наберёт.
 */
export interface SecretsPlace {
  /** Есть ли на машине рабочее системное хранилище. */
  keyringAvailable: boolean;
  masterEnabled: boolean;
  masterUnlocked: boolean;
  /** Куда пойдёт следующий секрет. `nowhere` требует решения человека. */
  target: "master" | "keyring" | "nowhere";
}

export const secretsApi = {
  place: () => invoke<SecretsPlace>("secrets_place"),
};
