<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  Settings,
  Key,
  FolderOpen,
  Terminal,
  Play,
  Network,
  Film,
  Laptop,
  Server,
} from "lucide-vue-next";
import { useSessionsStore } from "../stores/sessions";
import { useSftpStore } from "../stores/sftp";
import { useSnippetsStore } from "../stores/snippets";
import { useSyncStore } from "../stores/sync";
import type { Snippet } from "../api/snippets";
import { errorText } from "../lib/errors";
import GateMark from "./GateMark.vue";

const { t } = useI18n();
const router = useRouter();
const sessionsStore = useSessionsStore();
const sftp = useSftpStore();
const snippetsStore = useSnippetsStore();
const syncStore = useSyncStore();
const syncLabel = computed(() =>
  syncStore.status?.signedIn && syncStore.auto ? t("nav.syncOn") : t("nav.syncOff"),
);
const showTerminalTree = computed(() => sessionsStore.activeSessionId !== null);
let unlistenSync: UnlistenFn | null = null;

function navigate(path: string) {
  sessionsStore.setActive(null);
  router.push(path);
}

onMounted(async () => {
  await snippetsStore.ensureLoaded();
  unlistenSync = await syncStore.subscribe();
});
onBeforeUnmount(() => {
  unlistenSync?.();
});

/** SFTP идёт поверх живой сессии: без неё кнопка «Fayllar» неактивна. */
const canBrowseFiles = computed(() => sessionsStore.activeSession?.status === "CONNECTED");

/* ---- Быстрая отправка сниппетов (#104) ---- */

/** Секция сниппетов имеет смысл только при живой сессии — иначе слать некуда. */
const canRunSnippets = computed(() => sessionsStore.activeSession?.status === "CONNECTED");

const snippetError = ref<string | null>(null);

async function runSnippet(s: Snippet) {
  snippetError.value = null;
  try {
    const sent = await snippetsStore.runOnActiveSession(s);
    if (!sent) snippetError.value = t("snippets.noActiveSession");
  } catch (e) {
    snippetError.value = errorText(e);
  }
}
</script>

