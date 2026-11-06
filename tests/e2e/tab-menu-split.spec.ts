import { test, expect, type Page } from "@playwright/test";
import { mockTauriInvoke, emitTauriEvent } from "./helpers";

/** Контекстное меню вкладки и раскладки панелей (до 4, дерево делений). */
function host(id: number, name: string) {
  return {
    id, groupId: null, name, hostname: `10.0.0.${id}`, port: 22, username: "root",
    identityId: null, authMethod: "password", color: null, icon: null, jumpHostId: null,
    forwardAgent: false, keepAliveInterval: 30, description: null, lastConnectedAt: null,
    agentAccess: false, createdAt: "2026-08-18T00:00:00Z", updatedAt: "2026-08-18T00:00:00Z",
  };
}

async function setup(page: Page) {
  let n = 0;
  const log: string[] = [];
  await mockTauriInvoke(page, {
    hosts_list: () => [host(1, "alpha")], groups_list: () => [], snippets_list: () => [],
    settings_all: () => ({}), settings_get: () => null, settings_set: () => null,
    ssh_active_sessions: () => [],
    ssh_connect: () => { log.push("connect"); return { sessionId: `m-${++n}` }; },
    ssh_disconnect: () => { log.push("disconnect"); },
    ssh_latency: () => 12, ssh_send: () => {}, ssh_resize: () => {},
    master_status: () => ({ enabled: false, unlocked: false, lockoutSecondsLeft: 0, failedAttempts: 0, minLength: 12 }),
  } as never);
  await page.goto("/hosts");
  await page.getByTestId("host-row").first().getByTestId("host-connect").click();
  await expect(page.getByTestId("session-tab")).toHaveCount(1);
  await emitTauriEvent(page, "session:status", { sessionId: "m-1", status: "CONNECTED", error: null });
  return log;
}

const pane = (page: Page, id: string) => page.locator(`[data-session-id="${id}"]`);

test.describe("контекстное меню вкладки", () => {
  test("дублировать, переименовать, закрыть другие", async ({ page }) => {
    await setup(page);
    await page.getByTestId("session-tab").first().click({ button: "right" });
    await page.getByTestId("tab-menu-duplicate").click();
    await expect(page.getByTestId("session-tab")).toHaveCount(2);

    await page.getByTestId("session-tab").first().click({ button: "right" });
    await page.getByTestId("tab-menu-rename").click();
    await page.getByTestId("tab-rename-input").fill("prod-db");
    await page.keyboard.press("Enter");
    await expect(page.getByTestId("session-tab").first()).toContainText("prod-db");

    await page.getByTestId("session-tab").first().click({ button: "right" });
    await page.getByTestId("tab-menu-close-others").click();
    await expect(page.getByTestId("session-tab")).toHaveCount(1);
    await expect(page.getByTestId("session-tab").first()).toContainText("prod-db");
  });

  test("«Переподключить» только у оборвавшейся сессии; у Kutubxona только «Закрыть другие»", async ({ page }) => {
    await setup(page);
    await page.getByTestId("session-tab").first().click({ button: "right" });
    await expect(page.getByTestId("tab-menu-reconnect")).toHaveCount(0);
    await page.keyboard.press("Escape");
    await emitTauriEvent(page, "session:status", { sessionId: "m-1", status: "FAILED", error: "x" });
    await page.getByTestId("session-tab").first().click({ button: "right" });
    await page.getByTestId("tab-menu-reconnect").click();
    await expect(page.getByTestId("session-tab")).toHaveCount(1);
    await page.getByTestId("library-tab").click({ button: "right" });
    await expect(page.getByTestId("tab-menu-duplicate")).toHaveCount(0);
    await expect(page.getByTestId("tab-menu-close-others")).toBeVisible();
  });
});

