<script setup lang="ts">
import { ref, onMounted, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { ChevronRight, Fingerprint, Terminal, Globe, Monitor, Info } from "lucide-vue-next";
import { useSettingsStore } from "../stores/settings";
import { TERMINAL_THEMES } from "../lib/terminal-themes";
import { applyAppTheme } from "../lib/theme";
import MasterPasswordSection from "../components/MasterPasswordSection.vue";
import SecretsPlaceSection from "../components/SecretsPlaceSection.vue";
import CrashReportsSection from "../components/CrashReportsSection.vue";
import PageHeader from "../components/PageHeader.vue";
import SyncAccountSection from "../components/SyncAccountSection.vue";
import { setLocale, type Locale } from "../i18n";
import { getVersion } from "@tauri-apps/api/app";
import { fontsApi } from "../api/settings";

const { t, locale } = useI18n();
const router = useRouter();
const settings = useSettingsStore();

/**
 * Версия берётся у приложения, а не пишется в разметке: вписанная руками, она
 * расходится с настоящей на первом же выпуске, и человек в отчёте о сбое
 * назовёт не ту.
 */
const appVersion = ref("");
const availableFonts = ref<string[]>([]);

onMounted(async () => {
  settings.ensureLoaded();
  availableFonts.value = await fontsApi.list().catch(() => []);
  // Отказ не показываем: раздел «О программе» — не то место, где стоит
  // тревожить человека сообщением об ошибке.
  appVersion.value = await getVersion().catch(() => "");
});

const fontFamily = ref("");
const fontSize = ref(14);
const terminalTheme = ref("dark-default");
const defaultKeepAlive = ref(30);
const appTheme = ref("dark");
const activeSection = ref("account");
const hostsProbe = ref(false);
const decisionNotify = ref(true);

function loadFromStore() {
  fontFamily.value = settings.get("terminal.font.family") ?? "JetBrains Mono";
  fontSize.value = parseInt(settings.get("terminal.font.size") ?? "14", 10);
  terminalTheme.value = settings.get("terminal.theme") ?? "dark-default";
  defaultKeepAlive.value = parseInt(settings.get("ssh.default_keep_alive") ?? "30", 10);
  appTheme.value = settings.get("app.theme") ?? "dark";
  hostsProbe.value = settings.get("hosts.probe") === "1";
  decisionNotify.value = settings.get("notify.decision") !== "0";
}

watch(
  () => settings.initialized,
  (v) => {
    if (v) loadFromStore();
  },
  { immediate: true },
);

async function saveFontFamily() {
  await settings.set("terminal.font.family", fontFamily.value.trim() || "JetBrains Mono");
}

async function saveFontSize() {
  const val = Math.max(6, Math.min(72, fontSize.value));
  fontSize.value = val;
  await settings.set("terminal.font.size", String(val));
}

async function saveTerminalTheme() {
  await settings.set("terminal.theme", terminalTheme.value);
}

async function saveKeepAlive() {
  const val = Math.max(1, Math.min(3600, defaultKeepAlive.value));
  defaultKeepAlive.value = val;
  await settings.set("ssh.default_keep_alive", String(val));
}

async function saveHostsProbe() {
  await settings.set("hosts.probe", hostsProbe.value ? "1" : "0");
}

async function saveDecisionNotify() {
  await settings.set("notify.decision", decisionNotify.value ? "1" : "0");
}

async function saveAppTheme() {
  await settings.set("app.theme", appTheme.value);
  applyAppTheme(appTheme.value);
}

/**
 * Язык интерфейса. После удаления дропдауна из сайдбара сменить его можно было
 * только через командную палитру (Ctrl+K) — неочевидно, поэтому селектор живёт здесь.
 * Значение хранит сам `setLocale` (localStorage), в settings-таблицу не дублируем.
 */
const language = ref<Locale>(locale.value as Locale);

/**
 * Возвращать ли вкладки при запуске (#145).
 *
 * По умолчанию включено: человек закрывает приложение с четырьмя открытыми
 * серверами и ждёт увидеть их снова. Выключается для тех, кому каждый запуск
 * важно начинать с чистого листа.
 */
const restoreTabs = ref(settings.get("session.restore_tabs") !== "false");

/**
 * Проверка обновлений (#164).
 *
 * Только по кнопке. Приложение обязано работать без сети, и лезть в неё при
 * запуске без спроса ему незачем: человек, поставивший его на машину без
 * интернета, не должен видеть ошибок из-за возможности, которой не просил.
 *
 * Скачанное обновление проверяется подписью — ключ зашит в приложение при
 * сборке. Подменённый файл не установится, даже если подменить и сам сервер.
 */
type UpdateState = "idle" | "checking" | "downloading" | "none" | "failed" | "ready";
const updateState = ref<UpdateState>("idle");
const updateMessage = ref("");

async function checkForUpdates() {
  updateState.value = "checking";
  updateMessage.value = t("settings.updateChecking");
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();

    if (!update) {
      updateState.value = "none";
      updateMessage.value = t("settings.updateNone");
      return;
    }

    updateState.value = "downloading";
    updateMessage.value = t("settings.updateDownloading", { version: update.version });
    await update.downloadAndInstall();

    updateState.value = "ready";
    updateMessage.value = t("settings.updateReady");
  } catch (e) {
    updateState.value = "failed";
    // Отдельного словаря здесь нет намеренно: причин у неудачной проверки
    // немного и все они внешние — нет сети, сервер обновлений недоступен.
    updateMessage.value = t("settings.updateFailed");
    console.error("[updater]", e);
  }
}
async function saveRestoreTabs() {
  await settings.set("session.restore_tabs", restoreTabs.value ? "true" : "false");
}
function saveLanguage() {
  setLocale(language.value);
}
</script>

