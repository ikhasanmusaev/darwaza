import { test, expect } from "@playwright/test";
import { mockTauriInvoke, emitTauriEvent } from "./helpers";

/**
 * Подтверждение планов агента (#205, #206).
 *
 * Проверяем не «диалог открылся», а то, ради чего он существует: человек
 * видит замысел, все шаги и построчную разницу — и ответ доходит до агента,
 * который в это время ждёт.
 */

const BASE = {
  hostId: 1,
  hostAddress: "10.0.0.5",
  hostUser: "deploy",
  client: "claude-mcp",
  session: "1a2b-3c4d",
  expiresAt: Date.now() + 300_000,
};

const PLAN = {
  ...BASE,
  id: "plan-1",
  host: "prod",
  intent: "Переключить nginx на 443 и перезапустить",
  steps: [
    { kind: "run", title: "nginx -t" },
    { kind: "service", title: "systemctl reload nginx" },
  ],
};

const FILE_PLAN = {
  ...BASE,
  id: "plan-2",
  host: "prod",
  intent: "правка файла (/etc/nginx/nginx.conf)",
  steps: [
    {
      kind: "edit",
      title: "записать /etc/nginx/nginx.conf",
      diff: [
        { kind: "context", text: "server {" },
        { kind: "removed", text: "    listen 80;" },
        { kind: "added", text: "    listen 443 ssl;" },
        { kind: "context", text: "}" },
      ],
    },
  ],
};

function makeHandlers(state: any, pending: unknown[] = []) {
  return {
    hosts_list: () => [{ id: 1, name: "prod", hostname: "192.0.2.99", username: "stale" }],
    groups_list: () => [],
    snippets_list: () => [],
    settings_all: () => ({}),
    settings_get: () => null,
    ssh_active_sessions: () => [],
    master_status: () => ({
      enabled: false,
      unlocked: false,
      lockoutSecondsLeft: 0,
      failedAttempts: 0,
    }),
    agent_plans_pending: () => pending,
    agent_plan_approve: (args: any) => {
      state.approved.push(args.id);
      return true;
    },
    agent_plan_reject: (args: any) => {
      state.rejected.push(args.id);
      return true;
    },
    agent_plans_reject_all: () => {
      state.rejectedAll = true;
      return 1;
    },
  };
}

function freshState() {
  return { approved: [] as string[], rejected: [] as string[], rejectedAll: false };
}

