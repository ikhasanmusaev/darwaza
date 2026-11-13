<script setup lang="ts">
import { ref, computed, watch, onBeforeUnmount } from "vue";
import { useI18n } from "vue-i18n";
import {
  Home,
  X,
  Loader2,
  AlertTriangle,
  FolderTree,
  Circle,
  Square,
  Bookmark,
  Columns2,
  Rows2,
  Plus,
  Search,
} from "lucide-vue-next";
import TabContextMenu from "./TabContextMenu.vue";
import { useSessionsStore } from "../stores/sessions";
import { useSftpStore } from "../stores/sftp";
import { useRecordingsStore } from "../stores/recordings";
import { useBookmarksStore } from "../stores/bookmarks";
import { useLayoutStore } from "../stores/layout";
import { useCommandPaletteStore } from "../stores/command-palette";
import { sshApi, type SessionStatus } from "../api/ssh";
import { errorText } from "../lib/errors";
import { linkView } from "../lib/linkQuality";
import { shortcutLabel } from "../lib/platform";

const { t } = useI18n();
const store = useSessionsStore();
const sftp = useSftpStore();
const recordings = useRecordingsStore();
const bookmarks = useBookmarksStore();
const layout = useLayoutStore();
const palette = useCommandPaletteStore();
const splitError = ref<string | null>(null);
const recordError = ref<string | null>(null);

/** Переключить запись активной сессии (#144). */
async function toggleRecording() {
  const id = store.activeSessionId;
  if (!id) return;
  recordError.value = null;
  try {
    if (recordings.isRecording(id)) {
      await recordings.stop(id);
    } else {
      await recordings.start(id);
    }
  } catch (e) {
    recordError.value = errorText(e);
  }
}

// Кнопка файлов имеет смысл только на живой сессии — SFTP идёт поверх неё.
const canBrowseFiles = computed(
  () => store.sessions.find((s) => s.sessionId === store.activeSessionId)?.status === "CONNECTED",
);

const isLibraryActive = computed(() => store.activeSessionId === null);
const dragOverIndex = ref<number | null>(null);
const latencyMs = ref<number | null>(null);
/** Сколько замеров подряд не удалось: по ним видно, что сеть пропала. */
const latencyFailures = ref(0);
const link = computed(() => linkView(latencyMs.value, latencyFailures.value));
let latencyTimer: ReturnType<typeof setTimeout> | null = null;
/** Пока прошлый замер не вернулся, новый не шлём: иначе они копятся в очереди. */
let latencyInFlight = false;

/** Связь в порядке — спрашиваем реже, плохая — чаще, чтобы быстрее увидеть, что ожила. */
const LATENCY_EVERY_MS = 5000;
const LATENCY_DEGRADED_EVERY_MS = 2000;
/** Сам замер ждёт ответа 3 с; здесь — запас на случай, если ответ не придёт вовсе. */
const LATENCY_GIVE_UP_MS = 4000;

function latencyWithTimeout(id: string): Promise<number> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("latency timeout")), LATENCY_GIVE_UP_MS);
    sshApi.latency(id).then(
      (v) => {
        clearTimeout(timer);
        resolve(v);
      },
      (e) => {
        clearTimeout(timer);
        reject(e);
      },
    );
  });
}

async function refreshLatency() {
  const id = store.activeSessionId;
  if (!id || store.activeSession?.status !== "CONNECTED") return;
  if (latencyInFlight) return;
  latencyInFlight = true;
  try {
    const value = await latencyWithTimeout(id);
    if (store.activeSessionId === id) {
      latencyMs.value = value;
      latencyFailures.value = 0;
    }
  } catch {
    if (store.activeSessionId === id) latencyFailures.value += 1;
  } finally {
    latencyInFlight = false;
  }
}

function scheduleLatency() {
  if (!store.activeSessionId || store.activeSession?.status !== "CONNECTED") return;
  const every = link.value.quality === "good" ? LATENCY_EVERY_MS : LATENCY_DEGRADED_EVERY_MS;
  latencyTimer = setTimeout(async () => {
    await refreshLatency();
    scheduleLatency();
  }, every);
}

watch(
  () => [store.activeSessionId, store.activeSession?.status] as const,
  () => {
    if (latencyTimer) clearTimeout(latencyTimer);
    latencyTimer = null;
    latencyMs.value = null;
    latencyFailures.value = 0;
    if (store.activeSessionId && store.activeSession?.status === "CONNECTED") {
      void refreshLatency().then(scheduleLatency);
    }
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  if (latencyTimer) clearTimeout(latencyTimer);
  latencyTimer = null;
});

/** Переименование вкладки на месте: меняется только подпись вкладки, не хост. */
const renamingId = ref<string | null>(null);
const renameValue = ref("");

function startRename(id: string) {
  const tab = store.sessions.find((x) => x.sessionId === id);
  if (!tab) return;
  renameValue.value = tab.title ?? tab.hostLabel;
  renamingId.value = id;
  // Меню закрывается и возвращает фокус на вкладку — поле берём после него.
  setTimeout(() => {
    const el = document.querySelector<HTMLInputElement>('[data-testid="tab-rename-input"]');
    el?.focus();
    el?.select();
  }, 50);
}

