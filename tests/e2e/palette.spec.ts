import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

function makeHandlers() {
  return {
    snippets_list: () => [],
    hosts_list: () => [
      { id: 1, name: "Web Server", hostname: "web.local", port: 22, username: "root", groupId: null, identityId: null, authMethod: "password", color: null, icon: null, jumpHostId: null, keepAliveInterval: 60, description: null, lastConnectedAt: null, createdAt: "", updatedAt: "" },
      { id: 2, name: "DB Server", hostname: "db.local", port: 22, username: "postgres", groupId: null, identityId: null, authMethod: "password", color: null, icon: null, jumpHostId: null, keepAliveInterval: 60, description: null, lastConnectedAt: null, createdAt: "", updatedAt: "" },
    ],
    identities_list: () => [
      { id: 1, label: "Main Key", kind: "private_key", keyPath: "~/.ssh/id_rsa", keyringRef: null, hasPassphrase: false, publicKey: null, createdAt: "", updatedAt: "" },
      { id: 2, label: "Work Password", kind: "password", keyPath: null, keyringRef: null, hasPassphrase: false, publicKey: null, createdAt: "", updatedAt: "" },
    ],
    settings_all: () => ({}),
    settings_get: () => null,
    settings_set: () => {},
    groups_list: () => [],
    known_hosts_list: () => [],
    ssh_connect: () => ({ sessionId: "test-session-1" }),
    ssh_disconnect: () => {},
    ssh_send: () => {},
    ssh_resize: () => {},
    ssh_active_sessions: () => [],
    ssh_accept_host_key: () => {},
    ssh_reject_host_key: () => {},
    identity_set_password: () => {},
    identity_get_password: () => null,
  };
}

test.describe("S11 — Command Palette", () => {
  test("opens with Ctrl+K, shows hosts/identities/actions, filters and navigates", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers());
    await page.goto("/hosts");

    // Ensure host list loaded so stores are hydrated.
    // Имя хоста есть и в дереве сайдбара — смотрим на строку таблицы.
    await expect(
      page.getByTestId("host-row").filter({ hasText: "Web Server" }),
    ).toHaveCount(1);

    // Open palette
    await page.keyboard.press("Control+k");

    // Wait for palette dialog input (inside Radix Dialog content)
    const input = page.locator('[role="dialog"] input[type="text"]');
    await expect(input).toBeVisible();

    // Focus should be on palette input (Radix focuses first focusable element)
    await expect(input).toBeFocused();

    // Check hosts visible inside dialog
    await expect(page.locator('[role="dialog"] >> text=Web Server')).toBeVisible();
    await expect(page.locator('[role="dialog"] >> text=DB Server')).toBeVisible();

    // Check identities visible
    await expect(page.locator('[role="dialog"] >> text=Main Key')).toBeVisible();
    await expect(page.locator('[role="dialog"] >> text=Work Password')).toBeVisible();

    // Filter: type "db"
    await input.fill("db");
    await expect(page.locator('[role="dialog"] >> text=DB Server')).toBeVisible();
    await expect(page.locator('[role="dialog"] >> text=Web Server')).not.toBeVisible();

    // Clear filter
    await input.fill("");

    // Navigate down twice to select "Main Key" identity (index 2: Web=0, DB=1, Main Key=2)
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");

    // Should navigate to identity edit page
    // Keychain переехал на top-level /identities (#081)
    await expect(page).toHaveURL(/\/identities\/1\/edit/);

    // Reopen and close with Escape
    await page.keyboard.press("Control+k");
    await expect(input).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(input).not.toBeVisible();
  });
});
