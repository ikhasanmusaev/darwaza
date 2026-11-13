import { test, expect } from "@playwright/test";
import { mockTauriInvoke, emitTauriEvent } from "./helpers";

/**
 * Управление вкладками (#209).
 *
 * Четыре жалобы владельца, проверяются по отдельности: Ctrl+W, средняя кнопка
 * мыши, `exit` в терминале и копирование Ctrl+C/Ctrl+V.
 */

function host(id: number, name: string) {
  return {
    id,
    groupId: null,
    name,
    hostname: `10.0.0.${id}`,
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
}

function makeHandlers(sent?: string[]) {
  let n = 0;
  return {
    hosts_list: () => [host(1, "alpha"), host(2, "beta")],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],
    ssh_connect: () => ({ sessionId: `tabs-session-${++n}` }),
    ssh_disconnect: () => {},
    ssh_latency: () => 12,
    ssh_send: (args?: any) => {
      // Декодируем как UTF-8: посимвольная склейка байтов ломает кириллицу.
      if (sent && args?.data) {
        sent.push(new TextDecoder().decode(Uint8Array.from(args.data)));
      }
    },
    ssh_resize: () => {},
    master_status: () => ({
      enabled: false,
      unlocked: false,
      lockoutSecondsLeft: 0,
      failedAttempts: 0,
    }),
  };
}

async function openHost(page: any, name: string, expectTabs: number) {
  await page.getByTestId("library-tab").click();
  await page
    .getByTestId("host-row")
    .filter({ hasText: name })
    .getByRole("button", { name: "Ulanish", exact: true })
    .click();
  await expect(page.getByTestId("session-tab")).toHaveCount(expectTabs);
}

test.describe("S209 — управление вкладками", () => {
  test("Ctrl+W закрывает активную вкладку", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers());
    await page.goto("/hosts");

    await openHost(page, "alpha", 1);
    await openHost(page, "beta", 2);

    await page.keyboard.press("Control+w");
    await expect(page.getByTestId("session-tab")).toHaveCount(1);
  });

  test("показывает задержку подключения и открывает Library новой кнопкой", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers());
    await page.goto("/hosts");
    await openHost(page, "alpha", 1);

    await emitTauriEvent(page, "session:status", {
      sessionId: "tabs-session-1",
      status: "CONNECTED",
      error: null,
    });

    await expect(page.getByTestId("connection-latency")).toContainText("12 ms");
    await page.getByTestId("new-tab").click();
    await expect(page.getByTestId("library-tab")).toHaveClass(/bg-surface-alt/);
  });

  test("сервер перестал отвечать — подпись красная, а не «подключено»", async ({ page }) => {
    let answer = true;
    await mockTauriInvoke(page, {
      ...makeHandlers(),
      ssh_latency: () => {
        if (!answer) throw new Error("SSH_TIMED_OUT");
        return 850;
      },
    });
    await page.goto("/hosts");
    await openHost(page, "alpha", 1);
    await emitTauriEvent(page, "session:status", {
      sessionId: "tabs-session-1",
      status: "CONNECTED",
      error: null,
    });

    const label = page.getByTestId("connection-latency");
    // 850 ms — печатать можно, но заметно медленно.
    await expect(label).toHaveAttribute("data-quality", "slow");
    await expect(label).toContainText("850 ms");

    answer = false;
    await expect(label).toHaveAttribute("data-quality", "bad", { timeout: 10_000 });
    await expect(label).toContainText("Server javob bermayapti");
  });

  test("средняя кнопка мыши закрывает вкладку", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers());
    await page.goto("/hosts");

    await openHost(page, "alpha", 1);
    await openHost(page, "beta", 2);

    // Привычка из браузера: колесо по вкладке закрывает её.
    await page.getByTestId("session-tab").first().click({ button: "middle" });
    await expect(page.getByTestId("session-tab")).toHaveCount(1);
  });

  test("завершение сессии закрывает вкладку", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers());
    await page.goto("/hosts");

    await openHost(page, "alpha", 1);

    // `exit` в оболочке: backend сообщает о закрытии сессии. Вкладка мёртвого
    // терминала не нужна — человек её всё равно закроет руками.
    await emitTauriEvent(page, "session:closed", {
      sessionId: "tabs-session-1",
      exitCode: 0,
    });
    await expect(page.getByTestId("session-tab")).toHaveCount(0);
  });

  test("вкладка со сбоем остаётся: на ней причина", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers());
    await page.goto("/hosts");

    await openHost(page, "alpha", 1);

    // Сбой закрывает сессию, и backend шлёт `closed` следом за `FAILED`.
    // Закрыть такую вкладку — значит унести с экрана единственное объяснение
    // того, что произошло. Ровно так пропадала «authentication failed»
    // (2026-08-28), и повторять это нельзя.
    await emitTauriEvent(page, "session:status", {
      sessionId: "tabs-session-1",
      status: "FAILED",
      error: "authentication failed",
    });
    await emitTauriEvent(page, "session:closed", {
      sessionId: "tabs-session-1",
      exitCode: 255,
    });

    await expect(page.getByTestId("session-tab")).toHaveCount(1);
  });

  test("Ctrl+Shift+V вставляет из буфера, а не мусор", async ({ page, context }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    const sent: string[] = [];
    await mockTauriInvoke(page, makeHandlers(sent));
    await page.goto("/hosts");

    await openHost(page, "alpha", 1);
    // Вставка в неподключённую сессию не идёт — доводим её до рабочего
    // состояния, иначе проверялось бы не то.
    await emitTauriEvent(page, "session:status", {
      sessionId: "tabs-session-1",
      status: "CONNECTED",
      error: null,
    });
    await page.evaluate(() => navigator.clipboard.writeText("echo вставка"));

    await page.locator(".xterm-helper-textarea").last().focus();
    await page.keyboard.press("Control+Shift+V");

    // Раньше вставка отправляла байты напрямую, минуя скобочную вставку:
    // многострочный текст выполнялся построчно прямо при вставке. Теперь идёт
    // через `term.paste`, который оборачивает его, если оболочка это включила.
    await expect
      .poll(() => sent.join(""), { timeout: 3000 })
      .toContain("echo вставка");
    // Вставка одна: раньше браузерное событие paste срабатывало вдобавок к
    // нашему обработчику, и текст уходил в терминал дважды.
    await page.waitForTimeout(400);
    expect(sent.join("").split("echo вставка").length - 1).toBe(1);
  });
});

