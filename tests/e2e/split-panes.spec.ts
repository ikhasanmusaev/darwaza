import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

function host(id: number, name: string) {
  return {
    id,
    groupId: null,
    name,
    hostname: `10.0.0.${id}`,
    port: 22,
    username: "root",
    identityId: null,
    authMethod: "password",
    color: null,
    icon: null,
    jumpHostId: null,
    forwardAgent: false,
    keepAliveInterval: 30,
    description: null,
    lastConnectedAt: null,
    createdAt: "2026-08-18T00:00:00Z",
    updatedAt: "2026-08-18T00:00:00Z",
  };
}

function makeHandlers() {
  let n = 0;
  return {
    hosts_list: () => [host(1, "alpha"), host(2, "beta")],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],
    ssh_connect: () => ({ sessionId: `split-session-${++n}` }),
    ssh_disconnect: () => {},
    ssh_send: () => {},
    ssh_resize: () => {},
    master_status: () => ({
      enabled: false,
      unlocked: false,
      lockoutSecondsLeft: 0,
      failedAttempts: 0,
    }),
  };
}

/** Открыть сессию к хосту и дождаться вкладки. */
async function openHost(page: any, name: string, expectTabs: number) {
  await page.getByTestId("library-tab").click();
  await page
    .getByTestId("host-row")
    .filter({ hasText: name })
    .getByRole("button", { name: "Ulanish", exact: true })
    .click();
  await expect(page.getByTestId("session-tab")).toHaveCount(expectTabs);
}

test.describe("S19 — Split panes", () => {
  test("ctrl+click shows two terminals side by side", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers());
    await page.goto("/hosts");

    await openHost(page, "alpha", 1);
    await openHost(page, "beta", 2);

    // Обычный режим: виден один терминал
    const terminals = page.locator(".xterm");
    await expect(terminals).toHaveCount(2); // смонтированы обе, видима одна
    await expect(terminals.nth(0)).toBeHidden();
    await expect(terminals.nth(1)).toBeVisible();

    // Ctrl+клик по первой вкладке — показать её рядом
    await page.getByTestId("session-tab").first().click({ modifiers: ["Control"] });

    await expect(terminals.nth(0)).toBeVisible();
    await expect(terminals.nth(1)).toBeVisible();

    // Появляется переключатель направления
    await expect(page.getByTestId("split-direction")).toBeVisible();
  });

  test("plain click returns to a single pane", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers());
    await page.goto("/hosts");

    await openHost(page, "alpha", 1);
    await openHost(page, "beta", 2);
    await page.getByTestId("session-tab").first().click({ modifiers: ["Control"] });
    await expect(page.getByTestId("split-direction")).toBeVisible();

    // Обычный клик схлопывает раскладку обратно
    await page.getByTestId("session-tab").nth(1).click();
    await expect(page.getByTestId("split-direction")).toHaveCount(0);

    const terminals = page.locator(".xterm");
    await expect(terminals.nth(0)).toBeHidden();
    await expect(terminals.nth(1)).toBeVisible();
  });

  test("closing a split session removes its pane", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers());
    await page.goto("/hosts");

    await openHost(page, "alpha", 1);
    await openHost(page, "beta", 2);
    await page.getByTestId("session-tab").first().click({ modifiers: ["Control"] });
    await expect(page.getByTestId("split-direction")).toBeVisible();

    // Закрываем одну из сессий — раскладка должна схлопнуться,
    // иначе на её месте остался бы пустой прямоугольник.
    const tab = page.getByTestId("session-tab").first();
    await tab.hover();
    await tab.locator('[role="button"]').click();

    await expect(page.getByTestId("session-tab")).toHaveCount(1);
    await expect(page.getByTestId("split-direction")).toHaveCount(0);
  });
});
