import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

/** R07: меню настроек переключает секции, справа видна одна; под заголовком есть подзаголовок. */
test("настройки: одна секция за раз и подзаголовок", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("locale", "en"));
  await mockTauriInvoke(page, {
    hosts_list: () => [], groups_list: () => [], snippets_list: () => [],
    settings_all: () => ({}), settings_get: () => null, settings_set: () => null,
    ssh_active_sessions: () => [], agent_plans_pending: () => [],
    master_status: () => ({ enabled: false, unlocked: false, lockoutSecondsLeft: 0, failedAttempts: 0, minLength: 12 }),
    sync_status: () => ({ endpoint: "", email: "", signedIn: false, pending: 0, lastError: null, cursor: 0 }),
    crash_reports_enabled: () => false, crash_endpoint_get: () => null, crash_reports_list: () => [],
  } as never);
  await page.goto("/settings");
  await expect(page.getByText("All settings are stored on this device")).toBeVisible();
  await expect(page.getByTestId("sync-section")).toBeVisible();
  await expect(page.getByTestId("hosts-probe")).toBeHidden();
  await page.getByRole("button", { name: "SSH", exact: true }).click();
  await expect(page.getByTestId("hosts-probe")).toBeVisible();
  await expect(page.getByTestId("sync-section")).toBeHidden();
});
