import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

/**
 * Форма хоста: пробелы по краям не уезжают в базу, а правка существующего
 * хоста не требует заново вводить пароль.
 */
const HOST = {
  id: 1, groupId: null, name: "prod", hostname: "10.0.0.7", port: 22, username: "root",
  identityId: null, authMethod: "password", color: null, icon: null, jumpHostId: null,
  forwardAgent: false, agentAccess: false, keepAliveInterval: 30, description: null,
  lastConnectedAt: null, createdAt: "2026-08-28T00:00:00Z", updatedAt: "2026-08-28T00:00:00Z",
};

async function setup(page: import("@playwright/test").Page, calls: any[]) {
  await mockTauriInvoke(page, {
    hosts_list: () => [HOST], groups_list: () => [], snippets_list: () => [],
    settings_all: () => ({}), settings_get: () => null, ssh_active_sessions: () => [],
    identities_list: () => [],
    hosts_update: (a: any) => { calls.push({ cmd: "update", ...a }); return { ...HOST, ...a.data }; },
    hosts_create: (a: any) => { calls.push({ cmd: "create", ...a }); return { ...HOST, id: 2, ...a.data }; },
  } as never);
}

test("правка существующего хоста: пароль не нужен, чтобы сменить название", async ({ page }) => {
  const calls: any[] = [];
  await setup(page, calls);
  await page.goto("/hosts/1/edit");
  await page.getByTestId("host-name-input").fill("prod-2");
  await page.getByTestId("host-form-submit").click();

  await expect.poll(() => calls.length).toBe(1);
  expect(calls[0].cmd).toBe("update");
  expect(calls[0].data.name).toBe("prod-2");
  // Пусто = оставить сохранённое: пароль не посылается.
  expect(calls[0].data.password).toBeNull();
  await expect(page.getByTestId("host-form-error")).toHaveCount(0);
});

test("создание без пароля по-прежнему требует пароль или identity", async ({ page }) => {
  const calls: any[] = [];
  await setup(page, calls);
  await page.goto("/hosts/new");
  await page.getByTestId("host-name-input").fill("new");
  await page.getByTestId("host-hostname-input").fill("10.0.0.9");
  await page.getByTestId("host-username-input").fill("root");
  await page.getByTestId("host-form-submit").click();
  await expect(page.getByTestId("host-form-error")).toBeVisible();
  expect(calls).toHaveLength(0);
});

test("пробелы по краям адреса, имени пользователя и названия обрезаются при сохранении", async ({ page }) => {
  const calls: any[] = [];
  await setup(page, calls);
  await page.goto("/hosts/new");
  await page.getByTestId("host-name-input").fill("  new  ");
  await page.getByTestId("host-hostname-input").fill(" 1.1.1.1 ");
  await page.getByTestId("host-username-input").fill(" root ");
  await page.getByTestId("host-password").fill("secret");
  await page.getByTestId("host-form-submit").click();

  await expect.poll(() => calls.length).toBe(1);
  expect(calls[0].data).toMatchObject({ name: "new", hostname: "1.1.1.1", username: "root" });
});
