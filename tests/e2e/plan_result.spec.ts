import { test, expect } from "@playwright/test";
import { mockTauriInvoke, emitTauriEvent } from "./helpers";

const BASE = {
  hosts_list: () => [],
  groups_list: () => [],
  snippets_list: () => [],
  settings_all: () => ({}),
  settings_get: () => null,
  ssh_active_sessions: () => [],
  agent_plans_pending: () => [],
};

test("completed result shows every done step and closes with the button", async ({ page }) => {
  await mockTauriInvoke(page, BASE);
  await page.goto("/hosts");
  await emitTauriEvent(page, "agent://plan_result", {
    planId: "plan-complete",
    host: "web-1.prod",
    status: "completed",
    steps: [
      { title: "nginx -t", outcome: "done", exitCode: 0 },
      { title: "systemctl reload nginx", outcome: "done", exitCode: 0 },
    ],
    reason: null,
    done: 2,
    total: 2,
  });

  const dialog = page.getByTestId("plan-result-dialog");
  await expect(dialog).toContainText("2/2 qadam bajarildi");
  await expect(dialog).toContainText("nginx -t");
  await expect(dialog).toContainText("Bajarildi");
  await page.getByTestId("plan-result-dismiss").click();
  await expect(dialog).toHaveCount(0);
});

test("stopped result distinguishes failed, blocked and notRun; Esc advances the queue", async ({ page }) => {
  await mockTauriInvoke(page, BASE);
  await page.goto("/hosts");
  await emitTauriEvent(page, "agent://plan_result", {
    planId: "plan-stopped",
    host: "web-1.prod",
    status: "stopped",
    steps: [
      { title: "nginx -t", outcome: "done", exitCode: 0 },
      { title: "systemctl reload nginx", outcome: "failed", exitCode: 1 },
      { title: "mkfs.ext4 /dev/sdb", outcome: "blocked", exitCode: null },
      { title: "systemctl status nginx", outcome: "notRun", exitCode: null },
    ],
    reason: "Execution stopped at the blocked step",
    done: 1,
    total: 4,
  });

  const dialog = page.getByTestId("plan-result-dialog");
  await expect(dialog).toContainText("Taqiqlangan qadamda to'xtatildi");
  await expect(dialog).toContainText("Bajarishda xato (kod 1)");
  await expect(dialog).toContainText("Siyosat taqiqlaydi");
  await expect(dialog).toContainText("Bajarilmagan");
  await expect(dialog).toContainText("Execution stopped at the blocked step");

  await emitTauriEvent(page, "agent://plan_result", {
    planId: "plan-next",
    host: "db-primary",
    status: "completed",
    steps: [{ title: "pg_isready", outcome: "done", exitCode: 0 }],
    reason: null,
    done: 1,
    total: 1,
  });
  await page.keyboard.press("Escape");
  await expect(dialog).toContainText("1/1 qadam bajarildi");
  await expect(dialog).toContainText("db-primary");
});
