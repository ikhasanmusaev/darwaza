import { test, expect } from "@playwright/test";
import { mockTauriInvoke, emitTauriEvent } from "./helpers";

/**
 * Съёмка скриншотов для витрины. **Не тест**, а инструмент.
 *
 * Исключён из обычного прогона в `playwright.config.ts`: он ничего не
 * проверяет, а перезаписывает картинки. Запуск вручную:
 *
 *   SHOOT=1 SHOT_DIR=/tmp/shots pnpm exec playwright test screenshots.spec.ts
 *   # в WebP: magick x.png -quality 85 x.webp
 *
 * Данные ВЫДУМАННЫЕ намеренно: настоящие адреса и логины на витрине продукта
 * про безопасность — подарок тому, кто будет её изучать.
 */
const SHOT_DIR = process.env.SHOT_DIR ?? "/tmp/darwaza-shots";

/**
 * Снимаем на всех шести языках.
 *
 * Одна картинка на все страницы подрывает ровно тот довод, который мы
 * заявляем: турок на турецкой странице видел бы узбекское меню.
 */
const LOCALES = ["uz", "ru", "en", "kk", "ky", "tr"] as const;

// Порядок действий важен дважды: xterm верстает строки по ТЕКУЩЕЙ ширине
// панели, поэтому вывод подаётся только после разделения экрана и после
// открытия файлового браузера. Иначе текст переносится посреди слова.
/**
 * Замысел плана — на языке страницы.
 *
 * Переводится отдельно от интерфейса: это текст, который на живом экране
 * пишет агент, а он пишет на языке владельца. Английская фраза на турецкой
 * странице подрывала бы ровно тот довод, ради которого снимок и делается.
 */
const AGENT_INTENT: Record<string, string> = {
  uz: "Nginx sozlamasini yangilash va xizmatni qayta yuklash",
  ru: "Обновить настройку nginx и перезагрузить службу",
  en: "Update the nginx configuration and reload the service",
  kk: "Nginx баптауын жаңартып, қызметті қайта жүктеу",
  ky: "Nginx жөндөөсүн жаңылап, кызматты кайра жүктөө",
  tr: "Nginx yapılandırmasını güncelle ve servisi yeniden yükle",
};

const G = "\u001b[32m";
const Y = "\u001b[33m";
const B = "\u001b[1;34m";
const R = "\u001b[0m";

const THEMES = ["dark", "light"] as const;

async function useLocale(page: any, locale: string) {
  await page.addInitScript((l: string) => {
    window.localStorage.setItem("locale", l);
  }, locale);
}

async function connect(page: any, name: string) {
  await page.getByTestId("host-row").filter({ hasText: name })
    .getByTestId("host-connect").click();
}

function host(id: number, name: string, hostname: string, group: number | null = 1) {
  return {
    id, groupId: group, name, hostname, port: 22, username: "deploy",
    identityId: null, authMethod: "publickey", color: null, icon: null,
    jumpHostId: null, forwardAgent: false, keepAliveInterval: 30,
    description: null, lastConnectedAt: null,
    createdAt: "2026-10-01T00:00:00Z", updatedAt: "2026-10-01T00:00:00Z",
  };
}

// Адреса — из документационных диапазонов (RFC 5737), имена — example.com.
const HOSTS = [
  host(1, "prod-01", "203.0.113.10"),
  host(2, "prod-02", "203.0.113.11"),
  host(3, "staging-01", "198.51.100.20", 2),
  host(4, "db-01", "203.0.113.30"),
];

