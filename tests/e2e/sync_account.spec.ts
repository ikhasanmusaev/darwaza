import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

/**
 * Аккаунт и синхронизация в настройках (#190-195).
 *
 * Проверяем не «раздел рисуется», а то, ради чего он есть: без аккаунта
 * ничего не мешает, вход доходит до backend, а про невосстановимый пароль
 * человек узнаёт ДО регистрации, а не после.
 */

function status(over: Record<string, unknown> = {}) {
  return {
    endpoint: "https://api.example.com/api/v1",
    email: "user@example.com",
    signedIn: false,
    pending: 0,
    lastError: null,
    cursor: 0,
    ...over,
  };
}

function makeHandlers(state: any, initial: Record<string, unknown> = {}) {
  let current = status(initial);
  return {
    hosts_list: () => [],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: (args: any) => (args?.key === "sync.auto" ? state.auto ?? null : null),
    settings_set: (args: any) => {
      if (args.key === "sync.auto") state.auto = args.value;
    },
    ssh_active_sessions: () => [],
    master_status: () => ({
      enabled: false,
      unlocked: false,
      lockoutSecondsLeft: 0,
      failedAttempts: 0,
      totpEnabled: false,
      minLength: 12,
    }),
    crash_reports_enabled: () => false,
    crash_endpoint_get: () => null,
    crash_reports_list: () => [],
    agent_plans_pending: () => [],
    sync_status: () => current,
    sync_set_endpoint: (args: any) => {
      state.endpoint = args.url;
      current = { ...current, endpoint: args.url };
    },
    sync_login: (args: any) => {
      state.login = args;
      current = { ...current, signedIn: true, email: args.email };
    },
    sync_register: (args: any) => {
      state.register = args;
      return "RRRR-2222-3333-4444-5555-6666-7777-8888";
    },
    sync_logout: () => {
      state.loggedOut = true;
      current = { ...current, signedIn: false };
    },
    sync_now: () => {
      state.synced = true;
      return { received: 3, sent: 2, conflicts: 1, failed: [], cursor: 9 };
    },
    sync_recovery_issue: (args: any) => {
      state.issued = args;
      return "AAAA-BBBB-CCCC-DDDD-EEEE-FFFF-GGGG-HHHH";
    },
    sync_recover: (args: any) => {
      state.recovered = args;
      return "1111-2222-3333-4444-5555-6666-7777-8888";
    },
  };
}

