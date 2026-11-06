import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

test.describe("S10 — Window resize", () => {
  test("sidebar remains visible after resize", async ({ page }) => {
    await mockTauriInvoke(page, {
      hosts_list: () => [],
      groups_list: () => [],
      snippets_list: () => [],
      settings_all: () => ({}),
      settings_get: () => null,
      ssh_active_sessions: () => [],
    });

    await page.goto("/");

    const navHosts = page
      .getByRole("complementary")
      .getByRole("button", { name: "Hostlar" });

    // Default viewport is 1280x720
    await expect(navHosts).toBeVisible();

    // Resize to a smaller desktop resolution
    await page.setViewportSize({ width: 1024, height: 768 });
    await expect(navHosts).toBeVisible();
    await expect(page.locator('[data-testid="hosts-empty"]')).toBeVisible();

    // Resize back
    await page.setViewportSize({ width: 1280, height: 720 });
    await expect(navHosts).toBeVisible();
  });
});
