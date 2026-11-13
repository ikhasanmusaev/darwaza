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
  { name: "www", path: "/root/www", kind: "symlink", size: 0, modified: null, permissions: null },
  { name: ".env", path: "/root/.env", kind: "file", size: 64, modified: null, permissions: 0o600 },
  { name: "app.conf", path: "/root/app.conf", kind: "file", size: 2048, modified: null, permissions: 0o644 },
];

const WWW_ENTRIES = [
  { name: "index.html", path: "/root/www/index.html", kind: "file", size: 10, modified: null, permissions: null },
];

const LOCAL_ENTRIES = [
  { name: "Downloads", path: "/home/tester/Downloads", kind: "dir", size: 0, modified: null },
  { name: "notes.txt", path: "/home/tester/notes.txt", kind: "file", size: 512, modified: null },
];

function makeHandlers(calls: string[], settings: Record<string, string> = {}) {
  return {
    hosts_list: () => [HOST],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => settings,
    settings_get: (args: any) => settings[args?.key] ?? null,
    settings_set: (args: any) => {
      settings[args.key] = args.value;
    },
    clipboard_write_text: (args: any) => {
      calls.push(`clipboard:${args.text}`);
    },
    ssh_active_sessions: () => [],
    ssh_connect: () => ({ sessionId: "sftp-session-1" }),
    ssh_disconnect: () => {},
    ssh_send: () => {},
    ssh_resize: () => {},

    sftp_home: () => "/root",
    sftp_list: (args: any) => {
      if (args.path === "/root/www") return WWW_ENTRIES;
      if (args.path === "/root/app.conf") throw new Error("not a directory");
      return REMOTE_ENTRIES;
    },
    sftp_parent: () => "/",
    local_home: () => "/home/tester",
    local_parent: () => "/home",
    local_list: () => LOCAL_ENTRIES,
    transfer_tree: (args: any) => {
      calls.push(
        `tree:${args.source.kind}:${args.sourcePath}:${args.isDir ? "dir" : "file"}->${args.target.kind}:${args.targetDir}` +
          (args.onConflict === "replace" ? " replace" : ""),
      );
      return "transfer-1";
    },
  };
}

