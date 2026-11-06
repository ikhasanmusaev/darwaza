import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

function makeHandlers(state: any) {
  return {
    hosts_list: () => [],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],
    onboarding_needed: () => true,
    onboarding_done: () => {
      state.done = true;
    },
    master_status: () => ({
      enabled: false,
      unlocked: false,
      lockoutSecondsLeft: 0,
      failedAttempts: 0,
      totpEnabled: false,
      recoveryKeySet: false,
      minLength: 12,
      kdf: { memoryKib: 65536, iterations: 3, parallelism: 1 },
    }),
    secrets_place: () => ({
      keyringAvailable: true,
      masterEnabled: false,
      masterUnlocked: false,
      target: "keyring",
    }),
    master_enable: () => 0,
    master_recovery_issue: () => "ABCD-EFGH-IJKM-NPQR-STVW-XYZA-BCDE-FGHI",
    sync_status: () => ({
      endpoint: "https://api.darwaza.uz/api/v1",
      email: null,
      signedIn: false,
      pending: 0,
      lastError: null,
      lastSyncAt: null,
      cursor: 0,
    }),
    sync_register: (args: any) => {
      state.registered = args;
      return "ABCD-EFGH-IJKM-NPQR-STVW-XYZA-BCDE-FGHI";
    },
  };
}

async function finishRecovery(page: any) {
  const done = page.getByTestId("recovery-key-done");
  await expect(done).toBeDisabled();
  await page.getByTestId("recovery-key-saved").check();
  await expect(done).toBeDisabled();
  await page.getByTestId("recovery-key-understood").check();
  await expect(done).toBeEnabled();
  await done.click();
}

test("first run can explicitly continue without a master password", async ({ page }) => {
  const state: any = {};
  await mockTauriInvoke(page, makeHandlers(state));
  await page.goto("/hosts");

  await expect(page.getByTestId("onboarding-gate")).toBeVisible();
  await page.getByTestId("onboarding-local").click();
  await page.getByTestId("onboarding-without-master").click();

  await expect.poll(() => state.done).toBe(true);
  await expect(page.getByTestId("onboarding-gate")).toHaveCount(0);
});

test("account registration shows a recovery key and completes only after both confirmations", async ({ page }) => {
  const state: any = {};
  await mockTauriInvoke(page, makeHandlers(state));
  await page.goto("/hosts");

  await page.getByTestId("onboarding-account").click();
  await page.getByTestId("onboarding-email").fill("person@example.org");
  await page.getByTestId("onboarding-account-password").fill("account-password");
  await page.getByTestId("onboarding-register").click();

  await expect.poll(() => state.registered.email).toBe("person@example.org");
  await expect(page.getByTestId("recovery-key-value")).toContainText("ABCD");
  await finishRecovery(page);
  await expect.poll(() => state.done).toBe(true);
});

test("optional master uses the 12-character rule and real KDF status", async ({ page }) => {
  const state: any = {};
  await mockTauriInvoke(page, makeHandlers(state));
  await page.goto("/hosts");

  await page.getByTestId("onboarding-local").click();
  await expect(page.getByText("Argon2id · 64 MiB · 3 o‘tish · parallelism 1")).toBeVisible();
  await page.getByTestId("onboarding-master-password").fill("short");
  await page.getByTestId("onboarding-master-repeat").fill("short");
  await expect(page.getByTestId("onboarding-master-enable")).toBeDisabled();
  await page.getByTestId("onboarding-master-password").fill("long-enough-password");
  await page.getByTestId("onboarding-master-repeat").fill("long-enough-password");
  await page.getByTestId("onboarding-master-enable").click();
  await finishRecovery(page);
  await expect.poll(() => state.done).toBe(true);
});

test("settings list system fonts and explain DNS for the host probe", async ({ page }) => {
  const state: any = { settings: {} };
  await mockTauriInvoke(page, {
    ...makeHandlers(state),
    onboarding_needed: () => false,
    system_fonts: () => ["JetBrains Mono", "IBM Plex Mono"],
    settings_set: (args: any) => {
      state.settings[args.key] = args.value;
    },
  });
  await page.goto("/settings");
  await page.getByRole("button", { name: "Terminal", exact: true }).click();
  await expect(page.locator("#system-fonts option")).toHaveCount(2);
  await page.getByRole("button", { name: "SSH", exact: true }).click();
  await expect(page.getByText(/DNS resolver/)).toBeVisible();
  await page.getByTestId("hosts-probe").check();
  await expect.poll(() => state.settings["hosts.probe"]).toBe("1");
  await expect(page.getByTestId("decision-notify")).toBeChecked();
  await page.getByTestId("decision-notify").uncheck();
  await expect.poll(() => state.settings["notify.decision"]).toBe("0");
});
