import { test, expect } from "@playwright/test";
import { mockTauriInvoke, emitTauriEvent } from "./helpers";

const HOST = {
  id: 1,
  groupId: null,
  name: "twofa-host",
  hostname: "10.0.0.8",
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

function makeHandlers(state: { submitted: string[][]; cancelled: string[] }) {
  return {
    hosts_list: () => [HOST],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],
    ssh_connect: () => ({ sessionId: "twofa-session-1" }),
    ssh_disconnect: () => {},
    ssh_send: () => {},
    ssh_resize: () => {},
    master_status: () => ({
      enabled: false,
      unlocked: false,
      lockoutSecondsLeft: 0,
      failedAttempts: 0,
    }),
    ssh_submit_auth_prompt: (args: any) => {
      state.submitted.push(args.responses);
    },
    ssh_cancel_auth_prompt: (args: any) => {
      state.cancelled.push(args.sessionId);
    },
  };
}

async function connectAndPrompt(page: any) {
  await page.goto("/hosts");
  await page
    .getByTestId("host-row")
    .filter({ hasText: "twofa-host" })
    .getByRole("button", { name: "Ulanish", exact: true })
    .click();

  // Сервер спрашивает код — событие приходит тем же каналом, что и от backend
  await emitTauriEvent(page, "session:auth_prompt", {
    sessionId: "twofa-session-1",
    hostname: "10.0.0.8:22",
    name: "2FA",
    instructions: "Введите код из приложения",
    prompts: [{ prompt: "Verification code: ", echo: false }],
  });
}

/**
 * Интерактивные вопросы сервера — штатный метод SSH, а не наша настройка.
 * Пользователь его не выбирает: так настроен сервер, и клиент обязан уметь
 * ответить, иначе к такому хосту просто не подключиться.
 */
test.describe("S20 — Interactive auth (server-driven)", () => {
  test("asks for the code and sends the answer", async ({ page }) => {
    const state = { submitted: [] as string[][], cancelled: [] as string[] };
    await mockTauriInvoke(page, makeHandlers(state));
    await connectAndPrompt(page);

    const dialog = page.getByTestId("auth-prompt-dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText("Verification code");

    // Код не должен отображаться при вводе — сервер прислал echo: false
    const input = page.getByTestId("auth-prompt-input-0");
    await expect(input).toHaveAttribute("type", "password");

    await input.fill("424242");
    await page.getByTestId("auth-prompt-submit").click();

    await expect.poll(() => state.submitted).toEqual([["424242"]]);
    await expect(dialog).toHaveCount(0);
  });

  test("cancelling reports back so the handshake does not hang", async ({ page }) => {
    const state = { submitted: [] as string[][], cancelled: [] as string[] };
    await mockTauriInvoke(page, makeHandlers(state));
    await connectAndPrompt(page);

    await expect(page.getByTestId("auth-prompt-dialog")).toBeVisible();
    await page.getByRole("button", { name: "Bekor qilish" }).click();

    await expect.poll(() => state.cancelled).toEqual(["twofa-session-1"]);
    await expect(page.getByTestId("auth-prompt-dialog")).toHaveCount(0);
  });
});
