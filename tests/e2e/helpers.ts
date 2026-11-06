import type { Page } from "@playwright/test";

/**
 * Мокает Tauri `invoke()` для Playwright-тестов, запущенных через Vite dev (не tauri dev).
 * Создаёт `window.__TAURI_INTERNALS__.invoke`, который проксирует вызовы в Node через
 * `page.exposeFunction('__mockInvoke', ...)`.
 *
 * Пример:
 * ```ts
 * await mockTauriInvoke(page, {
 *   'hosts_list': () => [{ id: 1, name: 'test', hostname: '1.2.3.4', port: 22, username: 'root', authMethod: 'password' }],
 *   'identities_create': (args) => ({ id: 1, ...args.data }),
 * });
 * ```
 *
 * Дополнительно поднимается событийная часть Tauri (`transformCallback` +
 * `plugin:event|listen`). Без неё `listen()` падал с
 * `transformCallback is not a function`, приложение не получало ни одного
 * `session:*` события, и тесты не могли проверить ничего, что зависит от
 * статуса сессии. Отправка события из теста — `emitTauriEvent`.
 */
export async function mockTauriInvoke(
  page: Page,
  handlers: Record<string, (args?: unknown) => unknown>,
) {
  // Экспонируем Node-функцию, которую браузер сможет вызывать
  await page.exposeFunction(
    "__mockInvoke",
    async (cmd: string, args?: unknown) => {
      const handler = handlers[cmd];
      if (!handler) {
        throw new Error(`[mockTauriInvoke] No handler registered for: ${cmd}`);
      }
      return handler(args);
    },
  );

  // В inject-скрипте подменяем Tauri runtime
  await page.addInitScript(() => {
    const w = window as any;
    w.__TAURI_CALLBACKS__ = {};
    w.__TAURI_EVENT_LISTENERS__ = {};
    let callbackId = 0;

    w.__TAURI_INTERNALS__ = {
      // Настоящий Tauri отдаёт метки текущего окна и webview; без них
      // getCurrentWebview() падает, и подписка на события ОС не встаёт.
      metadata: {
        currentWindow: { label: "main" },
        currentWebview: { label: "main" },
      },
      transformCallback: (callback: (payload: unknown) => void) => {
        const id = ++callbackId;
        w.__TAURI_CALLBACKS__[id] = callback;
        return id;
      },
      invoke: (cmd: string, args?: any) => {
        // Событийный плагин обслуживаем локально: до backend он не доходит.
        if (cmd === "plugin:event|listen") {
          const list = (w.__TAURI_EVENT_LISTENERS__[args.event] ??= []);
          list.push(args.handler);
          return Promise.resolve(args.handler);
        }
        if (cmd === "plugin:event|unlisten") {
          const list = w.__TAURI_EVENT_LISTENERS__[args.event] ?? [];
          const idx = list.indexOf(args.eventId);
          if (idx >= 0) list.splice(idx, 1);
          return Promise.resolve();
        }
        return w.__mockInvoke(cmd, args);
      },
    };

    /** Доставить событие всем подписчикам — форма payload как у настоящего Tauri. */
    w.__emitTauriEvent = (event: string, payload: unknown) => {
      for (const id of w.__TAURI_EVENT_LISTENERS__[event] ?? []) {
        w.__TAURI_CALLBACKS__[id]?.({ event, id, payload });
      }
    };
  });
}

/** Отправить Tauri-событие в приложение из теста. */
export async function emitTauriEvent(
  page: Page,
  event: string,
  payload: unknown,
) {
  await page.evaluate(
    ([e, p]) => (window as any).__emitTauriEvent(e, p),
    [event, payload] as const,
  );
}
