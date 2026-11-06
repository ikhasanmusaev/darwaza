<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from "vue";
import { useI18n } from "vue-i18n";
import {
  X,
  ArrowUp,
  RefreshCw,
  Folder,
  FileIcon,
  Link2,
  FolderPlus,
  Pencil,
  Trash2,
  ArrowRight,
  ArrowLeft,
  Loader2,
} from "lucide-vue-next";
import { localFsApi, type LocalEntry } from "../api/localfs";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useTransfersStore } from "../stores/transfers";
import { useSftpStore } from "../stores/sftp";
import type { SftpEntry } from "../api/sftp";
import ConfirmDialog from "./ConfirmDialog.vue";
import { errorText } from "../lib/errors";

const props = defineProps<{ sessionId: string }>();
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const store = useSftpStore();

/* ---- Локальная сторона (view-state, в Pinia не выносим: живёт только с панелью) ---- */
const localPath = ref("");
const localEntries = ref<LocalEntry[]>([]);
const localLoading = ref(false);
const localError = ref<string | null>(null);
const localSelected = ref<LocalEntry | null>(null);

async function openLocal(path?: string) {
  localLoading.value = true;
  localError.value = null;
  try {
    const target = path ?? (await localFsApi.home());
    localEntries.value = await localFsApi.list(target);
    localPath.value = target;
    localSelected.value = null;
  } catch (e) {
    localError.value = errorText(e);
  } finally {
    localLoading.value = false;
  }
}

async function localUp() {
  if (!localPath.value) return;
  const parent = await localFsApi.parent(localPath.value);
  if (parent !== localPath.value) await openLocal(parent);
}

/* ---- Удалённая сторона ---- */
const remote = computed(() => store.pane(props.sessionId));

/* ---- Перетаскивание из системного файлового менеджера (#124a) ---- */

/**
 * Ссылки на области списков — по ним определяем, в какую панель бросили.
 *
 * ОС сообщает только координаты курсора, поэтому попадание считаем сами:
 * событие приходит мимо DOM, обычный `drop` на элементе тут не срабатывает.
 */
const localZone = ref<HTMLElement | null>(null);
const remoteZone = ref<HTMLElement | null>(null);
let unlistenOsDrag: (() => void) | null = null;

/** Физические пиксели ОС → CSS-пиксели страницы. */
function hits(zone: HTMLElement | null, x: number, y: number): boolean {
  if (!zone) return false;
  const dpr = window.devicePixelRatio || 1;
  const r = zone.getBoundingClientRect();
  const cx = x / dpr;
  const cy = y / dpr;
  return cx >= r.left && cx <= r.right && cy >= r.top && cy <= r.bottom;
}

async function initOsDragDrop() {
  // Панель обязана работать, даже если это API недоступно: без него теряется
  // только перетаскивание из системы, а кнопки и передачи между панелями — нет.
  let webview;
  try {
    webview = getCurrentWebview();
  } catch {
    return;
  }

  unlistenOsDrag = await webview.onDragDropEvent(async (event) => {
    const payload = event.payload;

    if (payload.type === "leave") {
      dropTarget.value = null;
      return;
    }
    if (payload.type === "enter" || payload.type === "over") {
      const { x, y } = payload.position;
      // Подсвечиваем только серверную панель: перетащить файл из системы
      // в локальную папку — работа файлового менеджера, не наша.
      dropTarget.value = hits(remoteZone.value, x, y) ? "remote" : null;
      return;
    }
    if (payload.type !== "drop") return;

    const { x, y } = payload.position;
    dropTarget.value = null;
    if (!hits(remoteZone.value, x, y)) return;

    dragError.value = null;
    for (const path of payload.paths) {
      try {
        await transfers.startUpload(props.sessionId, path, remote.value.path);
      } catch (e) {
        // Одна неудача не должна отменять остальные файлы пачки.
        dragError.value = errorText(e);
      }
    }
  });
}

onMounted(initOsDragDrop);
onBeforeUnmount(() => {
  unlistenOsDrag?.();
  unlistenOsDrag = null;
});