test.describe("копирование и вставка: Ctrl+Shift+C/V, Ctrl+Insert, Shift+Insert", () => {
  async function ready(page: import("@playwright/test").Page, context: import("@playwright/test").BrowserContext, sent: string[]) {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await mockTauriInvoke(page, makeHandlers(sent));
    await page.goto("/hosts");
    await openHost(page, "alpha", 1);
    await emitTauriEvent(page, "session:status", { sessionId: "tabs-session-1", status: "CONNECTED", error: null });
    await emitTauriEvent(page, "session:data", {
      event: "data",
      sessionId: "tabs-session-1",
      chunk: Array.from(new TextEncoder().encode("hello world\r\nпривет мир\r\n")),
    });
    // Считаем записи в буфер: «ровно один раз» проверяется по счётчику, а не по итогу.
    await page.evaluate(() => {
      const w = window as unknown as { __copies: string[] };
      w.__copies = [];
      const orig = navigator.clipboard.writeText.bind(navigator.clipboard);
      navigator.clipboard.writeText = (t: string) => { w.__copies.push(t); return orig(t); };
    });
    await page.locator(".xterm-helper-textarea").last().focus();
  }
  const copies = (page: import("@playwright/test").Page) =>
    page.evaluate(() => (window as unknown as { __copies: string[] }).__copies);
  const word = (page: import("@playwright/test").Page, row: number, col: number) => {
    // Ячейка ~ 8×17 px; берём центр нужной ячейки экрана терминала.
    return page.locator(".xterm-screen").last().boundingBox().then((b) => ({ x: b!.x + col * 8 + 4, y: b!.y + row * 17 + 8 }));
  };

  test("Ctrl+Shift+C копирует выделение двойным кликом один раз и снимает выделение", async ({ page, context }) => {
    await ready(page, context, []);
    const pos = await word(page, 0, 2);
    await page.mouse.dblclick(pos.x, pos.y);
    await page.keyboard.press("Control+Shift+KeyC");
    await expect.poll(() => copies(page)).toEqual(["hello"]);
    await page.keyboard.press("Control+Shift+KeyC"); // выделения уже нет
    await page.waitForTimeout(200);
    expect(await copies(page)).toEqual(["hello"]);
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("hello");
  });

  test("тройной клик копирует строку; кириллица", async ({ page, context }) => {
    await ready(page, context, []);
    const pos = await word(page, 1, 2);
    await page.mouse.click(pos.x, pos.y, { clickCount: 3 });
    await page.keyboard.press("Control+Shift+KeyC");
    await expect.poll(async () => (await copies(page)).length).toBe(1);
    expect((await copies(page))[0]).toContain("привет мир");
  });

  test("без выделения копирование не затирает буфер", async ({ page, context }) => {
    await ready(page, context, []);
    await page.evaluate(() => navigator.clipboard.writeText("keep"));
    await page.evaluate(() => ((window as unknown as { __copies: string[] }).__copies = []));
    await page.keyboard.press("Control+Shift+KeyC");
    await page.waitForTimeout(200);
    expect(await copies(page)).toEqual([]);
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("keep");
  });

  test("браузерное paste-событие сразу после Ctrl+Shift+V не вставляет второй раз", async ({ page, context }) => {
    const sent: string[] = [];
    await ready(page, context, sent);
    await page.evaluate(() => navigator.clipboard.writeText("один"));
    await page.keyboard.press("Control+Shift+KeyV");
    // WebKitGTK присылает собственное paste на ту же вставку — имитируем его.
    await page.locator(".xterm-helper-textarea").last().evaluate((el) => {
      const dt = new DataTransfer();
      dt.setData("text/plain", "один");
      el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
    });
    await expect.poll(() => sent.join("")).toContain("один");
    await page.waitForTimeout(400);
    expect(sent.join("").split("один").length - 1).toBe(1);
  });

  test("голый Ctrl+V шлёт в оболочку только ^V и ничего не вставляет", async ({ page, context }) => {
    const sent: string[] = [];
    await ready(page, context, sent);
    await page.evaluate(() => navigator.clipboard.writeText("секрет"));
    await page.keyboard.press("Control+KeyV");
    // WebKitGTK запускал ещё и собственную вставку — имитируем её.
    await page.locator(".xterm-helper-textarea").last().evaluate((el) => {
      const dt = new DataTransfer();
      dt.setData("text/plain", "секрет");
      el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
    });
    await expect.poll(() => JSON.stringify(sent)).toBe(JSON.stringify(["\u0016"]));
    await page.waitForTimeout(500);
    expect(sent.join("")).toBe("\u0016");
  });

  /** OSC 52 с текстом: так просят записать в буфер Claude Code, tmux и vim. */
  const osc52 = (page: import("@playwright/test").Page, pd: string) =>
    emitTauriEvent(page, "session:data", {
      event: "data",
      sessionId: "tabs-session-1",
      chunk: Array.from(new TextEncoder().encode(`\x1b]52;c;${pd}\x07`)),
    });
  const base64 = (text: string) => Buffer.from(text, "utf8").toString("base64");

  test("OSC 52 из программы на сервере попадает в буфер; кириллица", async ({ page, context }) => {
    await ready(page, context, []);
    await osc52(page, base64("из Claude Code"));
    await expect.poll(() => copies(page)).toEqual(["из Claude Code"]);
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("из Claude Code");
  });

  test("OSC 52: сервер не может прочитать буфер, а пустой текст его не стирает", async ({ page, context }) => {
    const sent: string[] = [];
    await ready(page, context, sent);
    await page.evaluate(() => navigator.clipboard.writeText("секрет"));
    await page.evaluate(() => ((window as unknown as { __copies: string[] }).__copies = []));
    await osc52(page, "?");
    await osc52(page, "");
    await osc52(page, "не base64!");
    await page.waitForTimeout(300);
    expect(await copies(page)).toEqual([]);
    expect(sent.join("")).not.toContain(base64("секрет"));
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("секрет");
  });

  test("OSC 52 от терминала без фокуса буфер не меняет", async ({ page, context }) => {
    await ready(page, context, []);
    await page.locator(".xterm-helper-textarea").last().blur();
    await osc52(page, base64("чужое"));
    await page.waitForTimeout(300);
    expect(await copies(page)).toEqual([]);
  });

  async function readyWithImage(
    page: import("@playwright/test").Page,
    sent: string[],
    put: (args?: any) => unknown,
  ) {
    await mockTauriInvoke(page, {
      ...makeHandlers(sent),
      clipboard_read_image: () => "cG5n",
      sftp_put_clipboard_image: put,
    });
    await page.goto("/hosts");
    await openHost(page, "alpha", 1);
    await emitTauriEvent(page, "session:status", { sessionId: "tabs-session-1", status: "CONNECTED", error: null });
    await page.locator(".xterm-helper-textarea").last().focus();
  }

  test("Ctrl+V с картинкой передаёт её на сервер и вставляет путь вместо ^V", async ({ page }) => {
    const sent: string[] = [];
    const uploads: unknown[] = [];
    await readyWithImage(page, sent, (args) => {
      uploads.push(args);
      return "/tmp/darwaza-paste-1.png";
    });
    await page.keyboard.press("Control+KeyV");
    await expect.poll(() => sent.join("")).toContain("/tmp/darwaza-paste-1.png");
    expect(uploads).toEqual([{ sessionId: "tabs-session-1", data: "cG5n" }]);
    expect(sent.join("")).not.toContain("\u0016");
    await expect(page.getByTestId("image-paste-notice")).toHaveCount(0);
  });

  test("если картинка не передалась, человек видит сообщение, а в терминал ничего не уходит", async ({ page }) => {
    const sent: string[] = [];
    await readyWithImage(page, sent, () => {
      throw new Error("errors.unexpected");
    });
    await page.keyboard.press("Control+KeyV");
    await expect(page.getByTestId("image-paste-notice")).toBeVisible();
    expect(sent.join("")).toBe("");
  });

  test("Ctrl+Insert копирует выделение и не уходит в оболочку", async ({ page, context }) => {
    const sent: string[] = [];
    await ready(page, context, sent);
    const pos = await word(page, 0, 8);
    await page.mouse.dblclick(pos.x, pos.y);
    await page.keyboard.press("Control+Insert");
    await expect.poll(() => copies(page)).toEqual(["world"]);
    expect(sent.join("")).not.toContain("\u001b");
  });

  test("текст из другого приложения вставляется через Rust, даже если WebView буфер не отдаёт", async ({ page, context }) => {
    const sent: string[] = [];
    // Как в WebKitGTK на Wayland: readText видит только «свой» текст.
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await mockTauriInvoke(page, {
      ...makeHandlers(sent),
      clipboard_read_text: () => "из браузера",
    });
    await page.addInitScript(() => {
      navigator.clipboard.readText = () => Promise.resolve("");
    });
    await page.goto("/hosts");
    await openHost(page, "alpha", 1);
    await emitTauriEvent(page, "session:status", { sessionId: "tabs-session-1", status: "CONNECTED", error: null });
    await page.locator(".xterm-helper-textarea").last().focus();

    await page.keyboard.press("Control+Shift+V");
    await expect.poll(() => sent.join("")).toContain("из браузера");
    await page.keyboard.press("Shift+Insert");
    await expect.poll(() => sent.join("").split("из браузера").length - 1).toBe(2);
    await page.waitForTimeout(300);
    expect(sent.join("").split("из браузера").length - 1).toBe(2);
  });

  test("если Rust не смог прочитать буфер, берётся WebView, а ошибка попадает в консоль", async ({ page, context }) => {
    const sent: string[] = [];
    const warnings: string[] = [];
    page.on("console", (m) => { if (m.type() === "warning") warnings.push(m.text()); });
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await mockTauriInvoke(page, {
      ...makeHandlers(sent),
      clipboard_read_text: () => { throw new Error("владелец буфера не ответил"); },
    });
    await page.goto("/hosts");
    await openHost(page, "alpha", 1);
    await emitTauriEvent(page, "session:status", { sessionId: "tabs-session-1", status: "CONNECTED", error: null });
    await page.evaluate(() => navigator.clipboard.writeText("запасной"));
    await page.locator(".xterm-helper-textarea").last().focus();

    await page.keyboard.press("Control+Shift+V");
    await expect.poll(() => sent.join("")).toContain("запасной");
    expect(warnings.join("\n")).toContain("владелец буфера не ответил");
  });

  test("Shift+Insert вставляет один раз и не шлёт Insert в оболочку", async ({ page, context }) => {
    const sent: string[] = [];
    await ready(page, context, sent);
    await page.evaluate(() => navigator.clipboard.writeText("вставка"));
    await page.keyboard.press("Shift+Insert");
    await expect.poll(() => sent.join("")).toContain("вставка");
    await page.waitForTimeout(300);
    const all = sent.join("");
    expect(all.split("вставка").length - 1).toBe(1);
    expect(all).not.toContain("\u001b[2");
  });
});