function commitRename() {
  const id = renamingId.value;
  if (!id) return;
  renamingId.value = null;
  store.rename(id, renameValue.value);
}

function goLibrary() {
  store.setActive(null);
}

/**
 * Обычный клик — показать одну сессию. Ctrl+клик — добавить её в раскладку
 * рядом с текущей или убрать оттуда (#140).
 */
function selectSession(id: string, e: MouseEvent) {
  splitError.value = null;
  if (e.ctrlKey || e.metaKey) {
    if (!layout.togglePane(id, store.activeSessionId)) {
      splitError.value = t("layout.maxPanes", { count: layout.MAX_PANES });
      return;
    }
    store.setActive(id);
    return;
  }
  layout.reset();
  store.setActive(id);
}

/** Вызывается и по клику, и по Enter/Space на крестике — отсюда общий Event. */
async function closeSession(id: string, e: Event) {
  e.stopPropagation();
  await store.close(id);
}

/**
 * Средняя кнопка мыши закрывает вкладку (#209).
 *
 * Привычка из браузера, и её ждут: попасть колесом по вкладке проще, чем
 * крестиком, который до наведения вообще не виден.
 *
 * `auxclick`, а не `mousedown`: закрывать надо по завершённому нажатию,
 * иначе случайное задевание колеса при прокрутке уносило бы вкладку.
 */
async function closeByMiddleClick(id: string, e: MouseEvent) {
  if (e.button !== 1) return;
  e.preventDefault();
  e.stopPropagation();
  await store.close(id);
}

function statusIndicatorClass(status: SessionStatus): string {
  switch (status) {
    case "CONNECTED":
      return "bg-success";
    case "CONNECTING":
    case "AUTHENTICATING":
    case "PTY_OPENING":
      return "bg-warning";
    case "DISCONNECTING":
      return "bg-on-surface-faint";
    case "FAILED":
      return "bg-danger";
    case "CLOSED":
      return "bg-surface-alt";
  }
}

// --- Drag & Drop ---
function onDragStart(e: DragEvent, sessionId: string) {
  if (!e.dataTransfer) return;
  e.dataTransfer.setData("sessionId", sessionId);
  e.dataTransfer.effectAllowed = "move";
}

function onDragOver(e: DragEvent, index: number) {
  e.preventDefault();
  dragOverIndex.value = index;
}

function onDragLeave() {
  dragOverIndex.value = null;
}

function onDrop(e: DragEvent, toIndex: number) {
  e.preventDefault();
  const sessionId = e.dataTransfer?.getData("sessionId");
  if (sessionId) {
    store.reorder(sessionId, toIndex);
  }
  dragOverIndex.value = null;
}
</script>

