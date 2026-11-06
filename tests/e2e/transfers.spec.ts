import { test, expect } from "@playwright/test";
import { mockTauriInvoke, emitTauriEvent } from "./helpers";

const HOST = {
  id: 1,
  groupId: null,
  name: "files-host",
  hostname: "10.0.0.5",
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

function makeHandlers(state: any) {
  return {
    hosts_list: () => [HOST],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],
    ssh_connect: () => ({ sessionId: "files-session-1" }),
    ssh_disconnect: () => {},
    ssh_send: () => {},
    ssh_resize: () => {},
    master_status: () => ({
      enabled: false,
      unlocked: false,
      lockoutSecondsLeft: 0,
      failedAttempts: 0,
    }),

    local_home: () => "/home/tester",
    local_list: () => [
      { name: "notes.txt", path: "/home/tester/notes.txt", kind: "file", size: 120 },
    ],
    local_parent: () => "/home",
    sftp_home: () => "/root",
    sftp_list: () => [
      { name: "backup.tar", path: "/root/backup.tar", kind: "file", size: 4096 },
    ],
    transfers_download_dir: () => "/home/tester/Downloads",
    transfer_download: (args: any) => {
      state.downloads.push(args);
      return "t-download-1";
    },
    transfer_upload: (args: any) => {
      state.uploads.push(args);
      return "t-upload-1";
    },
    transfer_cancel: (args: any) => {
      state.cancelled.push(args.id);
      return true;
    },
  };
}

async function openFilePanel(page: any) {
  await page.goto("/hosts");
  await page
    .getByTestId("host-row")
    .filter({ hasText: "files-host" })
    .getByRole("button", { name: "Ulanish", exact: true })
    .click();
  await emitTauriEvent(page, "session:status", {
    event: "status",
    sessionId: "files-session-1",
    status: "CONNECTED",
  });
  await page.getByTestId("sftp-toggle").click();
  await expect(page.getByTestId("remote-entry").first()).toBeVisible();
}

test.describe("S23 — Drag & drop transfers", () => {
  test("dragging a remote file onto the local pane downloads into its open folder", async ({
    page,
  }) => {
    const state = { downloads: [] as any[], uploads: [] as any[], cancelled: [] as string[] };
    await mockTauriInvoke(page, makeHandlers(state));
    await openFilePanel(page);

    await page
      .getByTestId("remote-entry")
      .first()
      .dragTo(page.getByTestId("local-dropzone"));

    // Адресат — папка, открытая в локальной панели, а не какой-то фиксированный путь
    await expect.poll(() => state.downloads).toHaveLength(1);
    expect(state.downloads[0].remotePath).toBe("/root/backup.tar");
    expect(state.downloads[0].targetDir).toBe("/home/tester");
  });

  test("dragging a local file onto the remote pane uploads into its open folder", async ({
    page,
  }) => {
    const state = { downloads: [] as any[], uploads: [] as any[], cancelled: [] as string[] };
    await mockTauriInvoke(page, makeHandlers(state));
    await openFilePanel(page);

    await page
      .getByTestId("local-entry")
      .first()
      .dragTo(page.getByTestId("remote-dropzone"));

    await expect.poll(() => state.uploads).toHaveLength(1);
    expect(state.uploads[0].localPath).toBe("/home/tester/notes.txt");
    expect(state.uploads[0].remoteDir).toBe("/root");
  });

  test("progress shows at the bottom and can be stopped", async ({ page }) => {
    const state = { downloads: [] as any[], uploads: [] as any[], cancelled: [] as string[] };
    await mockTauriInvoke(page, makeHandlers(state));
    await openFilePanel(page);

    // Панели передач нет, пока нечего показывать
    await expect(page.getByTestId("transfers-bar")).toHaveCount(0);

    await emitTauriEvent(page, "transfer:progress", {
      id: "t-1",
      kind: "download",
      name: "backup.tar",
      transferred: 2048,
      total: 4096,
      status: "RUNNING",
    });

    const bar = page.getByTestId("transfers-bar");
    await expect(bar).toBeVisible();
    await expect(bar).toContainText("backup.tar");

    // Остановка идущей передачи уходит в backend
    await page.getByTestId("transfer-cancel").first().click();
    await expect.poll(() => state.cancelled).toEqual(["t-1"]);
  });

  test("dropping files from the system file manager uploads them to the open remote folder", async ({
    page,
  }) => {
    const state = { downloads: [] as any[], uploads: [] as any[], cancelled: [] as string[] };
    await mockTauriInvoke(page, makeHandlers(state));
    await openFilePanel(page);

    // Координаты дропа приходят от ОС в физических пикселях и мимо DOM,
    // поэтому попадание в панель считается по её прямоугольнику.
    const box = await page.getByTestId("remote-dropzone").boundingBox();
    const dpr = await page.evaluate(() => window.devicePixelRatio || 1);
    const position = {
      x: Math.round((box!.x + box!.width / 2) * dpr),
      y: Math.round((box!.y + box!.height / 2) * dpr),
    };

    await emitTauriEvent(page, "tauri://drag-drop", {
      paths: ["/home/tester/a.conf", "/home/tester/b.conf"],
      position,
    });

    // Оба файла пачки уходят в открытую серверную папку
    await expect.poll(() => state.uploads).toHaveLength(2);
    expect(state.uploads.map((u) => u.localPath)).toEqual([
      "/home/tester/a.conf",
      "/home/tester/b.conf",
    ]);
    expect(state.uploads[0].remoteDir).toBe("/root");
  });

  test("a system drop outside the remote pane is ignored", async ({ page }) => {
    const state = { downloads: [] as any[], uploads: [] as any[], cancelled: [] as string[] };
    await mockTauriInvoke(page, makeHandlers(state));
    await openFilePanel(page);

    const box = await page.getByTestId("local-dropzone").boundingBox();
    const dpr = await page.evaluate(() => window.devicePixelRatio || 1);

    await emitTauriEvent(page, "tauri://drag-drop", {
      paths: ["/home/tester/a.conf"],
      position: {
        x: Math.round((box!.x + box!.width / 2) * dpr),
        y: Math.round((box!.y + box!.height / 2) * dpr),
      },
    });

    // Копировать локальный файл в локальную же папку — работа файлового
    // менеджера, не наша: молчим.
    await page.waitForTimeout(300);
    expect(state.uploads).toHaveLength(0);
  });
});
