import { test, expect } from "@playwright/test";
import { mockTauriInvoke, emitTauriEvent } from "./helpers";

const HOST = {
  id: 1,
  groupId: null,
  name: "bm-host",
  hostname: "10.0.0.3",
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

function makeHandlers() {
  return {
    hosts_list: () => [HOST],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],
    ssh_connect: () => ({ sessionId: "bm-session-1" }),
    ssh_disconnect: () => {},
    ssh_send: () => {},
    ssh_resize: () => {},
    master_status: () => ({
      enabled: false,
      unlocked: false,
      lockoutSecondsLeft: 0,
      failedAttempts: 0,
    }),
    recordings_list: () => [],
    recordings_dir: () => "/rec",
  };
}

test.describe("S18 — Terminal bookmarks", () => {
  test("adds a bookmark labelled with the output line", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers());
    await page.goto("/hosts");

    await page
      .getByTestId("host-row")
      .filter({ hasText: "bm-host" })
      .getByRole("button", { name: "Ulanish", exact: true })
      .click();
    await emitTauriEvent(page, "session:status", {
      event: "status",
      sessionId: "bm-session-1",
      status: "CONNECTED",
    });

    // Пишем строку в терминал тем же каналом, что и backend
    await emitTauriEvent(page, "session:data", {
      event: "data",
      sessionId: "bm-session-1",
      chunk: Array.from(new TextEncoder().encode("$ systemctl status nginx\r\n")),
    });

    // Панели ещё нет — закладок не было
    await expect(page.getByTestId("bookmarks-panel")).toHaveCount(0);

    await page.getByTestId("bookmark-add").click();

    const panel = page.getByTestId("bookmarks-panel");
    await expect(panel).toBeVisible();
    // Метка берётся из текста строки, а не из номера
    await expect(panel).toContainText("systemctl status nginx");
  });

  test("removes a bookmark", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers());
    await page.goto("/hosts");

    await page
      .getByTestId("host-row")
      .filter({ hasText: "bm-host" })
      .getByRole("button", { name: "Ulanish", exact: true })
      .click();
    await emitTauriEvent(page, "session:status", {
      event: "status",
      sessionId: "bm-session-1",
      status: "CONNECTED",
    });
    await emitTauriEvent(page, "session:data", {
      event: "data",
      sessionId: "bm-session-1",
      chunk: Array.from(new TextEncoder().encode("deploy finished\r\n")),
    });

    await page.getByTestId("bookmark-add").click();
    const panel = page.getByTestId("bookmarks-panel");
    await expect(panel).toBeVisible();

    await panel.locator("button").first().hover();
    await panel.locator('[role="button"]').first().click();

    // Последняя закладка удалена — панель скрывается
    await expect(page.getByTestId("bookmarks-panel")).toHaveCount(0);
  });
});
