import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

const SSH_CONFIG_PREVIEW = {
  path: "/home/user/.ssh/config",
  entries: [
    { alias: "web1", hostname: "10.0.0.1", user: "ubuntu", port: 22, identityFile: "~/.ssh/id_ed25519", proxyJump: null },
    { alias: "db1", hostname: "db.internal", user: "root", port: null, identityFile: null, proxyJump: null },
  ],
};

test.describe("S8 — Import ~/.ssh/config", () => {
  test("preview and bulk import", async ({ page }) => {
    const hosts: any[] = [];

    await mockTauriInvoke(page, {
      snippets_list: () => [],
      hosts_list: () => hosts,
      hosts_create: (args: any) => {
        const id = hosts.length + 1;
        const created = { ...args.data, id, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() };
        hosts.push(created);
        return created;
      },
      groups_list: () => [],
      settings_all: () => ({}),
      ssh_config_preview: () => SSH_CONFIG_PREVIEW,
    });

    await page.goto("/hosts");

    // Open import modal
    await page.getByRole("button", { name: "~/.ssh/config dan import" }).click();

    // Modal should show preview entries
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByText("web1")).toBeVisible();
    await expect(dialog.getByText("db1")).toBeVisible();

    // Both entries checked by default — click import.
    // Кнопка подписана локалью (uz по умолчанию), а не английским текстом.
    await dialog
      .getByRole("button", { name: "Tanlanganlarni import qilish" })
      .click();

    // Modal closes, hosts appear in list.
    // Имена хостов есть и в дереве сайдбара — смотрим на строки таблицы.
    await expect(page.getByTestId("host-row").filter({ hasText: "web1" })).toHaveCount(1);
    await expect(page.getByTestId("host-row").filter({ hasText: "db1" })).toHaveCount(1);
  });
});