test.describe("хоткеи на кириллической раскладке", () => {
  /** Нажатие с буквой кириллицы в `key`, но физической латинской клавишей в `code`. */
  async function press(page: import("@playwright/test").Page, key: string, code: string, shift = false) {
    await page.locator(".xterm-helper-textarea").last().evaluate(
      (el, a) => {
        el.dispatchEvent(
          new KeyboardEvent("keydown", {
            key: a.key, code: a.code, ctrlKey: true, shiftKey: a.shift, bubbles: true, cancelable: true,
          }),
        );
      },
      { key, code, shift },
    );
  }

  test("Ctrl+F (ф) открывает поиск, Ctrl+Shift+V (М) не уходит в оболочку вводом", async ({ page }) => {
    const sent: string[] = [];
    await mockTauriInvoke(page, makeHandlers(sent));
    await page.goto("/hosts");
    await openHost(page, "alpha", 1);
    await emitTauriEvent(page, "session:status", {
      sessionId: "tabs-session-1", status: "CONNECTED", error: null,
    });

    await press(page, "а", "KeyF");
    await expect(page.locator("#terminal-search-input")).toBeFocused();

    const before = sent.join("");
    await press(page, "М", "KeyV", true);
    await page.waitForTimeout(300);
    expect(sent.join("")).toBe(before);
  });

  test("Ctrl+K (л) открывает палитру команд", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers([]));
    await page.goto("/hosts");
    await page.evaluate(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "л", code: "KeyK", ctrlKey: true, bubbles: true }));
    });
    await expect(page.getByRole("dialog")).toBeVisible();
  });
});

