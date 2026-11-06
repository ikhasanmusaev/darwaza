import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

const PREVIEW = {
  path: "/home/tester/.ssh/known_hosts",
  items: [
    {
      hostname: "new.example.com:22",
      keyType: "ssh-ed25519",
      fingerprint: "SHA256:aaaaNEW",
      alreadyKnown: false,
      conflicts: false,
    },
    {
      hostname: "known.example.com:22",
      keyType: "ssh-ed25519",
      fingerprint: "SHA256:bbbSAME",
      alreadyKnown: true,
      conflicts: false,
      currentFingerprint: "SHA256:bbbSAME",
    },
    {
      hostname: "changed.example.com:2222",
      keyType: "ssh-rsa",
      fingerprint: "SHA256:cccNEW",
      alreadyKnown: false,
      conflicts: true,
      currentFingerprint: "SHA256:cccOLD",
    },
  ],
  skippedHashed: 4,
  skippedWildcard: 1,
  skippedMarker: 0,
  skippedUnparsable: 2,
};

function makeHandlers(state: { imported: any[] }) {
  return {
    hosts_list: () => [],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],
    master_status: () => ({
      enabled: false,
      unlocked: false,
      lockoutSecondsLeft: 0,
      failedAttempts: 0,
    }),
    known_hosts_list: () => [],
    known_hosts_import_preview: () => PREVIEW,
    known_hosts_import: (args: any) => {
      state.imported = args.entries;
      return args.entries.length;
    },
  };
}

test.describe("S17 — known_hosts import", () => {
  test("preselects only new entries, not conflicting ones", async ({ page }) => {
    const state = { imported: [] as any[] };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/settings/known-hosts");
    await page.getByTestId("known-hosts-import").click();

    const list = page.getByTestId("known-hosts-import-list");
    await expect(list).toBeVisible();

    const boxes = list.locator('input[type="checkbox"]');
    // Новая — отмечена; уже известная — снята и заблокирована;
    // конфликтующая — намеренно НЕ отмечена, её импорт перезапишет ключ TOFU.
    await expect(boxes.nth(0)).toBeChecked();
    await expect(boxes.nth(1)).not.toBeChecked();
    await expect(boxes.nth(1)).toBeDisabled();
    await expect(boxes.nth(2)).not.toBeChecked();

    // Пропущенные записи посчитаны и показаны
    await expect(page.getByText(/7/)).toBeVisible();

    await page.getByTestId("known-hosts-import-run").click();
    await expect.poll(() => state.imported.length).toBe(1);
    expect(state.imported[0].hostname).toBe("new.example.com:22");
  });

  test("conflicting entry can be imported deliberately", async ({ page }) => {
    const state = { imported: [] as any[] };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/settings/known-hosts");
    await page.getByTestId("known-hosts-import").click();

    const boxes = page
      .getByTestId("known-hosts-import-list")
      .locator('input[type="checkbox"]');
    await boxes.nth(2).check();
    await page.getByTestId("known-hosts-import-run").click();

    await expect.poll(() => state.imported.length).toBe(2);
  });
});
