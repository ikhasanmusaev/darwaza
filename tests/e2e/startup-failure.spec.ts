import { test, expect } from "@playwright/test";
import { mockTauriInvoke, emitTauriEvent } from "./helpers";

/**
 * #168 — отказ базы человек обязан увидеть.
 *
 * До этого окно открывалось как ни в чём не бывало, а первая же страница
 * показывала «state not managed for field `state`». Тест проверяет не текст
 * экрана, а именно это: что отказ доезжает обоими путями и что без отказа
 * экран не появляется.
 */
const base = {
  hosts_list: () => [],
  groups_list: () => [],
  snippets_list: () => [],
  settings_all: () => ({}),
  settings_get: () => null,
  ssh_active_sessions: () => [],
};

test.describe("Отказ на старте", () => {
  test("отказ, случившийся до открытия окна, показан", async ({ page }) => {
    await mockTauriInvoke(page, {
      ...base,
      startup_failure: () => ({
        message: "failed to apply migrations: migration 20260820000001 was previously applied but has been modified",
        logPath: "/home/user/.local/state/darwaza/log",
      }),
    });

    await page.goto("/");

    const gate = page.locator('[data-testid="startup-failure-gate"]');
    await expect(gate).toBeVisible();
    // Техническая причина показывается целиком: без неё разговор сводится
    // к «не работает».
    await expect(page.locator('[data-testid="startup-failure-message"]')).toContainText(
      "was previously applied but has been modified",
    );
    // Путь к журналу — то, куда человек пойдёт дальше.
    await expect(gate).toContainText("/home/user/.local/state/darwaza/log");
  });

  test("без отказа экрана нет", async ({ page }) => {
    await mockTauriInvoke(page, { ...base, startup_failure: () => null });

    await page.goto("/");

    await expect(page.locator('[data-testid="hosts-empty"]')).toBeVisible();
    await expect(
      page.locator('[data-testid="startup-failure-gate"]'),
    ).toHaveCount(0);
  });

  test("отказ, случившийся после открытия окна, тоже показан", async ({ page }) => {
    // База открывается фоновой задачей: окно успевает встать раньше, чем
    // приходит отказ. Ради этого случая и существует событие.
    await mockTauriInvoke(page, { ...base, startup_failure: () => null });

    await page.goto("/");
    await expect(page.locator('[data-testid="hosts-empty"]')).toBeVisible();

    await emitTauriEvent(page, "app:startup-failed", {
      message: "не удалось открыть базу",
      logPath: "/tmp/log",
    });

    await expect(
      page.locator('[data-testid="startup-failure-gate"]'),
    ).toBeVisible();
  });
});