test.describe("сайдбар в режиме терминала", () => {
  test("нет списка хостов и групп; «Fayllar» открывает файловый браузер сессии", async ({ page }) => {
    await mockTauriInvoke(page, { ...makeHandlers(), sftp_list: () => [], localfs_list: () => [], localfs_home: () => "/home/u", sftp_home: () => "/root" } as never);
    await page.goto("/hosts");
    const sidebar = page.getByRole("complementary");
    await expect(sidebar.getByTestId("sidebar-files")).toHaveCount(0);
    await openHost(page, "alpha", 1);
    // Список хостов и групп из сайдбара ушёл: хост открывают на странице «Hostlar».
    await expect(sidebar.getByText("alpha")).toHaveCount(0);
    await expect(sidebar.locator("#sidebar-groups")).toHaveCount(0);
    // Пока сессия не подключилась, кнопка видна, но неактивна.
    await expect(sidebar.getByTestId("sidebar-files")).toBeDisabled();
    await emitTauriEvent(page, "session:status", { sessionId: "tabs-session-1", status: "CONNECTED", error: null });
    await expect(sidebar.getByTestId("sidebar-files")).toBeEnabled();
    await sidebar.getByTestId("sidebar-files").click();
    await expect(sidebar.getByTestId("sidebar-files")).toHaveAttribute("aria-pressed", "true");
  });
});