test.describe("S26 — аккаунт и синхронизация", () => {
  test("адрес сервера не спрашивают: он подставлен, а поле убрано", async ({
    page,
  }) => {
    // Раньше на видном месте стояло поле «Адрес сервера», которое человек
    // обязан был заполнить значением, известное только разработчику. Теперь
    // адрес подставлен, вход доступен сразу, а поле — для своего сервера.
    const state: any = {};
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/settings");

    await expect(page.getByTestId("sync-section")).toBeVisible();
    await expect(page.getByTestId("sync-signed-out")).toBeVisible();
    await expect(page.getByTestId("sync-endpoint")).toBeHidden();

    // Но добраться до него можно — иначе свой сервер стал бы недоступен.
    await page.getByTestId("sync-endpoint-toggle").click();
    await expect(page.getByTestId("sync-endpoint")).toBeVisible();
    await page.getByTestId("sync-endpoint").fill("https://sync.example.org/api/v1");
    await expect(page.getByTestId("sync-endpoint-warning")).toContainText(
      "https://api.example.com/api/v1",
    );
  });

  test("вход доходит до backend с введёнными данными", async ({ page }) => {
    const state: any = {};
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/settings");

    await page.getByTestId("sync-email").fill("me@example.com");
    await page.getByTestId("sync-password").fill("пароль");
    await page.getByTestId("sync-submit").click();

    await expect.poll(() => state.login?.email).toBe("me@example.com");
    expect(state.login.password).toBe("пароль");
  });

  test("про невосстановимый пароль сказано до регистрации, а не после", async ({ page }) => {
    // Это единственная вещь, которую нельзя починить потом.
    const state: any = {};
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/settings");

    await expect(page.getByTestId("sync-no-recovery")).toBeHidden();
    await page.getByTestId("sync-mode-register").click();
    await expect(page.getByTestId("sync-no-recovery")).toBeVisible();
  });

  test("ключ восстановления выдают вместе с аккаунтом, а не потом", async ({ page }) => {
    // Отложенный ключ не выпускают никогда. А без него у забытого пароля нет
    // запасного входа вовсе: сброса по почте здесь не существует.
    const state: any = {};
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/settings");

    await page.getByTestId("sync-mode-register").click();
    await page.getByTestId("sync-email").fill("me@example.com");
    await page.getByTestId("sync-password").fill("пароль");
    await page.getByTestId("sync-submit").click();

    const value = page.getByTestId("recovery-key-value");
    await expect(value).toContainText("RRRR");
    await expect(value).toContainText("8888");
  });

  /// Выпуск ключа требует пароль, а не только открытую сессию.
  test("ключ восстановления выпускается только с паролем", async ({ page }) => {
    // Перевыпуск отзывает прежнюю бумажку. По одной украденной сессии этого
    // делать нельзя — поэтому кнопка сначала спрашивает пароль.
    const state: any = {};
    await mockTauriInvoke(page, makeHandlers(state, { signedIn: true }));
    await page.goto("/settings");

    await page.getByTestId("sync-recovery-start").click();
    const confirm = page.getByTestId("sync-recovery-confirm");
    await expect(confirm).toBeDisabled();

    await page.getByTestId("sync-recovery-password").fill("пароль");
    await confirm.click();

    await expect.poll(() => state.issued?.password).toBe("пароль");
  });

  test("выпущенный ключ показывают один раз и целиком", async ({ page }) => {
    // Второго раза не будет: на сервере лежит только обёртка, открыть её без
    // ключа нельзя. Не показать ключ — значит выпустить его впустую.
    const state: any = {};
    await mockTauriInvoke(page, makeHandlers(state, { signedIn: true }));
    await page.goto("/settings");

    await page.getByTestId("sync-recovery-start").click();
    await page.getByTestId("sync-recovery-password").fill("пароль");
    await page.getByTestId("sync-recovery-confirm").click();

    // Окно показывает ключ группами, поэтому проверяем группы, а не строку
    // с дефисами: с экрана его переписывают именно так.
    const value = page.getByTestId("recovery-key-value");
    await expect(value).toContainText("AAAA");
    await expect(value).toContainText("HHHH");
  });

  test("окно ключа говорит про аккаунт, а не про мастер-пароль", async ({ page }) => {
    // Окно одно на два разных ключа. Показать рядом с ключом от аккаунта
    // «ключ действует только на этом компьютере» — значит соврать человеку
    // ровно там, где он читает внимательно.
    const state: any = {};
    await mockTauriInvoke(page, makeHandlers(state, { signedIn: true }));
    await page.goto("/settings");
    await page.getByTestId("sync-recovery-start").click();
    await page.getByTestId("sync-recovery-password").fill("пароль");
    await page.getByTestId("sync-recovery-confirm").click();

    const dialog = page.getByTestId("recovery-key-dialog");
    await expect(dialog).toBeVisible();
    // Текст про «только этот компьютер» принадлежит ключу мастер-пароля.
    await expect(dialog).not.toContainText("faqat shu kompyuterda ishlaydi");
  });

  test("забытый пароль восстанавливается ключом с бумаги", async ({ page }) => {
    const state: any = {};
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/settings");

    await page.getByTestId("sync-email").fill("me@example.com");
    await page.getByTestId("sync-recover-start").click();
    await page
      .getByTestId("sync-recovery-key")
      .fill("AAAA-BBBB-CCCC-DDDD-EEEE-FFFF-GGGG-HHHH");
    await page.getByTestId("sync-recovery-new-password").fill("новый пароль");
    await page.getByTestId("sync-recover-confirm").click();

    await expect.poll(() => state.recovered?.email).toBe("me@example.com");
    expect(state.recovered.recoveryKey).toBe("AAAA-BBBB-CCCC-DDDD-EEEE-FFFF-GGGG-HHHH");
    expect(state.recovered.newPassword).toBe("новый пароль");

    // И сразу показан НОВЫЙ ключ: использованный больше не работает, а
    // остаться без запасного входа человек не должен.
    const value = page.getByTestId("recovery-key-value");
    await expect(value).toContainText("1111");
    await expect(value).toContainText("8888");
  });

  test("после входа виден проход и его итог", async ({ page }) => {
    const state: any = {};
    await mockTauriInvoke(page, makeHandlers(state, { signedIn: true, pending: 4 }));
    await page.goto("/settings");

    await expect(page.getByTestId("sync-signed-in")).toBeVisible();
    await page.getByTestId("sync-now").click();

    await expect.poll(() => state.synced).toBe(true);
    // Итог называет, ЧТО произошло, а не «синхронизировано».
    const report = page.getByTestId("sync-report");
    await expect(report).toContainText("3");
    await expect(report).toContainText("2");
    await expect(report).toContainText("1");
  });

  test("прошлая ошибка видна, но не перекрывает работу", async ({ page }) => {
    const state: any = {};
    await mockTauriInvoke(
      page,
      makeHandlers(state, { signedIn: true, lastError: "errors.syncFailed" }),
    );
    await page.goto("/settings");

    // Backend хранит код, интерфейс показывает текст на своём языке (#178).
    await expect(page.getByTestId("sync-last-error")).toContainText(
      "Sinxronizatsiya bajarilmadi",
    );
    // И кнопка по-прежнему доступна: ошибка не должна запирать человека.
    await expect(page.getByTestId("sync-now")).toBeEnabled();
  });

  test("автосинхронизацию можно выключить", async ({ page }) => {
    // Раз она работает сама, её должно быть можно остановить: иначе человек
    // на дорогом мобильном интернете лишён выбора.
    const state: any = {};
    await mockTauriInvoke(page, makeHandlers(state, { signedIn: true }));
    await page.goto("/settings");

    const toggle = page.getByTestId("sync-auto");
    await expect(toggle).toBeChecked();

    await toggle.uncheck();
    await expect.poll(() => state.auto).toBe("0");
  });

  test("выключенная автосинхронизация видна после перезапуска", async ({ page }) => {
    // Настройка, не пережившая перезапуск, — это не настройка.
    const state: any = { auto: "0" };
    await mockTauriInvoke(page, makeHandlers(state, { signedIn: true }));
    await page.goto("/settings");

    await expect(page.getByTestId("sync-auto")).not.toBeChecked();
    // И ручная кнопка при этом остаётся: выключили расписание, а не
    // синхронизацию вообще.
    await expect(page.getByTestId("sync-now")).toBeEnabled();
  });

  test("адрес сервера сохраняется тем, что ввели", async ({ page }) => {
    const state: any = {};
    await mockTauriInvoke(page, makeHandlers(state, { endpoint: "" }));
    await page.goto("/settings");

    await page.getByTestId("sync-endpoint-toggle").click();
    await page.getByTestId("sync-endpoint").fill("https://darwaza.example/api/v1");
    await page.getByTestId("sync-endpoint-save").click();

    await expect.poll(() => state.endpoint).toBe("https://darwaza.example/api/v1");
  });
});
