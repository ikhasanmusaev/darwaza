import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

/** Двойной клик и Enter на строке хоста подключают; одиночный клик, флажок AGENT и «…» — нет. */
function host(id: number, name: string) {
  return {
    id, groupId: null, name, hostname: `10.0.0.${id}`, port: 22, username: "root",
    identityId: null, authMethod: "password", color: null, icon: null, jumpHostId: null,
    forwardAgent: false, keepAliveInterval: 30, description: null, lastConnectedAt: null,
    agentAccess: false, createdAt: "2026-08-18T00:00:00Z", updatedAt: "2026-08-18T00:00:00Z",
  };
}

async function setup(page: import("@playwright/test").Page) {
  const calls: string[] = [];
  let n = 0;
  await mockTauriInvoke(page, {
    hosts_list: () => [host(1, "alpha")], groups_list: () => [], snippets_list: () => [],
    settings_all: () => ({}), settings_get: () => null, ssh_active_sessions: () => [],
    ssh_connect: () => { calls.push("connect"); return { sessionId: `row-${++n}` }; },
    ssh_disconnect: () => {}, ssh_latency: () => 12, ssh_send: () => {}, ssh_resize: () => {},
    hosts_update: () => host(1, "alpha"),
    master_status: () => ({ enabled: false, unlocked: false, lockoutSecondsLeft: 0, failedAttempts: 0, minLength: 12 }),
  } as never);
  await page.goto("/hosts");
  return calls;
}

test.describe("подключение по строке хоста", () => {
  test("двойной клик по ячейке Host подключает, одиночный — нет", async ({ page }) => {
    const calls = await setup(page);
    const row = page.getByTestId("host-row").first();
    await row.locator("td").first().click();
    await page.waitForTimeout(300);
    expect(calls).toHaveLength(0);
    await row.locator("td").first().dblclick();
    await expect.poll(() => calls.length).toBe(1);
  });

  test("флажок AGENT и кнопки действий не подключают", async ({ page }) => {
    const calls = await setup(page);
    const row = page.getByTestId("host-row").first();
    await row.getByTestId("host-agent-access-row").dispatchEvent("dblclick");
    // Иконки действий: «войти» не трогаем (она и должна подключать), «…» — последняя.
    await row.getByRole("button").last().dispatchEvent("dblclick");
    await page.waitForTimeout(400);
    expect(calls).toHaveLength(0);
  });

  test("Enter на сфокусированной строке подключает", async ({ page }) => {
    const calls = await setup(page);
    await page.getByTestId("host-row").first().focus();
    await page.keyboard.press("Enter");
    await expect.poll(() => calls.length).toBe(1);
  });
});
