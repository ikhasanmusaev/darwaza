import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

const HOST = {
  id: 1,
  groupId: null,
  name: "prod-db",
  hostname: "10.0.0.7",
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
};

function makeHandlers(state: { forwards: any[]; started: string[] }) {
  return {
    hosts_list: () => [HOST],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],

    port_forwards_list: () => state.forwards,
    port_forwards_active: () => [],
    port_forwards_create: (args: any) => {
      const created = {
        id: state.forwards.length + 1,
        localHost: "127.0.0.1",
        remoteHost: null,
        remotePort: null,
        label: null,
        autoStart: false,
        createdAt: "x",
        updatedAt: "x",
        ...args.data,
      };
      state.forwards.push(created);
      return created;
    },
    port_forwards_remove: (args: any) => {
      state.forwards = state.forwards.filter((f) => f.id !== args.id);
    },
    port_forward_start: (args: any) => {
      state.started.push(String(args.id));
      return "127.0.0.1:5432";
    },
    port_forward_stop: () => true,
  };
}

test.describe("S14 — Port forwarding", () => {
  test("creates a local rule and requires a live session to start it", async ({
    page,
  }) => {
    const state = { forwards: [] as any[], started: [] as string[] };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/forwards");
    await page.getByRole("button", { name: "Yangi qoida", exact: true }).click();
    await expect(page.getByText("Hozircha qoida yo'q")).toBeVisible();
    await expect(page.getByTestId("forward-local-host")).toHaveValue("127.0.0.1");
    await expect(page.getByTestId("forward-network-warning")).toBeHidden();
    await page.getByTestId("forward-local-host").fill("0.0.0.0");
    await expect(page.getByTestId("forward-network-warning")).toBeVisible();
    await page.getByTestId("forward-local-host").fill("127.0.0.1");
    await expect(page.getByTestId("forward-network-warning")).toBeHidden();

    // Создаём local-правило
    await page.getByTestId("forward-local-port").fill("15432");
    await page.getByTestId("forward-remote-host").fill("127.0.0.1");
    await page.getByTestId("forward-remote-port").fill("5432");
    await page.getByRole("button", { name: "Saqlash" }).click();

    const row = page.getByTestId("forwards-table").locator("tbody tr").filter({ hasText: "15432" });
    await expect(row).toHaveCount(1);

    // Без активной сессии запуск невозможен — проброс живёт поверх неё
    await row.hover();
    await row.getByTitle("Ishga tushirish").click();
    await expect(page.getByText("Bu hostga aktiv ulanish kerak")).toBeVisible();
    expect(state.started).toHaveLength(0);
  });

  test("dynamic rule hides destination fields", async ({ page }) => {
    const state = { forwards: [] as any[], started: [] as string[] };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/forwards");
    await page.getByRole("button", { name: "Yangi qoida", exact: true }).click();
    // У SOCKS5 адрес назначения выбирает клиент, поля не нужны
    await expect(page.getByTestId("forward-remote-host")).toBeVisible();
    await page.getByTestId("forward-kind-select").selectOption("dynamic");
    await expect(page.getByTestId("forward-remote-host")).toHaveCount(0);
  });
});
