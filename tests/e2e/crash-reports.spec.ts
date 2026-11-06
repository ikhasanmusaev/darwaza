import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

function makeHandlers(state: { enabled: boolean; reports: any[]; toggles: boolean[] }) {
  return {
    hosts_list: () => [],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    settings_set: () => {},
    ssh_active_sessions: () => [],
    master_status: () => ({
      enabled: false,
      unlocked: false,
      lockoutSecondsLeft: 0,
      failedAttempts: 0,
    }),

    crash_reports_enabled: () => state.enabled,
    crash_reports_dir: () => "/home/tester/.local/state/darwaza/crashes",
    crash_reports_list: () => state.reports,
    crash_reports_set_enabled: (args: any) => {
      state.enabled = args.enabled;
      state.toggles.push(args.enabled);
    },
    crash_report_read: () =>
      "Darwaza crash report\nversion: 0.1.0\npanic: test panic\n",
    crash_report_remove: (args: any) => {
      state.reports = state.reports.filter((r) => r.path !== args.path);
    },
    crash_endpoint_get: () => state.endpoint ?? null,
    crash_endpoint_set: (args: any) => {
      if (args.url && !args.url.startsWith("http")) {
        throw new Error("errors.invalidUrl");
      }
      state.endpoint = args.url;
    },
    crash_report_send: (args: any) => {
      state.sent.push(args.path);
      return 200;
    },
    crash_report_export: () => "/home/tester/Downloads/crash-1755000000.txt",
  };
}

test.describe("S21 — Crash reports", () => {
  test("collection is off by default and can be enabled", async ({ page }) => {
    const state = { enabled: false, reports: [] as any[], toggles: [] as boolean[], endpoint: "", sent: [] as string[] };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/settings");
    await page.getByRole("button", { name: "Hisobotlar", exact: true }).click();

    // Сбор выключен по умолчанию — это принципиально, а не деталь
    const toggle = page.getByTestId("crash-toggle");
    await expect(toggle).not.toBeChecked();

    await toggle.click();
    await expect.poll(() => state.toggles).toEqual([true]);
    await expect(toggle).toBeChecked();
  });

  test("shows report contents before it is shared anywhere", async ({ page }) => {
    const state = {
      enabled: true,
      reports: [
        {
          name: "crash-1755000000.txt",
          path: "/crashes/crash-1755000000.txt",
          size: 2048,
          modified: 1755000000,
        },
      ] as any[],
      toggles: [] as boolean[],
      endpoint: "",
      sent: [] as string[],
    };
    await mockTauriInvoke(page, makeHandlers(state));

    await page.goto("/settings");
    await page.getByRole("button", { name: "Hisobotlar", exact: true }).click();
    const list = page.getByTestId("crash-list");
    await expect(list).toContainText("crash-1755000000.txt");

    // Пользователь должен видеть содержимое до того, как приложит файл куда-либо
    await list.getByTitle("Посмотреть").or(list.getByTitle("Ko'rish")).first().click();
    const viewer = page.getByTestId("crash-viewer");
    await expect(viewer).toBeVisible();
    await expect(viewer).toContainText("panic: test panic");
  });

  test("send button appears only once a destination is configured", async ({ page }) => {
    const state = {
      enabled: true,
      reports: [
        {
          name: "crash-1755000000.txt",
          path: "/crashes/crash-1755000000.txt",
          size: 2048,
          modified: 1755000000,
        },
      ] as any[],
      toggles: [] as boolean[],
      endpoint: "",
      sent: [] as string[],
    };
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/settings");
    await page.getByRole("button", { name: "Hisobotlar", exact: true }).click();

    // Без адреса отправлять некуда — кнопки нет
    await expect(page.getByTestId("crash-send")).toHaveCount(0);
    // Сохранить копию можно всегда: для этого сервер не нужен
    await expect(page.getByTestId("crash-export")).toBeVisible();

    await page.getByTestId("crash-endpoint").fill("https://example.com/hook");
    await page.getByTestId("crash-endpoint").blur();

    const sendButton = page.getByTestId("crash-send");
    await expect(sendButton).toBeVisible();
    await sendButton.click();
    await expect.poll(() => state.sent).toEqual(["/crashes/crash-1755000000.txt"]);
    await expect(page.getByTestId("crash-note")).toContainText("200");
  });

  test("a malformed destination is refused before it is ever needed", async ({ page }) => {
    const state = {
      enabled: true,
      reports: [] as any[],
      toggles: [] as boolean[],
      endpoint: "",
      sent: [] as string[],
    };
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/settings");
    await page.getByRole("button", { name: "Hisobotlar", exact: true }).click();
    // Дожидаемся загрузки секции: ввод до её завершения раньше затирался,
    // и тест падал через раз именно поэтому.
    await expect(page.getByTestId("crash-section-ready")).toBeVisible();

    await page.getByTestId("crash-endpoint").fill("example.com/hook");
    await page.getByTestId("crash-endpoint").blur();

    // Про опечатку узнаём в настройках, а не в момент сбоя
    // Backend отдаёт код, текст берётся из локали (#177).
    await expect(page.getByText(/http:\/\/ yoki https:\/\//)).toBeVisible();
    expect(state.endpoint).toBe("");
  });
});
