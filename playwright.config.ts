import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  // Съёмка скриншотов для витрины — инструмент, а не проверка: она
  // перезаписывает картинки и в обычном прогоне не нужна.
  // Запуск: SHOOT=1 pnpm exec playwright test screenshots.spec.ts
  testIgnore: process.env.SHOOT ? [] : "**/screenshots.spec.ts",
  testDir: "./tests/e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: "list",
  use: {
    baseURL: process.env.BASE_URL ?? "http://localhost:1420",
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  // Tauri dev server should be started manually: pnpm tauri dev
  // webServer is intentionally omitted to avoid auto-starting backend
});
