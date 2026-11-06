import { test, expect } from "@playwright/test";
import { mockTauriInvoke, emitTauriEvent } from "./helpers";

/**
 * Причина сбоя подключения обязана остаться на экране.
 *
 * Backend при сбое шлёт два события подряд: `FAILED` с текстом причины и
 * следом `closed`. Второе затирало первое, и человек видел «Отключено» без
 * единого слова о том, что произошло, — при том что в журнале причина была.
 */
const HOST = {
  id: 1,
  groupId: null,
  name: "fail-host",
  hostname: "10.0.0.9",
  port: 22,
  username: "root",
  identityId: null,
  authMethod: "password",
  color: null,
  icon: null,
  jumpHostId: null,
  forwardAgent: false,
  agentAccess: false,
  keepAliveInterval: 30,
  description: null,
  lastConnectedAt: null,
  createdAt: "2026-08-28T00:00:00Z",
  updatedAt: "2026-08-28T00:00:00Z",
};

const handlers = {
  hosts_list: () => [HOST],
  groups_list: () => [],
  snippets_list: () => [],
  settings_all: () => ({}),
  settings_get: () => null,
  ssh_active_sessions: () => [],
  ssh_connect: () => ({ sessionId: "fail-session-1" }),
  ssh_disconnect: () => {},
  master_status: () => ({
    enabled: false,
    unlocked: false,
    lockoutSecondsLeft: 0,
    failedAttempts: 0,
  }),
};

test.describe("Сбой подключения", () => {
  test("причина остаётся видна после закрытия сессии", async ({ page }) => {
    await mockTauriInvoke(page, handlers);
    await page.goto("/hosts");

    await page
      .getByTestId("host-row")
      .filter({ hasText: "fail-host" })
      .getByRole("button", { name: "Ulanish", exact: true })
      .click();

    await emitTauriEvent(page, "session:status", {
      event: "status",
      sessionId: "fail-session-1",
      status: "FAILED",
      error: "errors.sshAuthFailed",
    });
    // Так делает backend: сбой закрывает сессию, и `closed` идёт следом.
    await emitTauriEvent(page, "session:closed", {
      event: "closed",
      sessionId: "fail-session-1",
    });

    // Причина на экране, а не «Отключено» без объяснений.
    await expect(page.getByText("Ulanish xatosi")).toBeVisible();
    await expect(
      // Backend отдаёт код, человек видит объяснение на своём языке (#178).
      page.getByText("Server hisob ma’lumotlarini qabul qilmadi", { exact: false }),
    ).toBeVisible();
  });
});
