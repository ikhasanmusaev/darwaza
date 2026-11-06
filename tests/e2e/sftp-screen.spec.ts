import { test, expect } from "@playwright/test";
import { mockTauriInvoke, emitTauriEvent } from "./helpers";

/**
 * Экран SFTP в боковой панели: две панели, в каждой — компьютер или сервер.
 * Работает без открытой вкладки терминала; если у хоста вкладка есть, берёт её.
 */
const HOST = {
  id: 1,
  groupId: null,
  name: "files-host",
  hostname: "203.0.113.10",
  port: 22,
  username: "deploy",
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
const HOST2 = { ...HOST, id: 2, name: "second-host", hostname: "203.0.113.11" };

const REMOTE = [
  { name: "logs", path: "/srv/logs", kind: "dir", size: 0, modified: null, permissions: null },
  { name: "app.conf", path: "/srv/app.conf", kind: "file", size: 2048, modified: null, permissions: null },
];
const LOCAL = [
  { name: "Downloads", path: "/home/tester/Downloads", kind: "dir", size: 0, modified: null },
  { name: "notes.txt", path: "/home/tester/notes.txt", kind: "file", size: 512, modified: null },
];

function handlers(calls: string[]) {
  return {
    hosts_list: () => [HOST, HOST2],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],
    ssh_connect: () => ({ sessionId: "tab-session" }),
    sftp_connect: (a: any) => {
      calls.push(`connect:${a.hostId}`);
      return { sessionId: `files-session-${a.hostId}` };
    },
    ssh_disconnect: (a: any) => {
      calls.push(`disconnect:${a.sessionId}`);
    },
    ssh_send: () => {},
    ssh_resize: () => {},
    "plugin:app|version": () => "9.8.7",
    sftp_home: () => "/srv",
    sftp_list: (a: any) => {
      calls.push(`list:${a.sessionId}:${a.path}`);
      return REMOTE;
    },
    sftp_parent: () => "/",
    sftp_mkdir: (a: any) => calls.push(`rmkdir:${a.path}`),
    local_home: () => "/home/tester",
    local_parent: () => "/home",
    local_list: () => LOCAL,
    local_mkdir: (a: any) => {
      calls.push(`lmkdir:${a.dir}|${a.name}`);
      return `${a.dir}/${a.name}`;
    },
    local_rename: (a: any) => {
      calls.push(`lrename:${a.path}->${a.newName}`);
      return "x";
    },
    local_remove: (a: any) => {
      calls.push(`lremove:${a.path}:${a.isDir}`);
    },
    transfer_tree: (a: any) => {
      const side = (e: any) => (e.kind === "local" ? "local" : e.sessionId);
      calls.push(
        `tree:${side(a.source)}:${a.sourcePath}${a.isDir ? "/" : ""}->${side(a.target)}:${a.targetDir}`,
      );
      return "t1";
    },
    transfer_cancel: (a: any) => {
      calls.push(`cancel:${a.id}`);
      return true;
    },
  };
}

async function openScreen(page: import("@playwright/test").Page) {
  await page.getByTestId("sidebar-sftp").click();
  await expect(page.getByTestId("sftp-page")).toBeVisible();
}

