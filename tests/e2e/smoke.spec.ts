import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

test.describe("S1 — App startup smoke", () => {
  test("app starts, sidebar visible, hosts list empty", async ({ page }) => {
    // Без мока IPC `hosts_list` падает и страница показывает ошибку вместо
    // пустого состояния — пустой список это тоже результат работы backend.
    await mockTauriInvoke(page, {
      hosts_list: () => [],
      groups_list: () => [],
      snippets_list: () => [],
      settings_all: () => ({}),
      settings_get: () => null,
      ssh_active_sessions: () => [],
      sync_status: () => ({
        endpoint: "https://api.darwaza.uz/api/v1",
        email: null,
        signedIn: false,
        pending: 0,
        lastError: null,
        lastSyncAt: null,
        cursor: 0,
      }),
    });

    await page.goto("/");

    // Именно по роли button: подпись "Hostlar" встречается также на странице.
    const sidebar = page.getByRole("complementary");
    await expect(sidebar.getByRole("button", { name: "Hostlar" })).toBeVisible();
    await expect(
      sidebar.getByRole("button", { name: "Sozlamalar" }),
    ).toBeVisible();
    await expect(sidebar.getByTestId("local-mode-status")).toContainText("Lokal rejim");
    await expect(sidebar.getByTestId("local-mode-status")).toContainText("sinx o'chiq");

    // Hosts page rendered (redirect from /)
    await expect(page.locator('[data-testid="hosts-empty"]')).toBeVisible();

    // Add host button
    await expect(page.locator("text=Yangi host qo'shish")).toBeVisible();

    // Search input
    await expect(
      page.locator('input[placeholder="Qidirish..."]'),
    ).toBeVisible();
  });
});
