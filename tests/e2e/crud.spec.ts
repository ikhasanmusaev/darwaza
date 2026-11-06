import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

interface MockIdentity {
  id: number;
  label: string;
  kind: string;
  keyPath: string | null;
  keyringRef: string | null;
  hasPassphrase: boolean;
  publicKey: string | null;
  createdAt: string;
  updatedAt: string;
}

interface MockHost {
  id: number;
  groupId: number | null;
  name: string;
  hostname: string;
  port: number;
  username: string;
  identityId: number | null;
  authMethod: string;
  color: string | null;
  icon: string | null;
  jumpHostId: number | null;
  keepAliveInterval: number;
  description: string | null;
  agentAccess?: boolean;
  agentSudoRead?: string | null;
  lastConnectedAt: string | null;
  createdAt: string;
  updatedAt: string;
}

function makeHandlers(state: {
  identities: MockIdentity[];
  hosts: MockHost[];
}) {
  return {
    identities_list: () => state.identities,
    identities_create: (args: any) => {
      const id = state.identities.length + 1;
      const created = { ...args.data, id, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() };
      state.identities.push(created);
      return created;
    },
    identities_get: (args: any) => state.identities.find((i) => i.id === args.id),
    identities_used_by: () => ["web-1"],
    identities_update: (args: any) => {
      const idx = state.identities.findIndex((i) => i.id === args.id);
      if (idx >= 0) state.identities[idx] = { ...state.identities[idx], ...args.data };
      return state.identities[idx];
    },
    identities_remove: (args: any) => {
      state.identities = state.identities.filter((i) => i.id !== args.id);
    },
    identity_set_password: () => {},
    identity_get_password: () => null,

    snippets_list: () => [],
    hosts_list: () => state.hosts,
    hosts_probe: (args: any) => args.hostIds.map((hostId: number) => ({ hostId, state: "unknown" })),
    hosts_get: (args: any) => state.hosts.find((h) => h.id === args.id),
    hosts_create: (args: any) => {
      const id = state.hosts.length + 1;
      const created = { ...args.data, id, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() };
      state.hosts.push(created);
      return created;
    },
    hosts_update: (args: any) => {
      const idx = state.hosts.findIndex((h) => h.id === args.id);
      if (idx >= 0) state.hosts[idx] = { ...state.hosts[idx], ...args.data };
      return state.hosts[idx];
    },
    hosts_remove: (args: any) => {
      state.hosts = state.hosts.filter((h) => h.id !== args.id);
    },

    groups_list: () => [],
    groups_create: () => ({ id: 1, name: "Test", parentId: null, sortOrder: 0, createdAt: "", updatedAt: "" }),
    groups_update: () => ({ id: 1, name: "Test", parentId: null, sortOrder: 0, createdAt: "", updatedAt: "" }),
    groups_remove: () => {},

    settings_all: () => ({}),
    settings_get: () => null,
    settings_set: () => {},
    settings_remove: () => {},

    ssh_config_preview: () => ({ path: "/home/user/.ssh/config", entries: [] }),

    ssh_connect: () => ({ sessionId: "test-session-1" }),
    ssh_disconnect: () => {},
    ssh_send: () => {},
    ssh_resize: () => {},
    ssh_active_sessions: () => [],
    ssh_accept_host_key: () => {},
    ssh_reject_host_key: () => {},

    known_hosts_list: () => [],
    known_hosts_remove: () => {},
    known_hosts_clear: () => {},
  };
}