<template>
  <div class="p-6">
    <PageHeader :title="t('nav.settings')" :description="t('settings.subtitle')" />

    <div class="flex items-start gap-6">
      <nav class="w-52 shrink-0 space-y-1" :aria-label="t('nav.settings')">
        <button
          v-for="section in [
            ['account', 'settings.sectionAccount'],
            ['terminal', 'settings.terminal'],
            ['ssh', 'settings.ssh'],
            ['appearance', 'settings.appearance'],
            ['security', 'settings.sectionSecurity'],
            ['hosts', 'knownHosts.title'],
            ['reports', 'settings.sectionReports'],
            ['about', 'settings.about'],
          ]"
          :key="section[0]"
          type="button"
          class="w-full px-3 h-9 rounded-r8 text-left text-body"
          :class="
            activeSection === section[0]
              ? 'bg-surface-alt text-on-surface font-semibold'
              : 'text-on-surface-muted hover:bg-surface-alt hover:text-on-surface'
          "
          :aria-current="activeSection === section[0] ? 'page' : undefined"
          @click="activeSection = section[0]"
        >
          {{ t(section[1]) }}
        </button>
      </nav>

      <div class="flex-1 min-w-0">
        <div v-show="activeSection === 'account'">
          <SyncAccountSection />
        </div>

    <!-- Terminal settings -->
    <section v-show="activeSection === 'terminal'" class="bg-surface-card border border-edge rounded-lg p-4 mb-4">
      <div class="flex items-center gap-2 mb-4">
        <Terminal :size="18" class="text-on-surface-muted" />
        <h3 class="text-sm font-medium text-on-surface">{{ t("settings.terminal") }}</h3>
      </div>

      <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
        <div>
          <label class="block text-xs text-on-surface-muted mb-1">{{
            t("settings.terminalFont")
          }}</label>
          <input
            v-model="fontFamily"
            type="text"
            list="system-fonts"
            class="w-full bg-surface-input border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
            @blur="saveFontFamily"
            @keydown.enter="saveFontFamily"
          />
          <datalist id="system-fonts">
            <option v-for="font in availableFonts" :key="font" :value="font" />
          </datalist>
        </div>
        <div>
          <label class="block text-xs text-on-surface-muted mb-1">{{
            t("settings.terminalFontSize")
          }}</label>
          <input
            v-model.number="fontSize"
            type="number"
            min="6"
            max="72"
            class="w-full bg-surface-input border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
            @blur="saveFontSize"
            @keydown.enter="saveFontSize"
          />
        </div>
        <div>
          <label class="block text-xs text-on-surface-muted mb-1">{{
            t("settings.terminalTheme")
          }}</label>
          <select
            v-model="terminalTheme"
            class="w-full bg-surface-input border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
            @change="saveTerminalTheme"
          >
            <option v-for="t in TERMINAL_THEMES" :key="t.id" :value="t.id">
              {{ t.label }}
            </option>
          </select>
        </div>
      </div>
    </section>

    <!-- SSH settings -->
    <section v-show="activeSection === 'ssh'" class="bg-surface-card border border-edge rounded-lg p-4 mb-4">
      <div class="flex items-center gap-2 mb-4">
        <Globe :size="18" class="text-on-surface-muted" />
        <h3 class="text-sm font-medium text-on-surface">{{ t("settings.ssh") }}</h3>
      </div>

      <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
        <div>
          <label class="block text-xs text-on-surface-muted mb-1">{{
            t("settings.defaultKeepAlive")
          }}</label>
          <input
            v-model.number="defaultKeepAlive"
            type="number"
            min="1"
            max="3600"
            class="w-full bg-surface-input border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
            @blur="saveKeepAlive"
            @keydown.enter="saveKeepAlive"
          />
          <p class="text-xs text-on-surface-muted mt-1">{{ t("settings.defaultKeepAliveHint") }}</p>
        </div>
      </div>

      <label class="mt-4 flex items-start gap-2 text-sm text-on-surface cursor-pointer">
        <input
          v-model="hostsProbe"
          type="checkbox"
          data-testid="hosts-probe"
          class="accent-accent mt-0.5"
          @change="saveHostsProbe"
        />
        <span>{{ t("settings.hostsProbe") }}</span>
      </label>
      <p class="mt-1 text-xs text-on-surface-muted">{{ t("settings.hostsProbeHint") }}</p>

      <label class="mt-4 flex items-start gap-2 text-sm text-on-surface cursor-pointer">
        <input
          v-model="decisionNotify"
          type="checkbox"
          data-testid="decision-notify"
          class="accent-accent mt-0.5"
          @change="saveDecisionNotify"
        />
        <span>{{ t("settings.decisionNotify") }}</span>
      </label>
      <p class="mt-1 text-xs text-on-surface-muted">{{ t("settings.decisionNotifyHint") }}</p>
    </section>

    <!-- Application theme -->
    <section v-show="activeSection === 'appearance'" class="bg-surface-card border border-edge rounded-lg p-4 mb-4">
      <div class="flex items-center gap-2 mb-4">
        <Monitor :size="18" class="text-on-surface-muted" />
        <h3 class="text-sm font-medium text-on-surface">{{ t("settings.appearance") }}</h3>
      </div>

      <div class="flex gap-4 mb-4">
        <label class="flex items-center gap-2 text-sm text-on-surface cursor-pointer">
          <input
            v-model="appTheme"
            type="radio"
            value="dark"
            class="accent-accent"
            @change="saveAppTheme"
          />
          {{ t("settings.themeDark") }}
        </label>
        <label class="flex items-center gap-2 text-sm text-on-surface cursor-pointer">
          <input
            v-model="appTheme"
            type="radio"
            value="light"
            class="accent-accent"
            @change="saveAppTheme"
          />
          {{ t("settings.themeLight") }}
        </label>
      </div>

      <div class="max-w-xs">
        <label class="block text-xs text-on-surface-muted mb-1">{{ t("settings.language") }}</label>
        <select
          v-model="language"
          data-testid="language-select"
          class="w-full bg-surface-input border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
          @change="saveLanguage"
        >
          <option value="uz">O'zbekcha</option>
          <option value="ru">Русский</option>
          <option value="en">English</option>
          <option value="kk">Қазақша</option>
          <option value="ky">Кыргызча</option>
          <option value="tr">Türkçe</option>
        </select>
      </div>

      <!-- Восстановление вкладок (#145) -->
      <label class="flex items-start gap-2 text-sm text-on-surface cursor-pointer">
        <input
          v-model="restoreTabs"
          type="checkbox"
          data-testid="restore-tabs"
          class="accent-accent mt-0.5"
          @change="saveRestoreTabs"
        />
        <span>
          {{ t("settings.restoreTabs") }}
          <span class="block text-xs text-on-surface-muted">
            {{ t("settings.restoreTabsHint") }}
          </span>
        </span>
      </label>
    </section>

    <div v-show="activeSection === 'security'">
      <SecretsPlaceSection />
      <MasterPasswordSection />
    </div>

    <div v-show="activeSection === 'reports'">
      <CrashReportsSection />
    </div>

    <!-- Navigation sections -->
    <section v-show="activeSection === 'hosts'" class="space-y-3 mb-4">
      <!-- Known hosts -->
      <button
        type="button"
        class="w-full bg-surface-card border border-edge hover:border-on-surface-faint rounded-lg p-4 text-left transition-colors"
        @click="router.push('/settings/known-hosts')"
      >
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-3">
            <div
              class="w-9 h-9 rounded bg-surface-alt flex items-center justify-center text-on-surface-muted"
            >
              <Fingerprint :size="18" />
            </div>
            <div>
              <h3 class="text-sm font-medium text-on-surface">{{ t("knownHosts.title") }}</h3>
              <p class="text-xs text-on-surface-muted">{{ t("knownHosts.description") }}</p>
            </div>
          </div>
          <ChevronRight :size="16" class="text-on-surface-muted" />
        </div>
      </button>
    </section>

    <!-- About -->
    <section v-show="activeSection === 'about'" class="bg-surface-card border border-edge rounded-lg p-4">
      <div class="flex items-center gap-2 mb-2">
        <Info :size="18" class="text-on-surface-muted" />
        <h3 class="text-sm font-medium text-on-surface">{{ t("settings.about") }}</h3>
      </div>
      <p class="text-xs text-on-surface-muted mb-3">Darwaza v{{ appVersion }}</p>

      <!-- Проверка обновлений (#164) -->
      <button
        type="button"
        data-testid="check-updates"
        class="text-sm px-3 py-1.5 rounded border border-edge hover:bg-surface-alt disabled:opacity-50"
        :disabled="updateState === 'checking' || updateState === 'downloading'"
        @click="checkForUpdates"
      >
        {{ t("settings.checkUpdates") }}
      </button>

      <p
        v-if="updateMessage"
        data-testid="update-status"
        class="text-xs text-on-surface-muted mt-2"
      >
        {{ updateMessage }}
      </p>
    </section>
      </div>
    </div>
  </div>
</template>
