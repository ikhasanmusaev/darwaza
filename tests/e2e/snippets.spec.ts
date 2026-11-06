import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

interface MockSnippet {
  id: number;
  name: string;
  command: string;
  description: string | null;
  agentAccess?: boolean;
  sortOrder: number;
  createdAt: string;
  updatedAt: string;
}

function makeHandlers(state: { snippets: MockSnippet[] }) {
  return {
    snippets_list: () => state.snippets,
    snippets_get: (args: any) => state.snippets.find((s) => s.id === args.id),
    snippets_create: (args: any) => {
      const id = state.snippets.length + 1;
      const created: MockSnippet = {
        id,
        sortOrder: 0,
        description: null,
        createdAt: "2026-07-27T00:00:00Z",
        updatedAt: "2026-07-27T00:00:00Z",
        ...args.data,
      };
      state.snippets.push(created);
      return created;
    },
    snippets_update: (args: any) => {
      const idx = state.snippets.findIndex((s) => s.id === args.id);
      if (idx >= 0) {
        state.snippets[idx] = { ...state.snippets[idx], ...args.data };
      }
      return state.snippets[idx];
    },
    snippets_remove: (args: any) => {
      state.snippets = state.snippets.filter((s) => s.id !== args.id);
    },

    hosts_list: () => [],
    groups_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    settings_set: () => {},
    ssh_active_sessions: () => [],
  };
}

test.describe("S12 — Snippets CRUD", () => {
  test("create, edit and delete a snippet", async ({ page }) => {
    const state = { snippets: [] as MockSnippet[] };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/snippets");

    // Пустой список
    await expect(page.locator("text=Hozircha snippet yo'q")).toBeVisible();

    // ---- Создание ----
    await page.fill('input[placeholder*="Disk"]', "Disk usage");
    await page.fill('textarea[placeholder*="df -h"]', "df -h");
    await page.click('button:has-text("Saqlash")');

    await expect(page.locator("text=Disk usage")).toBeVisible();
    await expect(page.locator("code", { hasText: "df -h" }).first()).toBeVisible();

    // Форма очистилась после сохранения
    await expect(page.locator('input[placeholder*="Disk"]')).toHaveValue("");

    // ---- Редактирование ----
    await page.locator("li", { hasText: "Disk usage" }).hover();
    await page.locator('li button[title="Tahrirlash"]').first().click();
    await expect(page.locator('input[placeholder*="Disk"]')).toHaveValue(
      "Disk usage",
    );

    await page.fill('input[placeholder*="Disk"]', "Disk and memory");
    await page.click('button:has-text("Saqlash")');
    await expect(page.locator("text=Disk and memory")).toBeVisible();

    // ---- Удаление ----
    await page.locator("li", { hasText: "Disk and memory" }).hover();
    await page.locator('li button[title="O\'chirish"]').first().click();
    await page.click('button:has-text("O\'chirish")');

    await expect(page.locator("text=Hozircha snippet yo'q")).toBeVisible();
  });

  test("run button is disabled without an active session", async ({ page }) => {
    const state = {
      snippets: [
        {
          id: 1,
          name: "Uptime",
          command: "uptime",
          description: null,
          sortOrder: 0,
          createdAt: "2026-07-27T00:00:00Z",
          updatedAt: "2026-07-27T00:00:00Z",
        },
      ] as MockSnippet[],
    };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/snippets");

    await expect(page.locator("text=Aktiv sessiya yo'q").first()).toBeVisible();
    await page.locator("li", { hasText: "Uptime" }).hover();
    await expect(page.getByTitle("Bajarish")).toBeDisabled();
  });

  test("AGENT snippet shows the full multiline command", async ({ page }) => {
    const command = "systemctl restart nginx\nsystemctl is-active nginx";
    const state = {
      snippets: [
        {
          id: 1,
          name: "Restart",
          command,
          description: null,
          agentAccess: true,
          sortOrder: 0,
          createdAt: "2026-07-27T00:00:00Z",
          updatedAt: "2026-07-27T00:00:00Z",
        },
      ] as MockSnippet[],
    };
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/snippets");

    const body = page.locator("li code").first();
    await expect(body).toHaveText(command);
    await expect(body).toHaveClass(/whitespace-pre-wrap/);
  });
});