/* ---- Перетаскивание между панелями (#124) ---- */

const transfers = useTransfersStore();
/** Какая панель сейчас под курсором — подсвечиваем её как адресата. */
const dropTarget = ref<"local" | "remote" | null>(null);
const dragError = ref<string | null>(null);

/**
 * Что тащим. Держим в компоненте, а не в dataTransfer: там только строки,
 * а нам нужен и путь, и сторона, и тип записи.
 */
const dragging = ref<{ side: "local" | "remote"; path: string; isDir: boolean } | null>(null);

function onDragStart(side: "local" | "remote", path: string, isDir: boolean) {
  dragging.value = { side, path, isDir };
}

function onDragEnd() {
  dragging.value = null;
  dropTarget.value = null;
}

/**
 * Принимать ли дроп: только файл и только с другой стороны.
 * Папки не тащим — рекурсивная передача это отдельная история.
 */
function canDropOn(side: "local" | "remote"): boolean {
  const d = dragging.value;
  return !!d && d.side !== side && !d.isDir;
}

function onDragOver(side: "local" | "remote", e: DragEvent) {
  if (!canDropOn(side)) return;
  e.preventDefault();
  dropTarget.value = side;
}

/**
 * Дроп. Адресат — **папка, открытая в этой панели**: она и есть путь,
 * никакого выбора каталога спрашивать не нужно.
 */
async function onDrop(side: "local" | "remote") {
  const d = dragging.value;
  dropTarget.value = null;
  dragging.value = null;
  if (!d || d.side === side || d.isDir) return;

  dragError.value = null;
  try {
    if (side === "local") {
      await transfers.startDownload(props.sessionId, d.path, localPath.value);
    } else {
      await transfers.startUpload(props.sessionId, d.path, remote.value.path);
    }
  } catch (e) {
    dragError.value = errorText(e);
  }
}
const remoteSelected = ref<SftpEntry | null>(null);

watch(
  () => remote.value.path,
  () => {
    remoteSelected.value = null;
  },
);

onMounted(() => {
  openLocal();
  if (!remote.value.path) store.open(props.sessionId);
});

/* ---- Передача файлов (#123) ---- */
const transferring = ref(false);
const transferNote = ref<string | null>(null);
const transferError = ref<string | null>(null);

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/** Путь-приёмник строим из текущей директории противоположной панели. */
function joinLocal(dir: string, name: string): string {
  return dir.endsWith("/") ? `${dir}${name}` : `${dir}/${name}`;
}
function joinRemote(dir: string, name: string): string {
  return dir === "/" ? `/${name}` : `${dir.replace(/\/+$/, "")}/${name}`;
}

const canUpload = computed(() => localSelected.value?.kind === "file" && !!remote.value.path);
const canDownload = computed(() => remoteSelected.value?.kind === "file" && !!localPath.value);

async function upload() {
  const src = localSelected.value;
  if (!src || src.kind !== "file") return;
  transferring.value = true;
  transferError.value = null;
  transferNote.value = null;
  try {
    const bytes = await store.upload(
      props.sessionId,
      src.path,
      joinRemote(remote.value.path, src.name),
    );
    transferNote.value = t("sftp.uploaded", {
      name: src.name,
      size: formatBytes(bytes),
    });
  } catch (e) {
    transferError.value = errorText(e);
  } finally {
    transferring.value = false;
  }
}

async function download() {
  const src = remoteSelected.value;
  if (!src || src.kind !== "file") return;
  transferring.value = true;
  transferError.value = null;
  transferNote.value = null;
  try {
    const bytes = await store.download(
      props.sessionId,
      src.path,
      joinLocal(localPath.value, src.name),
    );
    transferNote.value = t("sftp.downloaded", {
      name: src.name,
      size: formatBytes(bytes),
    });
    await openLocal(localPath.value);
  } catch (e) {
    transferError.value = errorText(e);
  } finally {
    transferring.value = false;
  }
}

