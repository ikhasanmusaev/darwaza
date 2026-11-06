import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

/** Подпись «-L Lokal (-L) · -D SOCKS5 (-D) (SOCKS5)» дублировала фильтры и повторяла слова. */
test("порты: нет дублирующей подписи под списком", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("locale", "uz"));
  await mockTauriInvoke(page, {
    hosts_list: () => [], groups_list: () => [], port_forwards_list: () => [],
    port_forwards_active: () => [], settings_all: () => ({}), settings_get: () => null,
    ssh_active_sessions: () => [], agent_plans_pending: () => [],
    master_status: () => ({ enabled: false, unlocked: false, lockoutSecondsLeft: 0, failedAttempts: 0, minLength: 12 }),
  } as never);
  await page.goto("/forwards");
  await expect(page.getByText("Hozircha qoida yo'q")).toBeVisible();
  await expect(page.getByText("(SOCKS5)", { exact: false }).filter({ hasText: "-D SOCKS5" })).toHaveCount(0);
});
