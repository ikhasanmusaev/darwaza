import { test, expect } from "@playwright/test";

/**
 * Переключение языка живёт в Settings → Ko'rinish (раньше дропдаун был в сайдбаре,
 * его убрали при чистке сайдбара — тест ходил по несуществующему элементу).
 */
test.describe("S9 — i18n language switch", () => {
  test("switch between uz, ru, en", async ({ page }) => {
    await page.goto("/settings");
    await page.getByRole("button", { name: "Ko'rinish", exact: true }).click();

    const sidebar = page.getByRole("complementary");
    const select = page.getByTestId("language-select");

    // Default is uz
    await expect(sidebar.getByRole("button", { name: "Hostlar" })).toBeVisible();

    // Switch to Russian
    await select.selectOption("ru");
    await expect(sidebar.getByRole("button", { name: "Хосты" })).toBeVisible();

    // Switch to English
    await select.selectOption("en");
    await expect(sidebar.getByRole("button", { name: "Hosts" })).toBeVisible();

    // Switch back to Uzbek
    await select.selectOption("uz");
    await expect(sidebar.getByRole("button", { name: "Hostlar" })).toBeVisible();

    // Языки, добавленные в #170
    await select.selectOption("kk");
    await expect(sidebar.getByRole("button", { name: "Хосттар" })).toBeVisible();

    await select.selectOption("ky");
    await expect(sidebar.getByRole("button", { name: "Хосттор" })).toBeVisible();

    await select.selectOption("tr");
    await expect(sidebar.getByRole("button", { name: "Sunucular" })).toBeVisible();

    // Язык переживает перезагрузку (хранится в localStorage)
    await select.selectOption("ru");
    await page.reload();
    await expect(sidebar.getByRole("button", { name: "Хосты" })).toBeVisible();
  });
});

test("подпись режима внизу сайдбара не обрезается ни на одном языке", async ({ page }) => {
  // Длинное «Локальный режим · синхронизация выключена» раньше не
  // переносилось и уходило за край сайдбара.
  for (const locale of ["uz", "ru", "en", "kk", "ky", "tr"]) {
    await page.goto("/settings");
    await page.evaluate((l) => window.localStorage.setItem("locale", l), locale);
    await page.reload();
    const status = page.getByTestId("local-mode-status");
    await expect(status).toBeVisible();
    const clipped = await status.evaluate((el) => {
      const side = el.closest("aside")!.getBoundingClientRect();
      const r = el.getBoundingClientRect();
      return r.right > side.right + 0.5 || el.scrollWidth > el.clientWidth + 0.5;
    });
    expect(clipped, locale).toBe(false);
  }
});
