import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

function handlers(state: { opened: number; entries: unknown[] }) {
  return {
    hosts_list: () => [], groups_list: () => [], snippets_list: () => [],
    settings_all: () => ({}), settings_get: () => null, settings_set: () => null,
    ssh_active_sessions: () => [], agent_plans_pending: () => [],
    master_status: () => ({ enabled: false, unlocked: false, lockoutSecondsLeft: 0, failedAttempts: 0, minLength: 12 }),
    sync_status: () => ({ endpoint: "", email: "", signedIn: false, pending: 0, lastError: null, cursor: 0 }),
    crash_reports_enabled: () => false, crash_endpoint_get: () => null, crash_reports_list: () => [],
    crash_reports_dir: () => "/crashes",
    logs_dir: () => "/home/tester/.local/state/darwaza/log",
    logs_open_dir: () => { state.opened += 1; },
    agent_log_list: () => state.entries,
  };
}

async function openReports(page: import("@playwright/test").Page) {
  await page.addInitScript(() => localStorage.setItem("locale", "en"));
  await page.goto("/settings");
  await page.getByRole("button", { name: "Reports", exact: true }).click();
}

test("журналы: путь, кнопка папки и действия помощника с отказами", async ({ page }) => {
  const state = {
    opened: 0,
    entries: [
      { id: 2, at: "2026-11-06T06:43:06Z", host: "db", action: "run", detail: "rm -rf /tmp/x", outcome: "refused", exitCode: null, note: null },
      { id: 1, at: "2026-11-06T06:40:00Z", host: "db", action: "read", detail: "/etc/hosts", outcome: "ok", exitCode: null, note: null },
    ],
  };
  await mockTauriInvoke(page, handlers(state) as never);
  await openReports(page);

  await expect(page.getByTestId("logs-dir")).toHaveText("/home/tester/.local/state/darwaza/log");
  await expect(page.getByTestId("logs-agent-row")).toHaveCount(2);
  await expect(page.getByTestId("logs-agent-row").first()).toContainText("refused");
  await expect(page.getByTestId("logs-agent-row").first()).toContainText("rm -rf /tmp/x");

  await page.getByTestId("logs-open-dir").click();
  await expect.poll(() => state.opened).toBe(1);
});

test("журналы: пустой список помощника объяснён", async ({ page }) => {
  await mockTauriInvoke(page, handlers({ opened: 0, entries: [] }) as never);
  await openReports(page);
  await expect(page.getByTestId("logs-empty")).toBeVisible();
});