test.describe("S2-S3 — Create identity + host", () => {
  test("full crud flow", async ({ page }) => {
    const state = { identities: [] as MockIdentity[], hosts: [] as MockHost[] };
    await mockTauriInvoke(page, makeHandlers(state));

    // ---- S2: Create identity ----
    // Keychain переехал на top-level /identities (#081)
    await page.goto("/identities/new");

    await page.fill('input[type="text"]', "Test Key");
    // Select private_key radio
    await page.check('input[value="private_key"]');
    await page.fill('input[placeholder="~/.ssh/id_rsa"]', "~/.ssh/id_test");

    await page.click('button[type="submit"]');

    // Should redirect back to identities list
    await expect(page).toHaveURL(/\/identities$/);
    await expect(page.locator("text=Test Key")).toBeVisible();

    // ---- S3: Create host using the identity ----
    await page.goto("/hosts/new");

    await page.fill('input[placeholder*="Masalan"]', "test-server");
    await page.fill('input[placeholder*="192.168"]', "192.168.1.100");
    await page.fill('input[placeholder*="ubuntu"]', "ubuntu");

    // Select publickey auth
    await page.check('input[value="publickey"]');

    // Select identity from dropdown.
    // На форме два <select> (группа и identity) — адресуемся точечно.
    await page
      .getByTestId("host-identity-select")
      .selectOption({ label: "Test Key" });

    await page.click('button[type="submit"]');

    // Should redirect to hosts list with new host visible.
    // Имя хоста дублируется в дереве сайдбара — проверяем строку таблицы.
    await expect(page).toHaveURL(/hosts/);
    const card = page.getByTestId("host-row").filter({ hasText: "test-server" });
    await expect(card).toHaveCount(1);
    await expect(card).toContainText("192.168.1.100");
  });

  test("перед удалением ключа видно, какие хосты останутся без входа", async ({ page }) => {
    const state = {
      identities: [
        {
          id: 1,
          label: "deploy key",
          kind: "private_key",
          keyPath: null,
          keyringRef: null,
          hasPassphrase: false,
          publicKey: null,
          createdAt: "2026-07-27T00:00:00Z",
          updatedAt: "2026-07-27T00:00:00Z",
        },
      ] as MockIdentity[],
      hosts: [] as MockHost[],
    };
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/identities");
    await page.getByTitle("O'chirish").click();

    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toContainText("1 ta host");
    await expect(dialog).toContainText("web-1");
  });

  /**
   * Регрессия: подтверждение в ConfirmDialog не доходило до обработчика —
   * radix-вский AlertDialogAction закрывал диалог раньше, родитель успевал
   * обнулить `hostToDelete`, и `hosts_remove` не вызывался вообще.
   */
  test("delete host actually calls the backend", async ({ page }) => {
    const state = {
      identities: [] as MockIdentity[],
      hosts: [
        {
          id: 1,
          groupId: null,
          name: "doomed-host",
          hostname: "10.0.0.9",
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
          createdAt: "2026-07-27T00:00:00Z",
          updatedAt: "2026-07-27T00:00:00Z",
        },
      ] as MockHost[],
    };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/hosts");
    // Имя хоста есть и в дереве сайдбара, и в списке — смотрим только на список.
    const listedHost = page.getByTestId("host-row").filter({ hasText: "doomed-host" });
    await expect(listedHost).toHaveCount(1);

    await listedHost.getByTestId("host-actions-menu").click();
    await listedHost.getByTitle("O'chirish").click();
    await page.getByRole("alertdialog").getByRole("button", { name: "O'chirish", exact: true }).click();

    await expect(listedHost).toHaveCount(0);
  });
});

test.describe("S214 — читающие команды с sudo для агента", () => {
  test("список появляется вместе с доступом агента и доезжает до backend", async ({
    page,
  }) => {
    const state = { identities: [] as MockIdentity[], hosts: [] as MockHost[] };
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/hosts/new");

    // Поле прячется, пока доступ агента выключен: разрешать команды серверу,
    // к которому агента не пускают, незачем.
    await expect(page.getByTestId("host-agent-sudo")).toBeHidden();
    await page.getByTestId("host-agent-access").click();
    await page.getByRole("button", { name: "Yoqish", exact: true }).click();
    await expect(page.getByTestId("host-agent-sudo")).toBeVisible();

    await page.fill('input[placeholder*="Masalan"]', "web-1");
    await page.fill('input[placeholder*="192.168"]', "10.0.0.5");
    await page.fill('input[placeholder*="ubuntu"]', "root");
    // Без пароля или ключа форма не сохранится — проверка не про это.
    await page.fill('input[type="password"]', "s3cret");
    await page.getByTestId("host-agent-sudo").fill("ufw status\nss -tlnp");
    await page.click('button[type="submit"]');

    await expect(page).toHaveURL(/\/hosts$/);
    expect(state.hosts).toHaveLength(1);
    expect(state.hosts[0].agentSudoRead).toBe("ufw status\nss -tlnp");
    expect(state.hosts[0].agentAccess).toBe(true);
  });

  test("флажок AGENT требует подтверждения, а выключается сразу", async ({ page }) => {
    const state = {
      identities: [] as MockIdentity[],
      hosts: [
        {
          id: 1,
          groupId: null,
          name: "agent-host",
          hostname: "10.0.0.10",
          port: 22,
          username: "root",
          identityId: null,
          authMethod: "password",
          color: null,
          icon: "server-icon",
          jumpHostId: null,
          keepAliveInterval: 30,
          description: null,
          agentAccess: false,
          lastConnectedAt: null,
          createdAt: "2026-07-27T00:00:00Z",
          updatedAt: "2026-07-27T00:00:00Z",
        },
      ] as MockHost[],
    };
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/hosts");

    const row = page.getByTestId("host-row");
    await expect(row).toContainText("noma'lum");
    const checkbox = row.getByTestId("host-agent-access-row");
    await checkbox.click();
    await expect(page.getByRole("alertdialog")).toBeVisible();
    expect(state.hosts[0].agentAccess).toBe(false);

    await page.getByRole("button", { name: "Bekor qilish", exact: true }).click();
    await expect(checkbox).not.toBeChecked();
    await checkbox.click();
    await page.getByRole("button", { name: "Yoqish", exact: true }).click();
    await expect.poll(() => state.hosts[0].agentAccess).toBe(true);

    await checkbox.uncheck();
    await expect.poll(() => state.hosts[0].agentAccess).toBe(false);
    expect(state.hosts[0].icon).toBe("server-icon");
    await expect(page.getByRole("alertdialog")).toHaveCount(0);
  });
});
