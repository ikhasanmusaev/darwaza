import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

/**
 * Выбранная идентификация обязана остаться выбранной при открытии правки.
 *
 * Форма грузит хост и список идентификаций одновременно. Сторож, убирающий
 * недействительный выбор, срабатывал раньше, чем приезжал список: список ещё
 * пуст — значит выбор «недействителен» — значит стереть. Человек открывал
 * правку и видел пустое поле там, где идентификация выбрана, а сохранение
 * оставляло хост без неё.
 */
const IDENTITY = {
  id: 4,
  label: "hetzner",
  kind: "password",
  keyPath: null,
  keyringRef: null,
  hasPassphrase: false,
  hasTotp: false,
  publicKey: null,
  createdAt: "2026-08-28T00:00:00Z",
  updatedAt: "2026-08-28T00:00:00Z",
};

const HOST = {
  id: 1,
  groupId: null,
  name: "hetzner",
  hostname: "10.0.0.7",
  port: 22,
  username: "root",
  identityId: 4,
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

test("правка хоста показывает выбранную идентификацию", async ({ page }) => {
  await mockTauriInvoke(page, {
    hosts_list: () => [HOST],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],
    // Список приезжает ПОЗЖЕ хоста — ровно тот промежуток, в который
    // попадала форма.
    identities_list: async () => {
      await new Promise((resolve) => setTimeout(resolve, 80));
      return [IDENTITY];
    },
  });

  await page.goto("/hosts/1/edit");

  const select = page.locator('[data-testid="host-identity-select"]');
  await expect(select).toBeVisible();
  // Хост обязан найтись: пустая форма на существующем хосте — та же ошибка
  // с другой стороны.
  await expect(page.getByText("Host topilmadi")).toHaveCount(0);
  await expect(select).toHaveValue("4");
});