function handlers(theme: string, extra: Record<string, unknown> = {}) {
  let n = 0;
  return {
    hosts_list: () => HOSTS,
    groups_list: () => [
      { id: 1, name: "Production", parentId: null, sortOrder: 0,
        createdAt: "2026-10-01T00:00:00Z", updatedAt: "2026-10-01T00:00:00Z" },
      { id: 2, name: "Staging", parentId: null, sortOrder: 1,
        createdAt: "2026-10-01T00:00:00Z", updatedAt: "2026-10-01T00:00:00Z" },
    ],
    snippets_list: () => [],
    settings_all: () => ({ "terminal.font.size": "14", "app.theme": theme }),
    settings_get: (a: any) =>
      a?.key === "app.theme" ? theme : a?.key === "terminal.font.size" ? "14" : null,
    ssh_active_sessions: () => [],
    ssh_connect: () => ({ sessionId: `shot-${++n}` }),
    ssh_disconnect: () => {}, ssh_send: () => {}, ssh_resize: () => {},
    master_status: () => ({ enabled: false, unlocked: false, lockoutSecondsLeft: 0, failedAttempts: 0, totpEnabled: false, minLength: 12 }),
    recordings_list: () => [], recordings_dir: () => "/rec",
    agent_plans_pending: () => [],
    agent_plan_approve: () => true, agent_plan_reject: () => true,
    ...extra,
  };
}

async function feed(page: any, sessionId: string, lines: string[]) {
  for (const line of lines) {
    await emitTauriEvent(page, "session:data", {
      event: "data", sessionId,
      chunk: Array.from(new TextEncoder().encode(line)),
    });
  }
}

const prompt = (h: string) => G + "deploy@" + h + R + ":~$ ";

async function connectAll(page: any, names: string[]) {
  for (const name of names) {
    await page.getByTestId("library-tab").click();
    await connect(page, name);
  }
  for (let i = 1; i <= names.length; i++) {
    await emitTauriEvent(page, "session:status", { event: "status", sessionId: `shot-${i}`, status: "CONNECTED" });
  }
}

/** Три панели: 50% слева, справа 25/25 — бросками вкладок, как это делает человек. */

async function settle(page: any) {
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur?.());
  await page.mouse.move(1590, 990);
  await page.waitForTimeout(500);
}

async function shoot(page: any, theme: string, name: string, locale: string) {
  const file = `${SHOT_DIR}/${locale}/${name}${theme === "light" ? "-light" : ""}.png`;
  await page.screenshot({ path: file });
}