test.describe("экран SFTP", () => {
  test("обе панели помещаются в окно, а имена файлов не мельче 14px", async ({ page }) => {
    const long = "/home/tester/" + "очень-длинная-папка/".repeat(12);
    await mockTauriInvoke(page, {
      ...handlers([]),
      local_home: () => long,
      local_list: () => [
        { name: "x".repeat(120), path: long + "x", kind: "file", size: 1, modified: null },
      ],
    });
    await page.setViewportSize({ width: 1100, height: 700 });
    await page.goto("/hosts");
    await openScreen(page);

    const box = await page.getByTestId("sftp-page").locator("section").last().boundingBox();
    expect(box!.x + box!.width).toBeLessThanOrEqual(1100);
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBe(0);
    const px = await page
      .getByTestId("files-entry-left")
      .first()
      .evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
    expect(px).toBeGreaterThanOrEqual(14);
  });

  test("пункт виден без сессий, левая панель — компьютер, правая просит выбрать сервер", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, handlers(calls));
    await page.goto("/hosts");
    await expect(page.getByTestId("session-tab")).toHaveCount(0);
    await expect(page.getByTestId("sidebar-sftp")).toBeVisible();

    await openScreen(page);
    await expect(page.getByTestId("files-path-left")).toHaveText("/home/tester");
    await expect(page.getByTestId("files-entry-left")).toHaveCount(2);
    await expect(page.getByTestId("files-entry-right")).toHaveCount(0);
    await expect(page.getByTestId("sidebar-sftp")).toHaveClass(/font-semibold/);
  });

  test("сервер без вкладки подключается отдельным соединением и загружает файл", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, handlers(calls));
    await page.goto("/hosts");
    await openScreen(page);

    await page.getByTestId("files-source-right").selectOption("host:1");
    await expect(page.getByTestId("files-connecting-right")).toBeVisible();
    expect(calls).toContain("connect:1");
    // Вкладка терминала не появилась.
    await expect(page.getByTestId("session-tab")).toHaveCount(0);

    await emitTauriEvent(page, "session:status", {
      event: "status", sessionId: "files-session-1", status: "CONNECTED",
    });
    await expect(page.getByTestId("files-path-right")).toHaveText("/srv");
    await expect(page.getByTestId("files-entry-right")).toHaveCount(2);

    // Локальный файл → сервер.
    await page.getByTestId("files-entry-left").filter({ hasText: "notes.txt" }).click();
    await page.getByTestId("files-copy-right").click();
    await expect.poll(() => calls.some((c) => c === "tree:local:/home/tester/notes.txt->files-session-1:/srv")).toBe(true);

    // Файл сервера → компьютер.
    await page.getByTestId("files-entry-right").filter({ hasText: "app.conf" }).click();
    await page.getByTestId("files-copy-left").click();
    await expect.poll(() => calls.some((c) => c === "tree:files-session-1:/srv/app.conf->local:/home/tester")).toBe(true);
  });

  test("если у хоста уже есть живая вкладка, берётся её соединение", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, handlers(calls));
    await page.goto("/hosts");
    await page.getByTestId("host-row").filter({ hasText: "files-host" }).getByTestId("host-connect").click();
    await expect(page.getByTestId("session-tab")).toHaveCount(1);
    await emitTauriEvent(page, "session:status", { event: "status", sessionId: "tab-session", status: "CONNECTED" });

    await openScreen(page);
    await page.getByTestId("files-source-right").selectOption("host:1");
    await expect(page.getByTestId("files-entry-right")).toHaveCount(2);
    expect(calls.filter((c) => c.startsWith("connect:"))).toEqual([]);
    expect(calls).toContain("list:tab-session:/srv");
  });

  test("два сервера рядом: копирование между ними идёт, а своё соединение закрывается при смене источника", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, handlers(calls));
    await page.goto("/hosts");
    await openScreen(page);

    await page.getByTestId("files-source-left").selectOption("host:1");
    await emitTauriEvent(page, "session:status", { event: "status", sessionId: "files-session-1", status: "CONNECTED" });
    await page.getByTestId("files-source-right").selectOption("host:2");
    await emitTauriEvent(page, "session:status", { event: "status", sessionId: "files-session-2", status: "CONNECTED" });
    await expect(page.getByTestId("files-entry-left")).toHaveCount(2);
    await expect(page.getByTestId("files-entry-right")).toHaveCount(2);

    // Между двумя серверами копирование работает — потоком, без диска.
    await page.getByTestId("files-entry-left").filter({ hasText: "app.conf" }).click();
    await expect(page.getByTestId("files-copy-right")).toBeEnabled();
    await page.getByTestId("files-copy-right").click();
    await expect.poll(() => calls).toContain("tree:files-session-1:/srv/app.conf->files-session-2:/srv");

    // Правая панель снова на компьютер — её соединение с сервером больше не нужно.
    await page.getByTestId("files-source-right").selectOption("local");
    await expect.poll(() => calls).toContain("disconnect:files-session-2");
    expect(calls).not.toContain("disconnect:files-session-1");
  });

  test("сбой подключения показан в панели", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, handlers(calls));
    await page.goto("/hosts");
    await openScreen(page);
    await page.getByTestId("files-source-right").selectOption("host:1");
    await emitTauriEvent(page, "session:status", {
      event: "status", sessionId: "files-session-1", status: "FAILED", error: "authentication failed",
    });
    await expect(page.getByTestId("files-error-right")).toBeVisible();
  });

  test("локальная сторона: новая папка, переименование и удаление с подтверждением", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, handlers(calls));
    await page.goto("/hosts");
    await openScreen(page);

    await page.getByTestId("files-mkdir-left").click();
    await page.getByTestId("files-prompt-left").fill("новая");
    await page.getByTestId("files-prompt-left").press("Enter");
    await expect.poll(() => calls).toContain("lmkdir:/home/tester|новая");

    await page.getByTestId("files-entry-left").filter({ hasText: "notes.txt" }).click();
    await page.getByTestId("files-rename-left").click();
    await page.getByTestId("files-prompt-left").fill("notes2.txt");
    await page.getByTestId("files-prompt-left").press("Enter");
    await expect.poll(() => calls).toContain("lrename:/home/tester/notes.txt->notes2.txt");

    await page.getByTestId("files-entry-left").filter({ hasText: "notes.txt" }).click();
    await page.getByTestId("files-delete-left").click();
    // Без подтверждения ничего не удаляется.
    expect(calls.some((c) => c.startsWith("lremove:"))).toBe(false);
    await page.getByRole("button", { name: "O'chirish", exact: true }).click();
    await expect.poll(() => calls).toContain("lremove:/home/tester/notes.txt:false");
  });

  test("папка передаётся целиком в обе стороны, оба компьютера — копирование недоступно", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, handlers(calls));
    await page.goto("/hosts");
    await openScreen(page);
    await page.getByTestId("files-source-right").selectOption("host:1");
    await emitTauriEvent(page, "session:status", { event: "status", sessionId: "files-session-1", status: "CONNECTED" });
    await expect(page.getByTestId("files-entry-right")).toHaveCount(2);

    // Папка с компьютера → сервер.
    await page.getByTestId("files-entry-left").filter({ hasText: "Downloads" }).click();
    await page.getByTestId("files-copy-right").click();
    await expect.poll(() => calls).toContain("tree:local:/home/tester/Downloads/->files-session-1:/srv");

    // Папка с сервера → компьютер.
    await page.getByTestId("files-entry-right").filter({ hasText: "logs" }).click();
    await page.getByTestId("files-copy-left").click();
    await expect.poll(() => calls).toContain("tree:files-session-1:/srv/logs/->local:/home/tester");

    // Обе панели — компьютер: копировать здесь нечем.
    await page.getByTestId("files-source-right").selectOption("local");
    await page.getByTestId("files-entry-left").filter({ hasText: "Downloads" }).click();
    await expect(page.getByTestId("files-copy-right")).toBeDisabled();
  });

  test("прогресс папки идёт по общему размеру, ошибка называет файл, отмена доходит до backend", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, handlers(calls));
    await page.goto("/hosts");
    await openScreen(page);

    const base = { id: "tree-1", kind: "upload", name: "proj/", total: 1000 };
    await emitTauriEvent(page, "transfer:progress", {
      ...base, transferred: 250, status: "RUNNING", currentPath: "a/b/big.bin",
    });
    const row = page.getByTestId("transfer-row");
    await expect(row).toContainText("proj/");
    await expect(row).toContainText("250 B / 1000 B");
    await expect(page.getByTestId("transfer-current")).toHaveText("a/b/big.bin");

    await page.getByTestId("transfer-cancel").click();
    await expect.poll(() => calls).toContain("cancel:tree-1");

    await emitTauriEvent(page, "transfer:progress", {
      ...base, transferred: 0, total: null, status: "FAILED", error: "errors.unexpected", currentPath: "a/b/two.txt",
    });
    const error = page.getByTestId("transfer-error");
    await expect(error).toContainText("a/b/two.txt:");
  });

  test("в шапке — настоящая версия приложения", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, handlers(calls));
    await page.goto("/hosts");
    await expect(page.getByTestId("app-version")).toHaveText("v9.8.7");
  });
});