async function openPanel(page: any) {
  await page
    .getByTestId("host-row")
    .filter({ hasText: "files-host" })
    .getByRole("button", { name: "Ulanish", exact: true })
    .click();
  await emitTauriEvent(page, "session:status", {
    event: "status",
    sessionId: "sftp-session-1",
    status: "CONNECTED",
  });
  await page.getByTestId("sftp-toggle").click();
  await expect(page.getByText("app.conf")).toBeVisible();
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
      .poll(() => calls.find((c) => c.startsWith("tree:local")))
      .toBe("tree:local:/home/tester/notes.txt:file->remote:/root");

    // Скачивание: выбираем удалённый файл → стрелка влево
    await page.getByRole("button", { name: /app\.conf/ }).click();
    await page.getByTitle("Kompyuterga yuklab olish").click();
    await expect
      .poll(() => calls.find((c) => c.startsWith("tree:remote")))
      .toBe("tree:remote:/root/app.conf:file->local:/home/tester");
  });

  test("папка передаётся целиком — кнопкой и перетаскиванием", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, makeHandlers(calls));
    await page.goto("/hosts");
    await openPanel(page);

    await page.getByTestId("remote-entry").filter({ hasText: "logs" }).click();
    await page.getByTitle("Kompyuterga yuklab olish").click();
    await expect
      .poll(() => calls.find((c) => c.startsWith("tree:remote")))
      .toBe("tree:remote:/root/logs:dir->local:/home/tester");

    await page
      .getByTestId("local-entry")
      .filter({ hasText: "Downloads" })
      .dragTo(page.getByTestId("remote-dropzone"));
    await expect
      .poll(() => calls.find((c) => c.startsWith("tree:local")))
      .toBe("tree:local:/home/tester/Downloads:dir->remote:/root");
  });

  test("в ссылку на папку можно войти, ссылка на файл ничего не ломает", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers([]));
    await page.goto("/hosts");
    await openPanel(page);

    await page.getByTestId("remote-entry").filter({ hasText: "www" }).dblclick();
    await expect(page.getByText("index.html")).toBeVisible();
    await expect(page.getByTitle("/root/www")).toBeVisible();
  });

  test("скрытые файлы, фильтр и копирование пути", async ({ page }) => {
    const calls: string[] = [];
    const settings: Record<string, string> = {};
    await mockTauriInvoke(page, makeHandlers(calls, settings));
    await page.goto("/hosts");
    await openPanel(page);

    const remote = page.getByTestId("remote-entry");
    await expect(remote.filter({ hasText: ".env" })).toBeVisible();
    // Права видны в подсказке строки.
    await expect(remote.filter({ hasText: ".env" })).toHaveAttribute("title", /rw-------/);

    await page.getByTestId("sftp-toggle-hidden").click();
    await expect(remote.filter({ hasText: ".env" })).toHaveCount(0);
    expect(settings["files.show_hidden"]).toBe("false");

    await page.getByTestId("sftp-filter").fill("CONF");
    await expect(remote).toHaveCount(1);
    await expect(remote.first()).toContainText("app.conf");

    await remote.first().click();
    await page.getByTestId("sftp-copy-path").click();
    await expect.poll(() => calls.find((c) => c.startsWith("clipboard:"))).toBe("clipboard:/root/app.conf");
    await expect(page.getByTestId("sftp-note")).toContainText("/root/app.conf");
  });

  test("Ctrl+щелчок выбирает несколько, и скачиваются все", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, makeHandlers(calls));
    await page.goto("/hosts");
    await openPanel(page);

    const remote = page.getByTestId("remote-entry");
    await remote.filter({ hasText: "app.conf" }).click();
    await remote.filter({ hasText: "logs" }).click({ modifiers: ["Control"] });
    await expect(remote.filter({ hasText: "logs" })).toHaveAttribute("aria-selected", "true");
    await expect(remote.filter({ hasText: "app.conf" })).toHaveAttribute("aria-selected", "true");

    await page.getByTitle("Kompyuterga yuklab olish").click();
    await expect.poll(() => calls.filter((c) => c.startsWith("tree:remote")).sort()).toEqual([
      "tree:remote:/root/app.conf:file->local:/home/tester",
      "tree:remote:/root/logs:dir->local:/home/tester",
    ]);
  });

  test("имя уже занято — спрашивает: заменить, оставить оба или пропустить", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, {
      ...makeHandlers(calls),
      // На сервере уже лежит notes.txt.
      sftp_list: () => [
        ...REMOTE_ENTRIES,
        { name: "notes.txt", path: "/root/notes.txt", kind: "file", size: 1, modified: null, permissions: null },
      ],
    });
    await page.goto("/hosts");
    await openPanel(page);

    await page.getByTestId("local-entry").filter({ hasText: "notes.txt" }).click();
    const upload = page.getByTitle("Serverga yuklash");

    // Пропустить — ничего не уходит.
    await upload.click();
    await expect(page.getByTestId("conflict-dialog")).toContainText("notes.txt");
    await page.getByTestId("conflict-skip").click();
    await expect(page.getByTestId("conflict-dialog")).toHaveCount(0);
    expect(calls.filter((c) => c.startsWith("tree:"))).toEqual([]);

    // Enter по умолчанию — «оставить оба»: ничего не затирается.
    await upload.click();
    await expect(page.getByTestId("conflict-rename")).toBeFocused();
    await page.keyboard.press("Enter");
    await expect.poll(() => calls.filter((c) => c.startsWith("tree:"))).toEqual([
      "tree:local:/home/tester/notes.txt:file->remote:/root",
    ]);

    // Заменить.
    await upload.click();
    await page.getByTestId("conflict-replace").click();
    await expect.poll(() => calls.filter((c) => c.startsWith("tree:")).length).toBe(2);
    expect(calls.filter((c) => c.startsWith("tree:"))[1]).toBe(
      "tree:local:/home/tester/notes.txt:file->remote:/root replace",
    );
  });

  test("несколько совпадений: «для всех» задаёт ответ один раз", async ({ page }) => {
    const calls: string[] = [];
    await mockTauriInvoke(page, {
      ...makeHandlers(calls),
      sftp_list: () => [
        ...REMOTE_ENTRIES,
        { name: "notes.txt", path: "/root/notes.txt", kind: "file", size: 1, modified: null, permissions: null },
        { name: "Downloads", path: "/root/Downloads", kind: "dir", size: 0, modified: null, permissions: null },
      ],
    });
    await page.goto("/hosts");
    await openPanel(page);

    const local = page.getByTestId("local-entry");
    await local.filter({ hasText: "Downloads" }).click();
    await local.filter({ hasText: "notes.txt" }).click({ modifiers: ["Shift"] });
    await page.getByTitle("Serverga yuklash").click();

    // Первая — папка: вопрос про слияние.
    await expect(page.getByTestId("conflict-replace")).toHaveText("Birlashtirish");
    await page.getByTestId("conflict-for-all").check();
    await page.getByTestId("conflict-replace").click();
    await expect.poll(() => calls.filter((c) => c.startsWith("tree:")).sort()).toEqual([
      "tree:local:/home/tester/Downloads:dir->remote:/root replace",
      "tree:local:/home/tester/notes.txt:file->remote:/root replace",
    ]);
    await expect(page.getByTestId("conflict-dialog")).toHaveCount(0);
  });
});
