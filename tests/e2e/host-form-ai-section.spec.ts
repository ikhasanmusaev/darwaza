import { test, expect } from "@playwright/test";
import { mockTauriInvoke } from "./helpers";

/** Настройки ИИ-помощника — один сворачиваемый раздел внизу формы хоста. */
const BASE = {
  id: 1, groupId: null, name: "prod", hostname: "10.0.0.7", port: 22, username: "root",
  identityId: null, authMethod: "password", color: null, icon: null, jumpHostId: null,
  forwardAgent: false, agentAccess: false, agentSudoRead: null, agentNotes: null,
  keepAliveInterval: 30, description: null, lastConnectedAt: null,
  createdAt: "2026-08-28T00:00:00Z", updatedAt: "2026-08-28T00:00:00Z",
};

async function setup(page: import("@playwright/test").Page, host: object, calls: any[]) {
  await mockTauriInvoke(page, {
    hosts_list: () => [host], groups_list: () => [], snippets_list: () => [],
    settings_all: () => ({}), settings_get: () => null, ssh_active_sessions: () => [],
    identities_list: () => [],
    hosts_update: (a: any) => { calls.push(a); return { ...host, ...a.data }; },
  } as never);
}

test("обычный хост: раздел свёрнут, заметок и отметок нет", async ({ page }) => {
  await setup(page, BASE, []);
  await page.goto("/hosts/1/edit");
  await expect(page.getByTestId("host-ai-section")).toBeVisible();
  await expect(page.getByTestId("host-ai-toggle")).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByTestId("host-ai-body")).toBeHidden();
  await expect(page.getByTestId("host-agent-notes")).toHaveCount(0);
  await expect(page.getByTestId("host-ai-badge-access")).toHaveCount(0);
  await expect(page.getByTestId("host-ai-badge-notes")).toHaveCount(0);
});

test("раздел сворачивается и разворачивается; предупреждение о риске на месте", async ({ page }) => {
  await setup(page, BASE, []);
  await page.goto("/hosts/1/edit");
  await page.getByTestId("host-ai-toggle").click();
  await expect(page.getByTestId("host-ai-body")).toBeVisible();
  await expect(page.getByTestId("host-agent-access")).toBeVisible();
  await page.getByTestId("host-ai-toggle").click();
  await expect(page.getByTestId("host-ai-body")).toBeHidden();
});

test("с заметками: отметка видна, блок есть и очищается", async ({ page }) => {
  const calls: any[] = [];
  await setup(page, { ...BASE, agentNotes: "nginx в /etc/nginx" }, calls);
  await page.goto("/hosts/1/edit");
  await expect(page.getByTestId("host-ai-badge-notes")).toBeVisible();
  await expect(page.getByTestId("host-ai-body")).toBeHidden(); // по умолчанию свёрнут
  await page.getByTestId("host-ai-toggle").click();
  await expect(page.getByTestId("host-agent-notes")).toContainText("nginx в /etc/nginx");
  await page.getByTestId("host-agent-notes-clear").click();
  await expect(page.getByTestId("host-agent-notes")).toHaveCount(0);
  await expect(page.getByTestId("host-ai-badge-notes")).toHaveCount(0);
});

test("свёрнутый раздел не сбрасывает значения: сохраняются доступ, sudo и заметки", async ({ page }) => {
  const calls: any[] = [];
  await setup(
    page,
    { ...BASE, agentAccess: true, agentSudoRead: "ufw status", agentNotes: "заметка" },
    calls,
  );
  await page.goto("/hosts/1/edit");
  await expect(page.getByTestId("host-ai-badge-access")).toBeVisible();
  // Раздел не открывали, правим только название.
  await page.getByTestId("host-name-input").fill("prod-2");
  await page.getByTestId("host-form-submit").click();
  await expect.poll(() => calls.length).toBe(1);
  expect(calls[0].data).toMatchObject({
    name: "prod-2", agentAccess: true, agentSudoRead: "ufw status", agentNotes: "заметка",
  });
});