test.describe("Alt+стрелки переключают вкладки", () => {
  test("Alt+←/→ идут по кругу включая Kutubxona; Alt+↑/↓ без сплита — то же; до оболочки не доходят", async ({ page }) => {
    const sent: string[] = [];
    await mockTauriInvoke(page, makeHandlers(sent));
    await page.goto("/hosts");
    await openHost(page, "alpha", 1);
    await openHost(page, "beta", 2);
    for (const id of ["tabs-session-1", "tabs-session-2"]) {
      await emitTauriEvent(page, "session:status", { sessionId: id, status: "CONNECTED", error: null });
    }
    const terminals = page.locator(".xterm");
    await expect(terminals.nth(1)).toBeVisible();

    await page.locator(".xterm-helper-textarea").last().focus();
    await page.keyboard.press("Alt+ArrowRight"); // последняя → Kutubxona
    await expect(terminals.nth(0)).toBeHidden();
    await expect(terminals.nth(1)).toBeHidden();
    await page.keyboard.press("Alt+ArrowRight"); // → первая сессия
    await expect(terminals.nth(0)).toBeVisible();
    await page.keyboard.press("Alt+ArrowLeft"); // ← Kutubxona
    await expect(terminals.nth(0)).toBeHidden();
    await page.keyboard.press("Alt+ArrowDown"); // без сплита как →
    await expect(terminals.nth(0)).toBeVisible();
    await page.keyboard.press("Alt+ArrowUp"); // как ←
    await expect(terminals.nth(0)).toBeHidden();
    // Нажатия не ушли в оболочку ESC-последовательностью.
    expect(sent.join("")).not.toContain("\u001b");
  });

  test("при сплите Alt+↑/↓ переключают панели, а не вкладки", async ({ page }) => {
    await mockTauriInvoke(page, makeHandlers());
    await page.goto("/hosts");
    await openHost(page, "alpha", 1);
    await openHost(page, "beta", 2);
    await page.getByTestId("session-tab").first().click({ modifiers: ["Control"] });
    const terminals = page.locator(".xterm");
    await expect(terminals.nth(0)).toBeVisible();
    await expect(terminals.nth(1)).toBeVisible();
    await page.locator(".xterm-helper-textarea").first().focus();
    await page.keyboard.press("Alt+ArrowUp");
    await expect(terminals.nth(0)).toBeVisible();
    await expect(terminals.nth(1)).toBeVisible(); // раскладка на месте, вкладка не ушла в Kutubxona
    await expect(page.locator(`[data-session-id="tabs-session-2"] .xterm-helper-textarea`)).toBeFocused();
  });
});

