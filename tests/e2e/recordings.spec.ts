import { test, expect } from "@playwright/test";
import { mockTauriInvoke, emitTauriEvent } from "./helpers";

const HOST = {
  id: 1,
  groupId: null,
  name: "rec-host",
  hostname: "10.0.0.9",
  port: 22,
  username: "root",
  identityId: null,
  authMethod: "password",
  color: null,
  icon: null,
  jumpHostId: null,
  forwardAgent: false,
  keepAliveInterval: 30,
  description: null,
  lastConnectedAt: null,
  createdAt: "2026-08-18T00:00:00Z",
  updatedAt: "2026-08-18T00:00:00Z",
};

function makeHandlers(state: { calls: string[]; files: any[] }) {
  return {
    hosts_list: () => [HOST],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],
    ssh_connect: () => ({ sessionId: "rec-session-1" }),
    ssh_disconnect: () => {},
    ssh_send: () => {},
    ssh_resize: () => {},
    master_status: () => ({
      enabled: false,
      unlocked: false,
      lockoutSecondsLeft: 0,
      failedAttempts: 0,
    }),

    recordings_dir: () => "/home/tester/.local/share/termius-analog/recordings",
    recordings_list: () => state.files,
    recordings_remove: (args: any) => {
      state.files = state.files.filter((f) => f.path !== args.path);
    },
    session_recording_start: (args: any) => {
      state.calls.push(`start:${args.sessionId}`);
      return "/rec/rec-host-20260818.cast";
    },
    session_recording_stop: (args: any) => {
      state.calls.push(`stop:${args.sessionId}`);
      state.files.push({
        name: "rec-host-20260818.cast",
        path: "/rec/rec-host-20260818.cast",
        size: 4096,
        modified: 1755000000,
      });
      return "/rec/rec-host-20260818.cast";
    },
  };
}

test.describe("S16 — Session recording", () => {
  test("starts and stops recording from the tab bar", async ({ page }) => {
    const state = { calls: [] as string[], files: [] as any[] };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/hosts");
    await page
      .getByTestId("host-row")
      .filter({ hasText: "rec-host" })
      .getByRole("button", { name: "Ulanish", exact: true })
      .click();

    // Кнопка записи появляется только на живой сессии
    await expect(page.getByTestId("record-toggle")).toHaveCount(0);
    await emitTauriEvent(page, "session:status", {
      event: "status",
      sessionId: "rec-session-1",
      status: "CONNECTED",
    });

    const toggle = page.getByTestId("record-toggle");
    await expect(toggle).toBeVisible();

    await toggle.click();
    await expect.poll(() => state.calls).toContain("start:rec-session-1");

    await toggle.click();
    await expect.poll(() => state.calls).toContain("stop:rec-session-1");
  });

  test("lists recordings and deletes one", async ({ page }) => {
    const state = {
      calls: [] as string[],
      files: [
        {
          name: "web-1-20260818-101500.cast",
          path: "/rec/web-1-20260818-101500.cast",
          size: 20480,
          modified: 1755000000,
          title: "web-1 (10.0.0.9)",
          durationSeconds: 80,
          active: false,
        },
      ] as any[],
    };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/recordings");
    await expect(page.getByText("web-1 (10.0.0.9)")).toBeVisible();
    await expect(page.getByText("01:20")).toBeVisible();
    await expect(page.getByText(/shifrlanmagan va sinxronlanmaydi/)).toBeVisible();

    const row = page.getByTestId("recordings-table").locator("tbody tr").first();
    await row.hover();
    await row.getByTitle("O'chirish").click();
    await page.click('button:has-text("O\'chirish")');

    await expect(page.getByText("Hozircha yozuv yo'q")).toBeVisible();
    expect(state.files).toHaveLength(0);
  });
});
