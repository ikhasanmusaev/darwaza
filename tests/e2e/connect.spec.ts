import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

const MOCK_HOST_1 = {
  id: 1,
  groupId: null,
  name: "web-1",
  hostname: "10.0.0.1",
  port: 22,
  username: "ubuntu",
  identityId: null,
  authMethod: "password",
  color: null,
  icon: null,
  jumpHostId: null,
  keepAliveInterval: 30,
  description: null,
  lastConnectedAt: null,
  createdAt: "2026-05-13T00:00:00Z",
  updatedAt: "2026-05-13T00:00:00Z",
};

const MOCK_HOST_2 = {
  id: 2,
  groupId: null,
  name: "db-1",
  hostname: "10.0.0.2",
  port: 22,
  username: "root",
  identityId: null,
  authMethod: "password",
  color: null,
  icon: null,
  jumpHostId: null,
  keepAliveInterval: 30,
  description: null,
  lastConnectedAt: null,
  createdAt: "2026-05-13T00:00:00Z",
  updatedAt: "2026-05-13T00:00:00Z",
};

test.describe("S4-S5 — Connect opens a session tab", () => {
  test("click Connect creates a tab with CONNECTING status", async ({ page }) => {
    let sessionCounter = 0;

    await mockTauriInvoke(page, {
      snippets_list: () => [],
      hosts_list: () => [MOCK_HOST_1],
      groups_list: () => [],
      settings_all: () => ({}),
      ssh_connect: () => {
        sessionCounter++;
        return { sessionId: `test-session-${sessionCounter}` };
      },
      ssh_disconnect: () => {},
      ssh_send: () => {},
      ssh_resize: () => {},
      ssh_active_sessions: () => [],
    });

    await page.goto("/hosts");

    // Имя хоста встречается ещё и в дереве сайдбара, поэтому смотрим на строку таблицы.
    const card = page.getByTestId("host-row").filter({ hasText: "web-1" });
    await expect(card).toHaveCount(1);

    // Вкладок сессий пока нет
    await expect(page.getByTestId("session-tab")).toHaveCount(0);

    // Click connect (Monitor icon button)
    await card.getByRole("button", { name: "Ulanish", exact: true }).click();

    // A new tab appears
    const tabs = page.getByTestId("session-tab");
    await expect(tabs).toHaveCount(1);
    await expect(tabs.first()).toContainText("web-1");

    // Tab has amber spinner (CONNECTING status).
    // Ищем спиннер именно во вкладке: в самом терминале крутится ещё один.
    await expect(tabs.first().locator(".animate-spin")).toBeVisible();
  });
});

test.describe("S6-S7 — Multiple tabs + close", () => {
  test("open two tabs, then close one", async ({ page }) => {
    let sessionCounter = 0;

    await mockTauriInvoke(page, {
      snippets_list: () => [],
      hosts_list: () => [MOCK_HOST_1, MOCK_HOST_2],
      groups_list: () => [],
      settings_all: () => ({}),
      ssh_connect: () => {
        sessionCounter++;
        return { sessionId: `test-session-${sessionCounter}` };
      },
      ssh_disconnect: () => {},
      ssh_send: () => {},
      ssh_resize: () => {},
      ssh_active_sessions: () => [],
    });

    await page.goto("/hosts");

    const cards = page.getByTestId("host-row");
    const tabs = page.getByTestId("session-tab");

    // Open first host
    await cards.filter({ hasText: "web-1" }).getByRole("button", { name: "Ulanish", exact: true }).click();
    await expect(tabs).toHaveCount(1);

    // Открытие сессии переключает главную область на терминал — чтобы добраться
    // до списка хостов, возвращаемся во вкладку Library.
    await page.getByTestId("library-tab").click();

    // Open second host
    await cards.filter({ hasText: "db-1" }).getByRole("button", { name: "Ulanish", exact: true }).click();
    await expect(tabs).toHaveCount(2);

    // Закрываем вкладку web-1: делаем её активной и жмём крестик внутри неё
    const webTab = tabs.filter({ hasText: "web-1" });
    await webTab.click();
    await webTab.getByRole("button", { name: "Yopish" }).click();

    // Остаётся только db-1
    await expect(tabs).toHaveCount(1);
    await expect(tabs.first()).toContainText("db-1");
  });
});