test.describe("S145 — восстановление вкладок", () => {
  test("вкладки возвращаются после перезапуска", async ({ page }) => {
    // Настройки живут в состоянии обработчиков, как в настоящем backend:
    // приложение закрылось, а список открытых хостов остался в базе.
    const settings: Record<string, string> = { "session.restore_tabs": "true" };
    const handlers = {
      ...makeHandlers(),
      settings_all: () => settings,
      settings_get: (args?: any) => settings[args?.key] ?? null,
      settings_set: (args?: any) => {
        settings[args.key] = args.value;
      },
    };

    await mockTauriInvoke(page, handlers);
    await page.goto("/hosts");

    await openHost(page, "alpha", 1);
    await openHost(page, "beta", 2);

    // Приложение закрыли и открыли заново.
    await page.reload();

    await expect(page.getByTestId("session-tab")).toHaveCount(2);
  });

  test("восстановление идёт по очереди: второй хост ждёт, пока определится первый (#234)", async ({
    page,
  }) => {
    const settings: Record<string, string> = { "session.restore_tabs": "true", "session.open_hosts": "[1,2]" };
    const connects: number[] = [];
    let firstIsUp = false;
    const handlers = {
      ...makeHandlers(),
      settings_all: () => settings,
      settings_get: (args?: any) => settings[args?.key] ?? null,
      settings_set: (args?: any) => {
        settings[args.key] = args.value;
      },
      ssh_connect: (args?: any) => {
        connects.push(args.hostId);
        return { sessionId: `restore-${args.hostId}` };
      },
      // Первое подключение «висит» на вопросе о пароле, пока не отпустим.
      ssh_active_sessions: () => [
        {
          sessionId: "restore-1",
          hostId: 1,
          status: firstIsUp ? "CONNECTED" : "AUTHENTICATING",
        },
      ],
    };

    await mockTauriInvoke(page, handlers);
    await page.goto("/hosts");

    await expect(page.getByTestId("session-tab")).toHaveCount(1);
    await page.waitForTimeout(1000);
    expect(connects, "второй хост не должен подключаться, пока первый не определился").toEqual([1]);

    firstIsUp = true;
    await expect(page.getByTestId("session-tab")).toHaveCount(2);
    expect(connects).toEqual([1, 2]);
  });

  test("по умолчанию настройка выключена: чистый запуск, сохранённый список цел", async ({ page }) => {
    // Настройка никогда не включалась, а список от прежней версии лежит в базе.
    const settings: Record<string, string> = { "session.open_hosts": "[1,2]" };
    await mockTauriInvoke(page, {
      ...makeHandlers(),
      settings_all: () => settings,
      settings_get: (args?: any) => settings[args?.key] ?? null,
      settings_set: (args?: any) => {
        settings[args.key] = args.value;
      },
    });
    await page.goto("/hosts");
    await expect(page.getByTestId("session-tab")).toHaveCount(0);

    // Открыли и закрыли вкладку вручную — сохранённый список не стёрт.
    await openHost(page, "alpha", 1);
    await expect(page.getByTestId("session-tab")).toHaveCount(1);
    expect(settings["session.open_hosts"]).toBe("[1,2]");

    // В настройках переключатель выключен.
    await page.goto("/settings");
    await expect(page.getByTestId("restore-tabs")).not.toBeChecked();
  });

  test("выключенное восстановление не открывает ничего", async ({ page }) => {
    const settings: Record<string, string> = {
      "session.restore_tabs": "false",
      "session.open_hosts": "[1,2]",
    };
    const handlers = {
      ...makeHandlers(),
      settings_all: () => settings,
      settings_get: (args?: any) => settings[args?.key] ?? null,
      settings_set: (args?: any) => {
        settings[args.key] = args.value;
      },
    };

    await mockTauriInvoke(page, handlers);
    await page.goto("/hosts");

    // Список открытых хостов сохранён, но человек попросил не возвращать их.
    await expect(page.getByTestId("session-tab")).toHaveCount(0);
  });
});

