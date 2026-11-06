import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

/**
 * Второй фактор на вход в аккаунт (#153b).
 *
 * Это защита доступа к хранилищу и расшифровке паролей — к подключениям по
 * SSH отношения не имеет: там метод аутентификации задаёт сам сервер.
 */
function makeHandlers(state: any) {
  return {
    hosts_list: () => [],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],
    crash_reports_enabled: () => false,
    crash_reports_list: () => [],
    crash_reports_dir: () => "/crashes",
    crash_endpoint_get: () => null,

    master_status: () => ({
      enabled: state.enabled,
      unlocked: state.unlocked,
      lockoutSecondsLeft: 0,
      failedAttempts: 0,
      totpEnabled: state.totpEnabled,
      minLength: 12,
    }),
    master_unlock: (args: any) => {
      if (state.totpEnabled && !args.code) throw new Error("errors.masterTotpRequired");
      if (state.totpEnabled && args.code !== "424242") throw new Error("errors.masterTotpWrong");
      state.unlocked = true;
    },
    master_totp_begin: () => ({
      secret: "JBSWY3DPEHPK3PXP",
      uri: "otpauth://totp/Darwaza?secret=JBSWY3DPEHPK3PXP",
    }),
    master_totp_confirm: (args: any) => {
      if (args.code !== "424242") throw new Error("errors.masterTotpWrong");
      state.confirmed.push(args.secret);
      state.totpEnabled = true;
    },
    master_totp_disable: (args: any) => {
      if (args.code !== "424242") throw new Error("errors.masterTotpWrong");
      state.totpEnabled = false;
    },
  };
}

test.describe("S24 — Account second factor", () => {
  test("is not enabled until a code proves the authenticator works", async ({ page }) => {
    const state = { enabled: true, unlocked: true, totpEnabled: false, confirmed: [] as string[] };
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/settings");
    await page.getByRole("button", { name: "Xavfsizlik", exact: true }).click();

    await page.getByTestId("totp-begin").click();
    // Секрет показан, но ещё НЕ сохранён — иначе можно запереть себя снаружи
    await expect(page.getByTestId("totp-secret")).toContainText("JBSWY3DPEHPK3PXP");
    expect(state.totpEnabled).toBe(false);

    await page.getByTestId("totp-confirm-code").fill("000000");
    await page.getByTestId("totp-confirm").click();
    await expect(page.getByText(/Kod noto‘g‘ri/)).toBeVisible();
    expect(state.totpEnabled).toBe(false);

    await page.getByTestId("totp-confirm-code").fill("424242");
    await page.getByTestId("totp-confirm").click();
    await expect.poll(() => state.confirmed).toEqual(["JBSWY3DPEHPK3PXP"]);
  });

  test("asks for the code at sign-in and refuses a wrong one", async ({ page }) => {
    const state = { enabled: true, unlocked: false, totpEnabled: true, confirmed: [] as string[] };
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/hosts");

    const gate = page.getByTestId("master-unlock-gate");
    await expect(gate).toBeVisible();

    // Поле кода появляется только когда второй фактор включён
    const codeInput = page.getByTestId("master-unlock-code");
    await expect(codeInput).toBeVisible();

    await page.getByTestId("master-unlock-input").fill("correct horse");
    await codeInput.fill("000000");
    await page.getByRole("button", { name: /Qulfdan chiqarish|Разблокировать|Unlock/ }).click();
    await expect(page.getByText(/Kod noto‘g‘ri/)).toBeVisible();
    expect(state.unlocked).toBe(false);

    // Пароль остался на месте — перенабирать его из-за опечатки в коде не нужно
    await expect(page.getByTestId("master-unlock-input")).toHaveValue("correct horse");

    await codeInput.fill("424242");
    await page.getByRole("button", { name: /Qulfdan chiqarish|Разблокировать|Unlock/ }).click();
    await expect.poll(() => state.unlocked).toBe(true);
  });

  test("no code field when the second factor is off", async ({ page }) => {
    const state = { enabled: true, unlocked: false, totpEnabled: false, confirmed: [] as string[] };
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/hosts");

    await expect(page.getByTestId("master-unlock-gate")).toBeVisible();
    await expect(page.getByTestId("master-unlock-code")).toHaveCount(0);
  });
});