test.describe("раскладка панелей", () => {
  test("Ctrl+Shift+D делит вправо, Ctrl+Shift+E вниз: 50 + 25/25; предел 4 панели", async ({ page }) => {
    await setup(page);
    await page.locator(".xterm-helper-textarea").last().focus();
    await page.keyboard.press("Control+Shift+KeyD");
    await expect(page.getByTestId("session-tab")).toHaveCount(2);
    await emitTauriEvent(page, "session:status", { sessionId: "m-2", status: "CONNECTED", error: null });
    // Новая панель активна: делим её вниз → слева целая половина, справа две четверти.
    await page.keyboard.press("Control+Shift+KeyE");
    await expect(page.getByTestId("session-tab")).toHaveCount(3);
    const box = async (id: string) => (await pane(page, id).boundingBox())!;
    const [a, b, c] = [await box("m-1"), await box("m-2"), await box("m-3")];
    expect(Math.abs(a.width - b.width)).toBeLessThan(4); // 50 / 50 по ширине
    expect(Math.abs(a.height - (b.height + c.height))).toBeLessThan(4); // слева — целая высота
    expect(Math.abs(b.height - c.height)).toBeLessThan(4); // справа 25 / 25
    await expect(page.getByTestId("pane-divider")).toHaveCount(2);

    await page.keyboard.press("Control+Shift+KeyD");
    await expect(page.getByTestId("session-tab")).toHaveCount(4);
    await page.keyboard.press("Control+Shift+KeyD"); // пятой не будет
    await page.waitForTimeout(300);
    await expect(page.getByTestId("session-tab")).toHaveCount(4);
  });

  test("разделитель двигается и меняет размеры", async ({ page }) => {
    await setup(page);
    await page.locator(".xterm-helper-textarea").last().focus();
    await page.keyboard.press("Control+Shift+KeyD");
    await expect(page.getByTestId("pane-divider")).toHaveCount(1);
    const div = (await page.getByTestId("pane-divider").boundingBox())!;
    const before = (await pane(page, "m-1").boundingBox())!.width;
    await page.mouse.move(div.x + div.width / 2, div.y + 100);
    await page.mouse.down();
    await page.mouse.move(div.x + 200, div.y + 100, { steps: 5 });
    await page.mouse.up();
    const after = (await pane(page, "m-1").boundingBox())!.width;
    expect(after).toBeGreaterThan(before + 100);
  });

  test("закрытие панели возвращает место соседу", async ({ page }) => {
    await setup(page);
    await page.locator(".xterm-helper-textarea").last().focus();
    await page.keyboard.press("Control+Shift+KeyD");
    await expect(page.getByTestId("pane-divider")).toHaveCount(1);
    await page.getByTestId("session-tab").nth(1).click({ button: "right" });
    await page.getByTestId("tab-menu-close").click();
    await expect(page.getByTestId("pane-divider")).toHaveCount(0);
    await expect(pane(page, "m-1")).toBeVisible();
  });
});

test.describe("бросок вкладки на панель", () => {
  test("бросок в правую половину активной вкладки делит экран; центр — ничего", async ({ page }) => {
    await setup(page);
    await page.getByTestId("session-tab").first().click({ button: "right" });
    await page.getByTestId("tab-menu-duplicate").click(); // m-2 активна
    await expect(page.getByTestId("session-tab")).toHaveCount(2);
    const stage = page.getByTestId("terminal-stage");
    const tab1 = page.getByTestId("session-tab").first();

    // Центр: ничего не происходит.
    await tab1.dragTo(stage, { targetPosition: { x: 400, y: 250 } });
    await expect(page.getByTestId("pane-divider")).toHaveCount(0);

    // Правая половина: m-1 становится панелью справа от активной m-2.
    const box = (await stage.boundingBox())!;
    await tab1.dragTo(stage, { targetPosition: { x: box.width - 20, y: box.height / 2 } });
    await expect(page.getByTestId("pane-divider")).toHaveCount(1);
    const left = (await pane(page, "m-2").boundingBox())!;
    const right = (await pane(page, "m-1").boundingBox())!;
    expect(right.x).toBeGreaterThan(left.x + left.width - 4);
    expect(Math.abs(left.width - right.width)).toBeLessThan(4);
  });
});
