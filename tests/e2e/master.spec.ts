import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

interface MasterState {
  enabled: boolean;
  unlocked: boolean;
  password: string | null;
  failedAttempts: number;
  calls: string[];
  /** Действующий ключ восстановления, если выпущен (#180b). */
  recoveryKey?: string | null;
}

/** Ключ в том виде, в каком его показывает приложение. */
function printedKey(n: number): string {
  const group = String(n).padStart(4, "0").slice(-4);
  return Array(8).fill(group).join("-");
}

function makeHandlers(state: MasterState) {
  return {
    hosts_list: () => [],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    settings_set: () => {},
    ssh_active_sessions: () => [],

    master_status: () => ({
      enabled: state.enabled,
      unlocked: state.unlocked,
      lockoutSecondsLeft: 0,
      failedAttempts: state.failedAttempts,
      totpEnabled: false,
      recoveryKeySet: !!state.recoveryKey,
      minLength: 12,
      kdf: { memoryKib: 65536, iterations: 3, parallelism: 1 },
    }),
    master_recovery_issue: () => {
      state.recoveryKey = printedKey(state.calls.length + 1);
      state.calls.push("issue");
      return state.recoveryKey;
    },
    master_recover: (args: any) => {
      if (args.recoveryKey.trim().toUpperCase() !== state.recoveryKey) {
        throw new Error("errors.masterRecoveryWrong");
      }
      state.password = args.newPassword;
      state.unlocked = true;
      // Использованный ключ заменяется новым — так делает настоящий backend.
      state.recoveryKey = printedKey(state.calls.length + 7);
      state.calls.push("recover");
      return state.recoveryKey;
    },
    master_totp_begin: () => ({ secret: "", uri: "" }),
    secrets_place: () => ({
      keyringAvailable: true,
      masterEnabled: state.enabled,
      masterUnlocked: state.unlocked,
      target: state.enabled ? "master" : "keyring",
    }),
    master_enable: (args: any) => {
      state.enabled = true;
      state.unlocked = true;
      state.password = args.password;
      state.calls.push("enable");
      return 3;
    },
    master_disable: (args: any) => {
      if (args.password !== state.password) throw new Error("errors.masterWrongPassword");
      state.enabled = false;
      state.unlocked = false;
      state.calls.push("disable");
      return 3;
    },
    master_unlock: (args: any) => {
      if (args.password !== state.password) {
        state.failedAttempts += 1;
        throw new Error("errors.masterWrongPassword");
      }
      state.unlocked = true;
      state.calls.push("unlock");
    },
    master_lock: () => {
      state.unlocked = false;
      state.calls.push("lock");
    },
    master_change_password: () => {
      state.calls.push("change");
    },
  };
}

test.describe("S15 — Master password", () => {
  test("enables from settings and reports migrated secrets", async ({ page }) => {
    const state: MasterState = {
      enabled: false,
      unlocked: false,
      password: null,
      failedAttempts: 0,
      calls: [],
    };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/settings");
    await page.getByRole("button", { name: "Xavfsizlik", exact: true }).click();
    await page.getByTestId("master-enable").click();

    // Пока пароли не совпадают, сохранить нельзя — иначе пользователь
    // заблокирует секреты паролем, который набрал с опечаткой.
    await page.getByTestId("master-password").fill("s3cret-pass12");
    await page.getByTestId("master-password-repeat").fill("s3cret-typo");
    await expect(page.getByTestId("master-submit")).toBeDisabled();

    await page.getByTestId("master-password-repeat").fill("s3cret-pass12");
    await expect(page.getByTestId("master-submit")).toBeEnabled();
    await page.getByTestId("master-submit").click();

    await expect(page.getByText("Yoqildi, ko'chirilgan maxfiy ma'lumotlar: 3")).toBeVisible();
    expect(state.calls).toContain("enable");
  });

  test("blocks the app until unlocked and rejects a wrong password", async ({
    page,
  }) => {
    const state: MasterState = {
      enabled: true,
      unlocked: false,
      password: "right-one",
      failedAttempts: 0,
      calls: [],
    };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/hosts");

    // Экран разблокировки перекрывает приложение целиком.
    const gate = page.getByTestId("master-unlock-gate");
    await expect(gate).toBeVisible();

    await page.getByTestId("master-unlock-input").fill("wrong-one");
    await page.getByRole("button", { name: "Qulfdan chiqarish" }).click();
    // Backend отдаёт код, текст берётся из локали (#177).
    await expect(page.getByText("Parol noto‘g‘ri.")).toBeVisible();
    await expect(gate).toBeVisible();

    await page.getByTestId("master-unlock-input").fill("right-one");
    await page.getByRole("button", { name: "Qulfdan chiqarish" }).click();
    await expect(gate).toHaveCount(0);
    expect(state.calls).toContain("unlock");
  });
});