for (const locale of LOCALES) {
  for (const theme of THEMES) {
    test(`terminal-${locale}-${theme}`, async ({ page }) => {
      await useLocale(page, locale);
      await page.setViewportSize({ width: 1600, height: 1000 });
      await mockTauriInvoke(page, handlers(theme));
      await page.goto("/hosts");
      await connectAll(page, ["prod-01", "prod-02", "staging-01"]);

      // Левая панель — prod-01; prod-02 бросаем на её правый край, затем
      // staging-01 — на нижний край prod-02. Получается 50 + 25/25.
      const tabs = page.getByTestId("session-tab");
      await tabs.nth(0).click(); // видна только активная панель — делаем ею prod-01
      await tabs.nth(1).dragTo(page.locator('[data-session-id="shot-1"]'), {
        targetPosition: { x: 1200, y: 300 },
      });
      await page.waitForTimeout(300);
      await tabs.nth(2).dragTo(page.locator('[data-session-id="shot-2"]'), {
        targetPosition: { x: 300, y: 900 },
      });
      await page.waitForTimeout(500);

      await feed(page, "shot-1", [
        prompt("prod-01") + "docker ps\r\n",
        "CONTAINER ID  IMAGE              STATUS         PORTS                  NAMES\r\n",
        "3f1c9a7b2e10  example/web:2.4.1  Up 3 days      0.0.0.0:8080->8080/tcp web\r\n",
        "a82d41c0e5f7  example/worker:2.4 Up 3 days                             worker\r\n",
        "5be0172c9d34  redis:7-alpine     Up 12 days     6379/tcp               cache\r\n",
        "c71a8e03fb62  postgres:16        Up 12 days     5432/tcp               db\r\n",
        prompt("prod-01") + "uptime\r\n",
        " 14:22:05 up 42 days,  3:11,  2 users,  load average: 0.14, 0.09, 0.08\r\n",
        prompt("prod-01") + "df -h /srv\r\n",
        "Filesystem      Size  Used Avail Use% Mounted on\r\n",
        "/dev/nvme0n1p2   64G   28G   33G  46% /srv\r\n",
        prompt("prod-01") + "ls /srv/app\r\n",
        B + "current" + R + "  " + B + "releases" + R + "  " + B + "shared" + R + "  config.yml\r\n",
        prompt("prod-01"),
      ]);
      await feed(page, "shot-2", [
        prompt("prod-02") + "systemctl status nginx --no-pager\r\n",
        G + "●" + R + " nginx.service - A high performance web server\r\n",
        "     Loaded: loaded (/lib/systemd/system/nginx.service; enabled)\r\n",
        "     Active: " + G + "active (running)" + R + " since Mon 2026-10-26 09:14:02\r\n",
        "   Main PID: 1284 (nginx)\r\n",
        "      Tasks: 5 (limit: 18760)\r\n",
        "     Memory: 12.4M\r\n",
        prompt("prod-02") + "nginx -t\r\n",
        "nginx: configuration file /etc/nginx/nginx.conf test is successful\r\n",
        prompt("prod-02"),
      ]);
      await feed(page, "shot-3", [
        prompt("staging-01") + "tail -n 6 /var/log/app.log\r\n",
        "12:41:02 INFO  worker started\r\n",
        "12:41:07 INFO  cache warmed in 4.2s\r\n",
        "12:44:13 INFO  GET /health 200 3ms\r\n",
        "12:44:41 " + Y + "WARN" + R + "  slow query 412ms\r\n",
        "12:45:02 INFO  GET /api/items 200 18ms\r\n",
        "12:45:09 INFO  deploy 2.4.1 finished\r\n",
        prompt("staging-01"),
      ]);
      await settle(page);
      await shoot(page, theme, "terminal", locale);
    });

    test(`agent-${locale}-${theme}`, async ({ page }) => {
      await useLocale(page, locale);
      await page.setViewportSize({ width: 1600, height: 1000 });
      await mockTauriInvoke(page, handlers(theme));
      await page.goto("/hosts");
      await connectAll(page, ["prod-01", "prod-02"]);
      const tabs = page.getByTestId("session-tab");
      await tabs.nth(0).click(); // видна только активная панель — делаем ею prod-01
      await tabs.nth(1).dragTo(page.locator('[data-session-id="shot-1"]'), {
        targetPosition: { x: 1200, y: 300 },
      });
      await page.waitForTimeout(400);
      await feed(page, "shot-1", [
        prompt("prod-01") + "systemctl status nginx --no-pager\r\n",
        G + "●" + R + " nginx.service - A high performance web server\r\n",
        "     Active: " + G + "active (running)" + R + " since Mon 2026-10-26 09:14:02\r\n",
        prompt("prod-01") + "nginx -T | grep -c server_name\r\n",
        "7\r\n",
        prompt("prod-01") + "ls /etc/nginx/conf.d/\r\n",
        "example.conf  upstream.conf\r\n",
        prompt("prod-01"),
      ]);
      await feed(page, "shot-2", [
        prompt("prod-02") + "curl -sI localhost | head -3\r\n",
        "HTTP/1.1 200 OK\r\n",
        "Server: nginx/1.26.2\r\n",
        prompt("prod-02"),
      ]);
      await emitTauriEvent(page, "agent://plan", {
        id: "shot-plan",
        host: "prod-01",
        hostId: 1,
        hostAddress: "203.0.113.10",
        hostUser: "deploy",
        client: "claude-mcp",
        session: "1a2b-3c4d",
        expiresAt: Date.now() + 284_000,
        intent: AGENT_INTENT[locale],
        steps: [
          { kind: "check", title: "nginx -t" },
          {
            kind: "edit",
            title: "/etc/nginx/conf.d/example.conf",
            diff: [
              { kind: "context", text: "server {" },
              { kind: "context", text: "    server_name example.com;" },
              { kind: "removed", text: "    listen 80;" },
              { kind: "added", text: "    listen 443 ssl;" },
              { kind: "added", text: "    ssl_certificate /etc/letsencrypt/live/example.com/fullchain.pem;" },
              { kind: "context", text: "}" },
            ],
          },
          {
            kind: "edit",
            title: "/etc/nginx/conf.d/upstream.conf",
            diff: [
              { kind: "removed", text: "    server 127.0.0.1:8080;" },
              { kind: "added", text: "    server 127.0.0.1:8081;" },
            ],
          },
          { kind: "service", title: "systemctl reload nginx" },
          {
            kind: "blocked",
            title: "sed -i '/deploy-old/d' /home/deploy/.ssh/authorized_keys",
            blockedReason: "изменение 'authorized_keys' через агента не выполняется",
            blockedCode: "protected_path",
            blockedArg: "authorized_keys",
          },
          { kind: "skipped", title: "systemctl restart ssh" },
        ],
      });
      await settle(page);
      await shoot(page, theme, "agent", locale);
    });
  }

  test(`sftp-${locale}`, async ({ page }) => {
    await useLocale(page, locale);
    await page.setViewportSize({ width: 1600, height: 1000 });
    await mockTauriInvoke(page, handlers("dark", {
      local_home: () => "/home/deploy",
      local_parent: () => "/home",
      local_list: () => [
        { name: "releases", path: "/home/deploy/releases", kind: "dir", size: 0 },
        { name: "app-2.4.1.tar.gz", path: "/home/deploy/app-2.4.1.tar.gz", kind: "file", size: 48210944 },
        { name: "nginx.conf", path: "/home/deploy/nginx.conf", kind: "file", size: 3120 },
        { name: "deploy.sh", path: "/home/deploy/deploy.sh", kind: "file", size: 1840 },
      ],
      sftp_home: () => "/srv/app",
      sftp_list: () => [
        { name: "current", path: "/srv/app/current", kind: "dir", size: 0 },
        { name: "releases", path: "/srv/app/releases", kind: "dir", size: 0 },
        { name: "shared", path: "/srv/app/shared", kind: "dir", size: 0 },
        { name: "app-2.4.0.tar.gz", path: "/srv/app/app-2.4.0.tar.gz", kind: "file", size: 47992832 },
        { name: "config.yml", path: "/srv/app/config.yml", kind: "file", size: 2048 },
      ],
      transfers_download_dir: () => "/home/deploy/Downloads",
    }));
    await page.goto("/hosts");
    await connect(page, "prod-01");
    await emitTauriEvent(page, "session:status", { event: "status", sessionId: "shot-1", status: "CONNECTED" });
    await page.getByTestId("sftp-toggle").click();
    await expect(page.getByTestId("remote-entry").first()).toBeVisible();
    await feed(page, "shot-1", [
      prompt("prod-01") + "ls /srv/app\r\n",
      "current  releases  shared\r\n",
      prompt("prod-01") + "df -h /srv\r\n",
      "Size  Used Avail Use%\r\n",
      " 64G   28G   33G  46%\r\n",
      prompt("prod-01"),
    ]);
    await emitTauriEvent(page, "transfer:progress", {
      id: "t-1", kind: "upload", name: "app-2.4.1.tar.gz",
      transferred: 31457280, total: 48210944, status: "RUNNING",
    });
    await emitTauriEvent(page, "transfer:progress", {
      id: "t-2", kind: "download", name: "config.yml",
      transferred: 2048, total: 2048, status: "DONE",
    });
    await settle(page);
    await shoot(page, "dark", "sftp", locale);
  });

  test(`hostkey-${locale}`, async ({ page }) => {
    await useLocale(page, locale);
    await page.setViewportSize({ width: 1600, height: 1000 });
    await mockTauriInvoke(page, handlers("dark"));
    await page.goto("/hosts");
    await connect(page, "prod-01");
    await emitTauriEvent(page, "session:host_key_prompt", {
      sessionId: "shot-1",
      hostname: "203.0.113.10:22",
      keyType: "ssh-ed25519",
      fingerprint: "SHA256:9Yk3xQvT2mR8pL4nW7sD1cF6hJ0bV5gA3eZ8uK2yX4M",
      kind: "changed",
      previousFingerprint: "SHA256:4Bn7mK1qW9zR3tY6uP0sX2vC8dG5hL7jN4aE1fT9wQ6",
    });
    await settle(page);
    await shoot(page, "dark", "hostkey", locale);
  });
}