test.describe("S216 — агент открывает вкладку с командой", () => {
  test("команда набрана, но не выполнена", async ({ page }) => {
    const sent: string[] = [];
    const handlers = {
      ...makeHandlers(),
      ssh_send: (args?: any) => {
        if (args?.data) sent.push(new TextDecoder().decode(Uint8Array.from(args.data)));
      },
    };
    await mockTauriInvoke(page, handlers);
    await page.goto("/hosts");
    await expect(page.getByTestId("session-tab")).toHaveCount(0);

    await emitTauriEvent(page, "agent://terminal", {
      hostId: 1,
      host: "alpha",
      command: "sudo fdisk /dev/nvme0n1",
    });

    // Вкладка открывается сама — человеку не надо её искать.
    await expect(page.getByTestId("session-tab")).toHaveCount(1);

    // Команда уходит в терминал только после подключения: в закрытый канал
    // она бы потерялась, и человек увидел бы пустое приглашение.
    await emitTauriEvent(page, "session:status", {
      sessionId: "tabs-session-1",
      status: "CONNECTED",
      error: null,
    });

    await expect.poll(() => sent.join(""), { timeout: 5000 }).toBe("sudo fdisk /dev/nvme0n1");
    // Главное: никакого перевода строки. Enter нажимает человек, и до этого
    // на сервере не происходит ничего.
    expect(sent.join("")).not.toContain("\n");
    expect(sent.join("")).not.toContain("\r");
  });
});

test.describe("macOS: сочетания на Cmd, Ctrl и Option остаются оболочке", () => {
  test.use({
    userAgent:
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)",
  });

  async function twoConnected(page: any, sent: string[]) {
    await mockTauriInvoke(page, makeHandlers(sent));
    await page.goto("/hosts");
    await openHost(page, "alpha", 1);
    await openHost(page, "beta", 2);
    for (const id of ["tabs-session-1", "tabs-session-2"]) {
      await emitTauriEvent(page, "session:status", { sessionId: id, status: "CONNECTED", error: null });
    }
    await expect(page.locator(".xterm").nth(1)).toBeVisible();
    await page.locator(".xterm-helper-textarea").last().focus();
  }

  test("⌘W закрывает вкладку, а Ctrl+W уходит в оболочку стирать слово", async ({ page }) => {
    const sent: string[] = [];
    await twoConnected(page, sent);

    await page.keyboard.press("Control+w");
    await expect(page.getByTestId("session-tab")).toHaveCount(2);
    expect(sent.join("")).toContain("\u0017");

    await page.keyboard.press("Meta+w");
    await expect(page.getByTestId("session-tab")).toHaveCount(1);
  });

  test("Option+стрелки не переключают вкладки, ⌥⌘стрелки и ⇧⌘] — переключают", async ({ page }) => {
    const sent: string[] = [];
    await twoConnected(page, sent);
    const terminals = page.locator(".xterm");

    await page.keyboard.press("Alt+ArrowLeft");
    await expect(terminals.nth(1)).toBeVisible();

    await page.keyboard.press("Alt+Meta+ArrowRight"); // последняя → Kutubxona
    await expect(terminals.nth(1)).toBeHidden();
    await page.keyboard.press("Meta+Shift+BracketRight"); // → первая
    await expect(terminals.nth(0)).toBeVisible();
  });

  test("⌘K открывает палитру, подсказка на кнопке — ⌘K", async ({ page }) => {
    await twoConnected(page, []);
    await expect(page.getByTestId("palette-open")).toContainText("⌘K");
    await page.keyboard.press("Meta+k");
    await expect(page.getByRole("dialog")).toBeVisible();
  });
});