<template>
  <div
    class="h-10 bg-surface border-b border-edge flex items-center px-2 gap-1 text-body overflow-x-auto"
  >
    <TabContextMenu :session-id="null" @error="splitError = $event">
    <button
      type="button"
      data-testid="library-tab"
      class="inline-flex items-center gap-1.5 px-3 h-8 rounded-r8 text-xs font-medium shrink-0"
      :class="
        isLibraryActive
          ? 'bg-surface-alt text-on-surface'
          : 'text-on-surface-muted hover:bg-surface-alt hover:text-on-surface'
      "
      @click="goLibrary"
    >
      <Home :size="16" />
      <span>{{ t("tabs.library") }}</span>
    </button>
    </TabContextMenu>

    <div v-if="dragOverIndex === 0" class="w-0.5 h-6 bg-accent rounded shrink-0" />

    <TabContextMenu
      v-for="(s, idx) in store.sessions"
      :key="s.sessionId"
      :session-id="s.sessionId"
      @rename="startRename"
      @error="splitError = $event"
    >
    <button
      type="button"
      data-testid="session-tab"
      draggable="true"
      class="group inline-flex items-center gap-1.5 px-2.5 h-8 rounded-r8 text-xs max-w-[200px] shrink-0 transition-opacity"
      :class="[
        s.sessionId === store.activeSessionId
          ? 'bg-surface-alt text-on-surface'
          : 'text-on-surface-muted hover:bg-surface-alt hover:text-on-surface',
        dragOverIndex === idx ? 'opacity-70' : '',
      ]"
      :title="s.title ?? s.hostLabel"
      @auxclick="closeByMiddleClick(s.sessionId, $event)"
      @click="selectSession(s.sessionId, $event)"
      @dragstart="onDragStart($event, s.sessionId)"
      @dragover.prevent="onDragOver($event, idx)"
      @dragleave="onDragLeave"
      @drop="onDrop($event, idx)"
    >
      <Loader2
        v-if="
          s.status === 'CONNECTING' || s.status === 'AUTHENTICATING' || s.status === 'PTY_OPENING'
        "
        :size="12"
        class="animate-spin text-warning"
      />
      <AlertTriangle v-else-if="s.status === 'FAILED'" :size="12" class="text-danger" />
      <span v-else class="w-2 h-2 rounded-full shrink-0" :class="statusIndicatorClass(s.status)" />

      <input
        v-if="renamingId === s.sessionId"
        v-model="renameValue"
        data-testid="tab-rename-input"
        class="w-28 bg-surface border border-accent rounded px-1 text-xs text-on-surface focus:outline-none"
        :aria-label="t('tabMenu.rename')"
        @click.stop
        @mousedown.stop
        @keydown.enter.stop.prevent="commitRename"
        @keydown.esc.stop.prevent="renamingId = null"
        @keydown.space.stop
        @blur="commitRename"
      />
      <span v-else class="truncate">{{ s.title ?? s.hostLabel }}</span>

      <span
        v-if="store.unread[s.sessionId] > 0 && s.sessionId !== store.activeSessionId"
        class="w-2 h-2 rounded-full bg-info shrink-0"
      />

      <span
        role="button"
        tabindex="0"
        class="ml-1 inline-flex items-center justify-center w-4 h-4 rounded text-on-surface-muted hover:bg-surface hover:text-on-surface"
        :title="t('tabs.close')"
        :aria-label="t('tabs.close')"
        @click="closeSession(s.sessionId, $event)"
        @keydown.enter.stop="closeSession(s.sessionId, $event)"
        @keydown.space.prevent.stop="closeSession(s.sessionId, $event)"
      >
        <X :size="12" />
      </span>
    </button>
    </TabContextMenu>

    <button
      type="button"
      data-testid="new-tab"
      class="inline-flex items-center justify-center w-8 h-8 rounded-r8 text-on-surface-muted hover:bg-surface-alt hover:text-on-surface shrink-0"
      :title="t('tabs.library')"
      :aria-label="t('tabs.library')"
      @click="goLibrary"
    >
      <Plus :size="16" />
    </button>

    <div class="flex-1 shrink-0" />

    <button
      v-if="layout.isSplit"
      type="button"
      data-testid="split-direction"
      class="mr-1 inline-flex items-center justify-center w-7 h-7 rounded-r6 text-on-surface-muted hover:bg-surface-alt hover:text-on-surface"
      :title="t('layout.toggleDirection')"
      @click="layout.toggleDirection()"
    >
      <component :is="layout.direction === 'row' ? Columns2 : Rows2" :size="14" />
    </button>

    <button
      v-if="canBrowseFiles"
      type="button"
      data-testid="bookmark-add"
      class="mr-1 inline-flex items-center justify-center w-7 h-7 rounded-r6 text-on-surface-muted hover:bg-surface-alt hover:text-warning"
      :title="t('bookmarks.add', { keys: shortcutLabel('Shift', 'B') })"
      @click="store.activeSessionId && bookmarks.addHere(store.activeSessionId)"
    >
      <Bookmark :size="14" />
    </button>

    <button
      v-if="canBrowseFiles"
      type="button"
      data-testid="record-toggle"
      class="mr-1 inline-flex items-center gap-1.5 px-2 h-7 rounded-r6 text-xs shrink-0"
      :class="
        store.activeSessionId && recordings.isRecording(store.activeSessionId)
          ? 'bg-danger-soft text-danger-text'
          : 'text-on-surface-muted hover:bg-surface-alt hover:text-on-surface'
      "
      :title="recordError ?? t('recordings.toggle')"
      @click="toggleRecording"
    >
      <component
        :is="
          store.activeSessionId && recordings.isRecording(store.activeSessionId) ? Square : Circle
        "
        :size="12"
      />
      <span>{{ t("recordings.short") }}</span>
    </button>

    <button
      v-if="canBrowseFiles"
      type="button"
      data-testid="sftp-toggle"
      class="mr-1 inline-flex items-center gap-1.5 px-2 h-7 rounded-r6 text-xs shrink-0"
      :class="
        sftp.panelOpen
          ? 'bg-surface-alt text-on-surface'
          : 'text-on-surface-muted hover:bg-surface-alt hover:text-on-surface'
      "
      :title="t('sftp.title')"
      @click="sftp.togglePanel()"
    >
      <FolderTree :size="14" />
      <span>{{ t("sftp.title") }}</span>
    </button>

    <button
      type="button"
      data-testid="palette-open"
      class="ml-2 inline-flex items-center gap-1.5 px-2 h-7 rounded-r6 border border-edge text-on-surface-muted hover:bg-surface-alt hover:text-on-surface shrink-0"
      :aria-label="t('palette.placeholder')"
      @click="palette.open"
    >
      <Search :size="14" />
      <kbd class="font-mono text-[10px]">{{ shortcutLabel("K") }}</kbd>
    </button>

    <div
      v-if="store.activeSession?.status === 'CONNECTED'"
      class="ml-2 mr-1 font-mono text-mono shrink-0"
      :class="{
        'text-success-text': link.quality === 'good',
        'text-warning-text': link.quality === 'slow',
        'text-danger-text': link.quality === 'bad',
      }"
      :data-quality="link.quality"
      data-testid="connection-latency"
    >
      {{ t(link.label) }}<template v-if="link.ms !== null"> · {{ link.ms }} ms</template>
    </div>
  </div>
</template>
