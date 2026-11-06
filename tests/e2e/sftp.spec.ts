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
  keepAliveInterval: 30,
  description: null,
  lastConnectedAt: null,
  createdAt: "2026-07-27T00:00:00Z",
  updatedAt: "2026-07-27T00:00:00Z",
};

const REMOTE_ENTRIES = [
  { name: "logs", path: "/root/logs", kind: "dir", size: 0, modified: null, permissions: null },
  { name: "app.conf", path: "/root/app.conf", kind: "file", size: 2048, modified: null, permissions: null },
];

const LOCAL_ENTRIES = [
  { name: "Downloads", path: "/home/tester/Downloads", kind: "dir", size: 0, modified: null },
  { name: "notes.txt", path: "/home/tester/notes.txt", kind: "file", size: 512, modified: null },
];

function makeHandlers(calls: string[]) {
  return {
    hosts_list: () => [HOST],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],
    ssh_connect: () => ({ sessionId: "sftp-session-1" }),
    ssh_disconnect: () => {},
    ssh_send: () => {},
    ssh_resize: () => {},

    sftp_home: () => "/root",
    sftp_list: () => REMOTE_ENTRIES,
    sftp_parent: () => "/",
    local_home: () => "/home/tester",
    local_parent: () => "/home",
    local_list: () => LOCAL_ENTRIES,
    sftp_upload: (args: any) => {
      calls.push(`upload:${args.localPath}->${args.remotePath}`);
      return 512;
    },
    sftp_download: (args: any) => {
      calls.push(`download:${args.remotePath}->${args.localPath}`);
      return 2048;
    },
  };
}

test.describe("S13 — SFTP file browser", () => {
  test("opens two-pane browser and transfers both directions", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, makeHandlers(calls));
    await page.goto("/hosts");

    // Открываем сессию
    await page
      .getByTestId("host-row")
      .filter({ hasText: "files-host" })
      .getByRole("button", { name: "Ulanish", exact: true })
      .click();
    await expect(page.getByTestId("session-tab")).toHaveCount(1);

    // Панель файлов показывается только на CONNECTED, а статус приходит
    // событием от backend — в тесте досылаем его тем же каналом.
    await emitTauriEvent(page, "session:status", {
      event: "status",
      sessionId: "sftp-session-1",
      status: "CONNECTED",
    });

    // Кнопка файлов появляется только на живой сессии
    const toggle = page.getByTestId("sftp-toggle");
    await expect(toggle).toBeVisible();
    await toggle.click();

    // Обе колонки заполнились
    await expect(page.getByText("app.conf")).toBeVisible();
    await expect(page.getByText("notes.txt")).toBeVisible();

    // Выгрузка: выбираем локальный файл → стрелка вправо
    await page.getByRole("button", { name: /notes\.txt/ }).click();
    await page.getByTitle("Serverga yuklash").click();
    await expect
      .poll(() => calls.find((c) => c.startsWith("upload:")))
      .toBe("upload:/home/tester/notes.txt->/root/notes.txt");

    // Скачивание: выбираем удалённый файл → стрелка влево
    await page.getByRole("button", { name: /app\.conf/ }).click();
    await page.getByTitle("Kompyuterga yuklab olish").click();
    await expect
      .poll(() => calls.find((c) => c.startsWith("download:")))
      .toBe("download:/root/app.conf->/home/tester/app.conf");
  });
});