<template>
  <aside class="w-[232px] shrink-0 bg-surface-sidebar border-r border-edge flex flex-col">
    <div class="h-14 px-4 border-b border-edge-subtle flex items-center gap-2.5">
      <GateMark :size="22" class="text-accent" />
      <h1 class="text-logo text-on-surface">{{ t("app.title") }}</h1>
      <span class="ml-auto text-[11px] text-on-surface-muted">v0.1</span>
    </div>

    <nav class="flex-1 overflow-y-auto px-3 py-3">
      <!-- Режим терминала: только то, что относится к открытой сессии. Хосты
           открывают на странице «Hostlar» (двойной клик по строке). -->
      <section v-if="showTerminalTree" aria-labelledby="sidebar-session">
        <div id="sidebar-session" class="h-7 px-2 flex items-center text-overline uppercase text-on-surface-muted">
          {{ t("nav.session") }}
        </div>
        <button
          type="button"
          data-testid="sidebar-files"
          class="w-full flex items-center gap-2.5 px-3 h-8 rounded-r8 text-left text-body disabled:opacity-50 disabled:cursor-not-allowed"
          :class="
            sftp.panelOpen && canBrowseFiles
              ? 'bg-surface-alt font-semibold text-on-surface'
              : 'text-on-surface-muted hover:bg-surface-alt hover:text-on-surface'
          "
          :disabled="!canBrowseFiles"
          :aria-pressed="sftp.panelOpen && canBrowseFiles"
          :title="canBrowseFiles ? t('sftp.title') : t('nav.filesNeedSession')"
          @click="sftp.togglePanel()"
        >
          <FolderOpen :size="16" />
          <span>{{ t("sftp.title") }}</span>
        </button>
      </section>

      <div
        class="space-y-1"
        :class="showTerminalTree ? 'pt-3 mt-3 border-t border-edge-subtle' : ''"
      >
        <button
          type="button"
          class="w-full flex items-center gap-2.5 px-3 h-8 rounded-r8 text-left text-body text-on-surface-muted hover:bg-surface-alt hover:text-on-surface"
          :class="
            $route.path.startsWith('/hosts') && sessionsStore.activeSessionId === null
              ? 'bg-surface-alt font-semibold text-on-surface'
              : ''
          "
          @click="navigate('/hosts')"
        >
          <Server :size="16" />
          <span>{{ t("nav.hosts") }}</span>
        </button>

        <button
          type="button"
          class="w-full flex items-center gap-2.5 px-3 h-8 rounded-r8 text-left text-body text-on-surface-muted hover:bg-surface-alt hover:text-on-surface"
          :class="
            $route.path.startsWith('/identities') && sessionsStore.activeSessionId === null
              ? 'bg-surface-alt font-semibold text-on-surface'
              : ''
          "
          @click="navigate('/identities')"
        >
          <Key :size="16" />
          <span>{{ t("nav.keychain") }}</span>
        </button>

        <button
          type="button"
          class="w-full flex items-center gap-2.5 px-3 h-8 rounded-r8 text-left text-body text-on-surface-muted hover:bg-surface-alt hover:text-on-surface"
          :class="
            $route.path.startsWith('/snippets') && sessionsStore.activeSessionId === null
              ? 'bg-surface-alt font-semibold text-on-surface'
              : ''
          "
          @click="navigate('/snippets')"
        >
          <Terminal :size="16" />
          <span>{{ t("nav.snippets") }}</span>
        </button>

        <button
          type="button"
          class="w-full flex items-center gap-2.5 px-3 h-8 rounded-r8 text-left text-body text-on-surface-muted hover:bg-surface-alt hover:text-on-surface"
          :class="
            $route.path.startsWith('/forwards') && sessionsStore.activeSessionId === null
              ? 'bg-surface-alt font-semibold text-on-surface'
              : ''
          "
          @click="navigate('/forwards')"
        >
          <Network :size="16" />
          <span>{{ t("nav.forwards") }}</span>
        </button>

        <button
          type="button"
          class="w-full flex items-center gap-2.5 px-3 h-8 rounded-r8 text-left text-body text-on-surface-muted hover:bg-surface-alt hover:text-on-surface"
          :class="
            $route.path.startsWith('/recordings') && sessionsStore.activeSessionId === null
              ? 'bg-surface-alt font-semibold text-on-surface'
              : ''
          "
          @click="navigate('/recordings')"
        >
          <Film :size="16" />
          <span>{{ t("nav.recordings") }}</span>
        </button>

        <button
          type="button"
          class="w-full flex items-center gap-2.5 px-3 h-8 rounded-r8 text-left text-body text-on-surface-muted hover:bg-surface-alt hover:text-on-surface"
          :class="
            $route.path.startsWith('/settings') && sessionsStore.activeSessionId === null
              ? 'bg-surface-alt font-semibold text-on-surface'
              : ''
          "
          @click="navigate('/settings')"
        >
          <Settings :size="16" />
          <span>{{ t("nav.settings") }}</span>
        </button>
      </div>

      <template v-if="canRunSnippets && snippetsStore.snippets.length > 0">
        <div class="pt-3 mt-3 border-t border-edge-subtle">
          <div class="px-2 py-1 text-overline uppercase text-on-surface-muted">
            {{ t("nav.snippets") }}
          </div>
          <p v-if="snippetError" class="px-2 py-1 text-[11px] text-danger-text">
            {{ snippetError }}
          </p>
          <div class="space-y-0.5 mt-1">
            <button
              v-for="s in snippetsStore.snippets"
              :key="s.id"
              type="button"
              class="w-full flex items-center gap-2 px-2 py-1 rounded-r8 text-left text-on-surface-muted hover:bg-surface-alt hover:text-on-surface group/snippet"
              :title="s.command"
              @click="runSnippet(s)"
            >
              <span class="truncate text-xs">{{ s.name }}</span>
              <Play :size="12" class="shrink-0 ml-auto opacity-0 group-hover/snippet:opacity-100" />
            </button>
          </div>
        </div>
      </template>
    </nav>

    <div class="border-t border-edge-subtle px-4 py-3 flex items-start gap-2.5 text-on-surface-muted">
      <Laptop :size="15" class="mt-0.5 shrink-0" />
      <p class="text-[11px] leading-4 whitespace-nowrap" data-testid="local-mode-status">
        {{ t("nav.localMode") }}<span v-if="syncLabel"> · {{ syncLabel }}</span>
      </p>
    </div>
  </aside>
</template>