test.describe("S25 — подтверждение планов агента", () => {
  test("план показывает замысел и все шаги сразу, а не по одному", async ({ page }) => {
    // Подтверждать тридцать команд деплоя по одной невыносимо — через три
    // раза начнут жать «да» не глядя.
    const state = freshState();
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/hosts");

    await emitTauriEvent(page, "agent://plan", PLAN);

    await expect(page.getByTestId("agent-plan-dialog")).toBeVisible();
    await expect(page.getByTestId("agent-plan-intent")).toContainText(
      "Переключить nginx на 443",
    );
    await expect(page.getByTestId("agent-plan-dialog")).toContainText("deploy@10.0.0.5");
    await expect(page.getByTestId("agent-plan-dialog")).not.toContainText("192.0.2.99");
    const steps = page.getByTestId("agent-plan-steps");
    await expect(steps).toContainText("nginx -t");
    await expect(steps).toContainText("systemctl reload nginx");
  });

  test("подтверждение доходит до ожидающего агента", async ({ page }) => {
    const state = freshState();
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/hosts");
    await emitTauriEvent(page, "agent://plan", PLAN);

    await page.getByTestId("agent-plan-approve").click();

    await expect.poll(() => state.approved).toEqual(["plan-1"]);
    await expect(page.getByTestId("agent-plan-dialog")).toBeHidden();
  });

  test("отклонение тоже отвечает, а не оставляет агента ждать", async ({ page }) => {
    const state = freshState();
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/hosts");
    await emitTauriEvent(page, "agent://plan", PLAN);

    await page.getByTestId("agent-plan-reject").click();

    await expect.poll(() => state.rejected).toEqual(["plan-1"]);
    await expect(page.getByTestId("agent-plan-dialog")).toBeHidden();
  });

  test("правка файла показывается построчно, а не командой", async ({ page }) => {
    // sed -i 's/80/443/g' выглядит безобидно, пока не окажется, что в файле
    // было ещё три восьмидесятых.
    const state = freshState();
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/hosts");
    await emitTauriEvent(page, "agent://plan", FILE_PLAN);

    const diff = page.getByTestId("agent-plan-diff");
    await expect(diff).toContainText("− 	listen 80;".replace("\t", "    "));
    await expect(diff).toContainText("+ 	listen 443 ssl;".replace("\t", "    "));
    // Окружение обязательно: без него не понять, ГДЕ меняется.
    await expect(diff).toContainText("server {");
  });

  test("план, пришедший при закрытом окне, виден после запуска", async ({ page }) => {
    // Иначе он висел бы невидимым до самого предела ожидания, и человек не
    // понял бы, почему агент «завис».
    const state = freshState();
    await mockTauriInvoke(page, makeHandlers(state, [PLAN]));
    await page.goto("/hosts");

    await expect(page.getByTestId("agent-plan-dialog")).toBeVisible();
    await expect(page.getByTestId("agent-plan-intent")).toContainText("nginx");
  });

  test("«отклонить всё» закрывает очередь разом", async ({ page }) => {
    const state = freshState();
    await mockTauriInvoke(page, makeHandlers(state, [PLAN, FILE_PLAN]));
    await page.goto("/hosts");
    await expect(page.getByTestId("agent-plan-dialog")).toBeVisible();

    await page.getByTestId("agent-plan-reject-all").click();

    await expect.poll(() => state.rejectedAll).toBe(true);
    await expect(page.getByTestId("agent-plan-dialog")).toBeHidden();
  });

  test("безопасный Reject в фокусе, Enter действует только на нём", async ({ page }) => {
    const state = freshState();
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/hosts");
    await emitTauriEvent(page, "agent://plan", PLAN);

    const reject = page.getByTestId("agent-plan-reject");
    await expect(reject).toBeFocused();
    await page.keyboard.press("Enter");
    await expect.poll(() => state.rejected).toEqual(["plan-1"]);
    expect(state.approved).toEqual([]);
  });

  test("blocked и skipped видны, не запускаются, Esc только отклоняет", async ({ page }) => {
    const state = freshState();
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/hosts");
    await emitTauriEvent(page, "agent://plan", {
      ...BASE,
      id: "plan-blocked",
      host: "prod",
      intent: "Проверить конфигурацию",
      steps: [
        { kind: "check", title: "nginx -t" },
        { kind: "blocked", title: "mkfs.ext4 /dev/sdb", blockedReason: "запрещено правилами" },
        { kind: "skipped", title: "systemctl reload nginx" },
      ],
    });

    await expect(page.getByTestId("agent-plan-approve")).toBeEnabled();
    await expect(page.getByTestId("agent-plan-dialog")).toContainText("1 blocked");
    await expect(page.getByTestId("agent-plan-steps")).toContainText("skipped");
    await expect(page.getByTestId("agent-plan-dialog")).toContainText("Agent o'zini claude-mcp");
    await expect(page.getByTestId("agent-plan-dialog")).toContainText("Jurnalga yozilmoqda");

    await page.keyboard.press("Escape");
    await expect.poll(() => state.rejected).toEqual(["plan-blocked"]);
    expect(state.approved).toEqual([]);
  });

  test("истёкший план не подтверждается и закрывается через dismissGone", async ({ page }) => {
    const state = freshState();
    await mockTauriInvoke(page, makeHandlers(state));
    await page.goto("/hosts");
    await emitTauriEvent(page, "agent://plan", {
      ...PLAN,
      id: "plan-expired",
      expiresAt: Date.now() - 1000,
    });

    await expect(page.getByTestId("agent-plan-gone")).toContainText("Muddati tugadi");
    await expect(page.getByTestId("agent-plan-approve")).toBeDisabled();
    await page.getByTestId("agent-plan-dismiss").click();
    await expect(page.getByTestId("agent-plan-dialog")).toHaveCount(0);
    expect(state.approved).toEqual([]);
  });
});