/* ---- Операции над удалённой стороной ----
 *
 * Ввод имени — инлайн-полем, а не `window.prompt`: в WebView Tauri нативные
 * JS-диалоги ведут себя по-разному на разных платформах вплоть до полного
 * игнорирования, и операция молча ничего не сделает.
 */
const opError = ref<string | null>(null);
const promptMode = ref<"mkdir" | "rename" | null>(null);
const promptValue = ref("");

function startMkdir() {
  promptMode.value = "mkdir";
  promptValue.value = "";
  opError.value = null;
}

function startRename() {
  if (!remoteSelected.value) return;
  promptMode.value = "rename";
  promptValue.value = remoteSelected.value.name;
  opError.value = null;
}

function cancelPrompt() {
  promptMode.value = null;
  promptValue.value = "";
}

async function submitPrompt() {
  const name = promptValue.value.trim();
  if (!name) return;
  const mode = promptMode.value;
  const target = remoteSelected.value;
  opError.value = null;
  try {
    if (mode === "mkdir") {
      await store.mkdir(props.sessionId, joinRemote(remote.value.path, name));
    } else if (mode === "rename" && target && name !== target.name) {
      await store.rename(props.sessionId, target.path, joinRemote(remote.value.path, name));
    }
    cancelPrompt();
  } catch (e) {
    opError.value = errorText(e);
  }
}

const toDelete = ref<SftpEntry | null>(null);

async function onConfirmDelete() {
  const target = toDelete.value;
  toDelete.value = null;
  if (!target) return;
  opError.value = null;
  try {
    await store.remove(props.sessionId, target.path, target.kind === "dir");
  } catch (e) {
    opError.value = errorText(e);
  }
}

function iconFor(kind: string) {
  if (kind === "dir") return Folder;
  if (kind === "symlink") return Link2;
  return FileIcon;
}
</script>