test.describe("S180b — ключ восстановления мастер-пароля", () => {
  test("при включении выпускается ключ, и окно не закрыть, пока его не записали", async ({
    page,
  }) => {
    const state: MasterState = {
      enabled: false,
      unlocked: false,
      password: null,
      failedAttempts: 0,
      calls: [],
    };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/settings");
    await page.getByRole("button", { name: "Xavfsizlik", exact: true }).click();
    await page.getByTestId("master-enable").click();
    await page.getByTestId("master-password").fill("s3cret-pass12");
    await page.getByTestId("master-password-repeat").fill("s3cret-pass12");
    await page.getByTestId("master-submit").click();

    // Ключ выпускается сразу, а не «потом»: отложенный ключ не выпускают никогда.
    const dialog = page.getByTestId("recovery-key-dialog");
    await expect(dialog).toBeVisible();
    await expect(page.getByTestId("recovery-key-value")).toContainText(
      state.recoveryKey!.split("-")[0],
    );

    // Ключ показывается один раз — случайный клик мимо или Esc не должен его унести.
    await expect(page.getByTestId("recovery-key-saved")).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(dialog).toBeVisible();
    await expect(page.getByTestId("recovery-key-done")).toBeDisabled();
    await page.getByTestId("recovery-key-saved").check();
    await page.getByTestId("recovery-key-understood").check();
    await page.getByTestId("recovery-key-done").click();
    await expect(dialog).toHaveCount(0);

    expect(state.calls).toEqual(["enable", "issue"]);
    await expect(page.getByTestId("recovery-section")).toContainText("berilgan");
  });

  test("забытый пароль: ключ задаёт новый и выдаёт замену", async ({ page }) => {
    const state: MasterState = {
      enabled: true,
      unlocked: false,
      password: "забытый",
      failedAttempts: 0,
      calls: [],
      recoveryKey: printedKey(42),
    };
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/hosts");

    const gate = page.getByTestId("master-unlock-gate");
    await expect(gate).toBeVisible();
    await page.getByTestId("recovery-start").click();

    // Не тот ключ — понятный ответ, а не «что-то пошло не так».
    await page.getByTestId("recovery-input").fill(printedKey(7));
    await page.getByTestId("recovery-new-password").fill("новый-пароль");
    await page.getByTestId("recovery-new-password-repeat").fill("новый-пароль");
    await page.getByTestId("recovery-submit").click();
    await expect(page.getByText("Tiklash kaliti mos kelmadi", { exact: false })).toBeVisible();

    // Верный ключ, набранный строчными, — регистр не должен стоить доступа.
    await page.getByTestId("recovery-input").fill(printedKey(42).toLowerCase());
    await page.getByTestId("recovery-new-password").fill("новый-пароль");
    await page.getByTestId("recovery-new-password-repeat").fill("новый-пароль");
    await page.getByTestId("recovery-submit").click();

    // Новый ключ показан, и экран разблокировки не уходит, пока его не записали.
    await expect(page.getByTestId("recovery-key-dialog")).toBeVisible();
    await expect(gate).toBeVisible();
    await page.getByTestId("recovery-key-saved").check();
    await page.getByTestId("recovery-key-understood").check();
    await page.getByTestId("recovery-key-done").click();

    await expect(gate).toHaveCount(0);
    expect(state.password).toBe("новый-пароль");
    expect(state.calls).toContain("recover");
  });
});