<template>
  <div class="h-full flex flex-col bg-surface border-l border-edge">
    <!-- Header -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-edge shrink-0">
      <h3 class="text-sm font-semibold text-on-surface">{{ t("sftp.title") }}</h3>
      <Loader2 v-if="transferring" :size="14" class="animate-spin text-warning" />
      <button
        type="button"
        class="ml-auto inline-flex items-center justify-center w-7 h-7 rounded hover:bg-surface-alt text-on-surface-muted"
        :title="t('form.cancel')"
        @click="emit('close')"
      >
        <X :size="16" />
      </button>
    </div>

    <!-- Сообщения о передаче -->
    <div
      v-if="transferError"
      class="mx-3 mt-2 bg-danger-soft border border-danger/40 text-danger-text text-xs rounded px-2 py-1.5"
    >
      {{ transferError }}
    </div>
    <div
      v-else-if="transferNote"
      class="mx-3 mt-2 bg-success-soft border border-success/30 text-success-text text-xs rounded px-2 py-1.5"
    >
      {{ transferNote }}
    </div>
    <div
      v-if="opError"
      class="mx-3 mt-2 bg-danger-soft border border-danger/40 text-danger-text text-xs rounded px-2 py-1.5"
    >
      {{ opError }}
    </div>

    <div class="flex-1 min-h-0 grid grid-cols-1 md:grid-cols-[1fr_auto_1fr] gap-0">
      <!-- ЛОКАЛЬНАЯ панель -->
      <section class="flex flex-col min-h-0 min-w-0">
        <div class="flex items-center gap-1 px-2 py-1.5 border-b border-edge">
          <span class="text-[11px] uppercase tracking-wide text-on-surface-muted shrink-0">
            {{ t("sftp.local") }}
          </span>
          <button
            type="button"
            class="ml-1 p-1 rounded hover:bg-surface-alt text-on-surface-muted"
            :title="t('sftp.up')"
            @click="localUp"
          >
            <ArrowUp :size="14" />
          </button>
          <button
            type="button"
            class="p-1 rounded hover:bg-surface-alt text-on-surface-muted"
            :title="t('sftp.refresh')"
            @click="openLocal(localPath)"
          >
            <RefreshCw :size="14" />
          </button>
          <code class="ml-1 text-[11px] text-on-surface-muted truncate" :title="localPath">
            {{ localPath }}
          </code>
        </div>

        <div
          ref="localZone"
          class="flex-1 overflow-auto min-h-0 transition-colors"
          data-testid="local-dropzone"
          :class="dropTarget === 'local' ? 'bg-accent/10 ring-1 ring-inset ring-accent' : ''"
          @dragover="onDragOver('local', $event)"
          @dragleave="dropTarget = null"
          @drop.prevent="onDrop('local')"
        >
          <p v-if="localLoading" class="px-3 py-2 text-xs text-on-surface-muted">
            {{ t("common.loading") }}
          </p>
          <p v-else-if="localError" class="px-3 py-2 text-xs text-danger-text">
            {{ localError }}
          </p>
          <button
            v-for="e in localEntries"
            v-else
            :key="e.path"
            type="button"
            draggable="true"
            data-testid="local-entry"
            @dragstart="onDragStart('local', e.path, e.kind === 'dir')"
            @dragend="onDragEnd"
            class="w-full flex items-center gap-2 px-3 py-1 text-left text-xs hover:bg-surface-alt"
            :class="
              localSelected?.path === e.path
                ? 'bg-surface-alt text-on-surface'
                : 'text-on-surface-muted'
            "
            @click="localSelected = e"
            @dblclick="e.kind === 'dir' ? openLocal(e.path) : undefined"
          >
            <component
              :is="iconFor(e.kind)"
              :size="14"
              class="shrink-0"
              :class="e.kind === 'dir' ? 'text-warning' : ''"
            />
            <span class="truncate">{{ e.name }}</span>
            <span v-if="e.kind === 'file'" class="ml-auto shrink-0 text-on-surface-muted">
              {{ formatBytes(e.size) }}
            </span>
          </button>
        </div>
      </section>

      <!-- Кнопки передачи -->
      <div class="flex md:flex-col items-center justify-center gap-2 px-2 border-x border-edge">
        <button
          type="button"
          class="p-2 rounded bg-surface-alt hover:bg-surface-card text-on-surface disabled:opacity-40"
          :title="t('sftp.upload')"
          :disabled="!canUpload || transferring"
          @click="upload"
        >
          <ArrowRight :size="16" />
        </button>
        <button
          type="button"
          class="p-2 rounded bg-surface-alt hover:bg-surface-card text-on-surface disabled:opacity-40"
          :title="t('sftp.download')"
          :disabled="!canDownload || transferring"
          @click="download"
        >
          <ArrowLeft :size="16" />
        </button>
      </div>

      <!-- УДАЛЁННАЯ панель -->
      <section class="flex flex-col min-h-0 min-w-0">
        <div class="flex items-center gap-1 px-2 py-1.5 border-b border-edge">
          <span class="text-[11px] uppercase tracking-wide text-on-surface-muted shrink-0">
            {{ t("sftp.remote") }}
          </span>
          <button
            type="button"
            class="ml-1 p-1 rounded hover:bg-surface-alt text-on-surface-muted"
            :title="t('sftp.up')"
            @click="store.goUp(props.sessionId)"
          >
            <ArrowUp :size="14" />
          </button>
          <button
            type="button"
            class="p-1 rounded hover:bg-surface-alt text-on-surface-muted"
            :title="t('sftp.refresh')"
            @click="store.refresh(props.sessionId)"
          >
            <RefreshCw :size="14" />
          </button>
          <button
            type="button"
            class="p-1 rounded hover:bg-surface-alt text-on-surface-muted"
            :title="t('sftp.newFolder')"
            @click="startMkdir"
          >
            <FolderPlus :size="14" />
          </button>
          <button
            type="button"
            class="p-1 rounded hover:bg-surface-alt text-on-surface-muted disabled:opacity-40"
            :title="t('sftp.rename')"
            :disabled="!remoteSelected"
            @click="startRename"
          >
            <Pencil :size="14" />
          </button>
          <button
            type="button"
            class="p-1 rounded hover:bg-danger-soft text-on-surface-muted hover:text-danger-text disabled:opacity-40"
            :title="t('sftp.delete')"
            :disabled="!remoteSelected"
            @click="toDelete = remoteSelected"
          >
            <Trash2 :size="14" />
          </button>
          <code class="ml-1 text-[11px] text-on-surface-muted truncate" :title="remote.path">
            {{ remote.path }}
          </code>
        </div>

        <!-- Инлайн-ввод имени для создания папки / переименования -->
        <div
          v-if="promptMode"
          class="flex items-center gap-1 px-2 py-1.5 border-b border-edge bg-surface-card"
        >
          <input
            v-model="promptValue"
            type="text"
            autofocus
            class="flex-1 min-w-0 bg-surface border border-edge rounded px-2 py-1 text-xs focus:outline-none focus:border-accent"
            :placeholder="
              promptMode === 'mkdir' ? t('sftp.newFolderPrompt') : t('sftp.renamePrompt')
            "
            @keydown.enter="submitPrompt"
            @keydown.esc="cancelPrompt"
          />
          <button
            type="button"
            class="px-2 py-1 text-xs rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-50"
            :disabled="!promptValue.trim()"
            @click="submitPrompt"
          >
            {{ t("form.save") }}
          </button>
          <button
            type="button"
            class="px-2 py-1 text-xs rounded bg-surface-alt hover:bg-surface text-on-surface"
            @click="cancelPrompt"
          >
            {{ t("form.cancel") }}
          </button>
        </div>

        <div
          ref="remoteZone"
          class="flex-1 overflow-auto min-h-0 transition-colors"
          data-testid="remote-dropzone"
          :class="dropTarget === 'remote' ? 'bg-accent/10 ring-1 ring-inset ring-accent' : ''"
          @dragover="onDragOver('remote', $event)"
          @dragleave="dropTarget = null"
          @drop.prevent="onDrop('remote')"
        >
          <p v-if="remote.loading" class="px-3 py-2 text-xs text-on-surface-muted">
            {{ t("common.loading") }}
          </p>
          <p v-else-if="remote.error" class="px-3 py-2 text-xs text-danger-text">
            {{ remote.error }}
          </p>
          <p
            v-else-if="remote.entries.length === 0"
            class="px-3 py-2 text-xs text-on-surface-muted"
          >
            {{ t("sftp.empty") }}
          </p>
          <button
            v-for="e in remote.entries"
            v-else
            :key="e.path"
            type="button"
            draggable="true"
            data-testid="remote-entry"
            @dragstart="onDragStart('remote', e.path, e.kind === 'dir')"
            @dragend="onDragEnd"
            class="w-full flex items-center gap-2 px-3 py-1 text-left text-xs hover:bg-surface-alt"
            :class="
              remoteSelected?.path === e.path
                ? 'bg-surface-alt text-on-surface'
                : 'text-on-surface-muted'
            "
            @click="remoteSelected = e"
            @dblclick="e.kind === 'dir' ? store.open(props.sessionId, e.path) : undefined"
          >
            <component
              :is="iconFor(e.kind)"
              :size="14"
              class="shrink-0"
              :class="e.kind === 'dir' ? 'text-warning' : ''"
            />
            <span class="truncate">{{ e.name }}</span>
            <span v-if="e.kind === 'file'" class="ml-auto shrink-0 text-on-surface-muted">
              {{ formatBytes(e.size) }}
            </span>
          </button>
        </div>
      </section>
    </div>

    <ConfirmDialog
      :open="toDelete !== null"
      :title="t('dialog.deleteTitle')"
      :description="
        toDelete
          ? `${toDelete.name} — ${t('dialog.deleteDescription')}`
          : t('dialog.deleteDescription')
      "
      :confirm-label="t('form.delete')"
      :cancel-label="t('form.cancel')"
      variant="danger"
      @update:open="
        (v: boolean) => {
          if (!v) toDelete = null;
        }
      "
      @confirm="onConfirmDelete"
      @cancel="toDelete = null"
    />
  </div>
</template>
