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
  Eye,
  EyeOff,
  Copy,
} from "lucide-vue-next";
import { localFsApi, type LocalEntry } from "../api/localfs";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useTransfersStore } from "../stores/transfers";
import { useSftpStore } from "../stores/sftp";
import type { SftpEntry } from "../api/sftp";
import ConfirmDialog from "./ConfirmDialog.vue";
import { errorText } from "../lib/errors";
import { formatBytes, formatModified, isHidden, permString } from "../lib/files";
import { writeClipboardText } from "../api/clipboard";
import type { TreeEndpoint } from "../api/transfers";
import { useConflictsStore } from "../stores/conflicts";
import { planTransfers, type OnConflict } from "../lib/transferPlan";
import { isPicked, nextSelection, type SelectionState } from "../lib/selection";
import { isMacOS } from "../lib/platform";

const props = defineProps<{ sessionId: string }>();
const emit = defineEmits<{ close: [] }>();

const { t, locale } = useI18n();
const store = useSftpStore();

/* ---- Локальная сторона (view-state, в Pinia не выносим: живёт только с панелью) ---- */
const localPath = ref("");
const localEntries = ref<LocalEntry[]>([]);
const localLoading = ref(false);
const localError = ref<string | null>(null);
/** Выбранные записи: щелчок — одна, Ctrl/⌘ — добавить, Shift — диапазон. */
const localSel = ref<SelectionState<LocalEntry>>({ picked: [], anchor: null });

async function openLocal(path?: string) {
  localLoading.value = true;
  localError.value = null;
  try {
    const target = path ?? (await localFsApi.home());
    localEntries.value = await localFsApi.list(target);
    localPath.value = target;
    localSel.value = { picked: [], anchor: null };
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

/** Фильтр по имени в серверной панели: в папке на тысячу файлов иначе не найти. */
const filter = ref("");
watch(
  () => remote.value.path,
  () => {
    filter.value = "";
  },
);

const visibleLocal = computed(() =>
  store.showHidden ? localEntries.value : localEntries.value.filter((e) => !isHidden(e.name)),
);
const visibleRemote = computed(() => {
  const needle = filter.value.trim().toLocaleLowerCase();
  return remote.value.entries.filter(
    (e) =>
      (store.showHidden || !isHidden(e.name)) &&
      (!needle || e.name.toLocaleLowerCase().includes(needle)),
  );
});

/** Короткая подсказка «путь скопирован» и т. п. — гаснет сама. */
const note = ref<string | null>(null);
let noteTimer: ReturnType<typeof setTimeout> | null = null;
function showNote(text: string) {
  note.value = text;
  if (noteTimer) clearTimeout(noteTimer);
  noteTimer = setTimeout(() => (note.value = null), 2500);
}
onBeforeUnmount(() => {
  if (noteTimer) clearTimeout(noteTimer);
});

async function copyRemotePath() {
  const path = remoteSelected.value?.path ?? remote.value.path;
  if (!path) return;
  try {
    await writeClipboardText(path);
    showNote(t("sftp.pathCopied", { path }));
  } catch (e) {
    opError.value = errorText(e);
  }
}

async function onRemoteDblClick(e: SftpEntry) {
  await store.enter(props.sessionId, e);
}

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
    // Файл это или папка, backend смотрит сам: система отдаёт только пути.
    const items = payload.paths.map((path) => ({
      path,
      name: path.split(/[\\/]/).filter(Boolean).pop() ?? path,
      kind: "file",
    }));
    try {
      await transferItems("local", items);
    } catch (e) {
      dragError.value = errorText(e);
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
const dragging = ref<{ side: "local" | "remote"; items: Transferable[] } | null>(null);

/** Тащат выбранную запись — едут все выбранные; невыбранную — только она. */
function onDragStart(side: "local" | "remote", entry: Transferable) {
  const picked = side === "local" ? localSel.value.picked : remoteSel.value.picked;
  dragging.value = { side, items: isPicked(picked, entry) ? [...picked] : [entry] };
}

function onDragEnd() {
  dragging.value = null;
  dropTarget.value = null;
}

/** Принимать ли дроп: файл или папку, но только с другой стороны. */
function canDropOn(side: "local" | "remote"): boolean {
  const d = dragging.value;
  return !!d && d.side !== side;
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
  if (!d || d.side === side) return;

  dragError.value = null;
  try {
    await transferItems(d.side, d.items);
  } catch (e) {
    dragError.value = errorText(e);
  }
}
const remoteSel = ref<SelectionState<SftpEntry>>({ picked: [], anchor: null });
const remoteSelected = computed(() => remoteSel.value.anchor);

watch(
  () => remote.value.path,
  () => {
    remoteSel.value = { picked: [], anchor: null };
  },
);

/** Модификатор «добавить к выбору»: ⌘ на Mac, Ctrl на остальных. */
function pickMods(ev: MouseEvent) {
  return { toggle: isMacOS() ? ev.metaKey : ev.ctrlKey, range: ev.shiftKey };
}
function pickLocal(e: LocalEntry, ev: MouseEvent) {
  localSel.value = nextSelection(visibleLocal.value, localSel.value, e, pickMods(ev));
}
function pickRemote(e: SftpEntry, ev: MouseEvent) {
  remoteSel.value = nextSelection(visibleRemote.value, remoteSel.value, e, pickMods(ev));
}

onMounted(() => {
  openLocal();
  if (!remote.value.path) store.open(props.sessionId);
});

/* ---- Передача файлов (#123, #124) ----
 *
 * И кнопки, и перетаскивание идут фоновой передачей: ход и отмена — в полосе
 * внизу окна, папки передаются целиком, а одноимённый файл на месте
 * назначения не затирается — новый получает имя «файл (2)».
 */
const LOCAL: TreeEndpoint = { kind: "local" };
const remoteEnd = (): TreeEndpoint => ({ kind: "remote", sessionId: props.sessionId });

interface Transferable {
  path: string;
  name: string;
  kind: string;
}

/** Передать запись с одной стороны в папку, открытую на другой. */
async function startTransfer(
  from: "local" | "remote",
  path: string,
  isDir: boolean,
  onConflict: OnConflict,
) {
  if (from === "local") {
    await transfers.startTree(LOCAL, path, isDir, remoteEnd(), remote.value.path, onConflict);
  } else {
    await transfers.startTree(remoteEnd(), path, isDir, LOCAL, localPath.value, onConflict);
  }
}

/**
 * Передать несколько записей. Если в папке назначения уже есть такие имена,
 * сначала спрашиваем: заменить, оставить оба или пропустить.
 */
const conflicts = useConflictsStore();
async function transferItems(from: "local" | "remote", items: Transferable[]) {
  const wanted = items.filter((i) => transferable(i.kind));
  const existing =
    from === "local"
      ? remote.value.entries.map((e) => e.name)
      : localEntries.value.map((e) => e.name);
  const plan = await planTransfers(wanted, existing, conflicts.ask);
  for (const { item, onConflict } of plan) {
    // Одна неудача не должна отменять остальные записи пачки.
    try {
      await startTransfer(from, item.path, item.kind === "dir", onConflict);
    } catch (e) {
      transferError.value = errorText(e);
    }
  }
}

/** Ссылки и особые файлы не передаём: передача их всё равно пропустит. */
const transferable = (kind: string) => kind === "file" || kind === "dir";
const canUpload = computed(
  () => localSel.value.picked.some((e) => transferable(e.kind)) && !!remote.value.path,
);
const canDownload = computed(
  () => remoteSel.value.picked.some((e) => transferable(e.kind)) && !!localPath.value,
);

const transferError = ref<string | null>(null);

async function upload() {
  transferError.value = null;
  await transferItems("local", localSel.value.picked);
}

async function download() {
  transferError.value = null;
  await transferItems("remote", remoteSel.value.picked);
}

/** Передача закончилась — в обеих папках могли появиться новые записи. */
const finishedCount = computed(() => transfers.items.filter((x) => x.status === "DONE").length);
watch(finishedCount, (now, before) => {
  if (now <= before) return;
  void store.refresh(props.sessionId);
  if (localPath.value) void openLocal(localPath.value);
});

/* ---- Операции над удалённой стороной ----
 *
 * Ввод имени — инлайн-полем, а не `window.prompt`: в WebView Tauri нативные
 * JS-диалоги ведут себя по-разному на разных платформах вплоть до полного
 * игнорирования, и операция молча ничего не сделает.
 */
const opError = ref<string | null>(null);

function joinRemote(dir: string, name: string): string {
  return dir === "/" ? `/${name}` : `${dir.replace(/\/+$/, "")}/${name}`;
}
const promptMode = ref<"mkdir" | "rename" | null>(null);
const promptValue = ref("");

function startMkdir() {
  promptMode.value = "mkdir";
  promptValue.value = "";
  opError.value = null;
}

const canRename = computed(() => remoteSel.value.picked.length === 1);

function startRename() {
  if (!remoteSelected.value || !canRename.value) return;
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

const toDelete = ref<SftpEntry[] | null>(null);

const deleteDescription = computed(() => {
  const list = toDelete.value;
  if (!list || list.length === 0) return t("dialog.deleteDescription");
  if (list.length === 1) return `${list[0].name} — ${t("dialog.deleteDescription")}`;
  return t("sftp.deleteMany", { count: list.length });
});

async function onConfirmDelete() {
  const targets = toDelete.value ?? [];
  toDelete.value = null;
  opError.value = null;
  for (const target of targets) {
    try {
      await store.remove(props.sessionId, target.path, target.kind === "dir");
    } catch (e) {
      // Сервер на непустую папку отвечает общей ошибкой, без причины. Удаление
      // намеренно не рекурсивное, и человеку надо сказать, что делать.
      opError.value = target.kind === "dir" ? t("sftp.deleteDirFailed") : errorText(e);
    }
  }
  remoteSel.value = { picked: [], anchor: null };
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
      v-if="transferError || dragError"
      class="mx-3 mt-2 bg-danger-soft border border-danger/40 text-danger-text text-xs rounded px-2 py-1.5"
    >
      {{ transferError || dragError }}
    </div>
    <div
      v-if="note"
      data-testid="sftp-note"
      class="mx-3 mt-2 bg-success-soft border border-success/30 text-success-text text-xs rounded px-2 py-1.5 truncate"
    >
      {{ note }}
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
            v-for="e in visibleLocal"
            v-else
            :key="e.path"
            type="button"
            draggable="true"
            data-testid="local-entry"
            class="w-full flex items-center gap-2 px-3 py-1 text-left text-xs hover:bg-surface-alt"
            :class="
              isPicked(localSel.picked, e)
                ? 'bg-surface-alt text-on-surface'
                : 'text-on-surface-muted'
            "
            :aria-selected="isPicked(localSel.picked, e)"
            @dragstart="onDragStart('local', e)"
            @dragend="onDragEnd"
            @click="pickLocal(e, $event)"
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
          :disabled="!canUpload"
          @click="upload"
        >
          <ArrowRight :size="16" />
        </button>
        <button
          type="button"
          class="p-2 rounded bg-surface-alt hover:bg-surface-card text-on-surface disabled:opacity-40"
          :title="t('sftp.download')"
          :disabled="!canDownload"
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
            :disabled="!canRename"
            @click="startRename"
          >
            <Pencil :size="14" />
          </button>
          <button
            type="button"
            class="p-1 rounded hover:bg-danger-soft text-on-surface-muted hover:text-danger-text disabled:opacity-40"
            :title="t('sftp.delete')"
            :disabled="remoteSel.picked.length === 0"
            @click="toDelete = [...remoteSel.picked]"
          >
            <Trash2 :size="14" />
          </button>
          <button
            type="button"
            data-testid="sftp-copy-path"
            class="p-1 rounded hover:bg-surface-alt text-on-surface-muted"
            :title="t('sftp.copyPath')"
            @click="copyRemotePath"
          >
            <Copy :size="14" />
          </button>
          <button
            type="button"
            data-testid="sftp-toggle-hidden"
            class="p-1 rounded hover:bg-surface-alt text-on-surface-muted"
            :title="t('sftp.hiddenFiles')"
            :aria-pressed="store.showHidden"
            @click="store.setShowHidden(!store.showHidden)"
          >
            <component :is="store.showHidden ? Eye : EyeOff" :size="14" />
          </button>
          <code class="ml-1 text-[11px] text-on-surface-muted truncate" :title="remote.path">
            {{ remote.path }}
          </code>
        </div>
        <div class="px-2 py-1 border-b border-edge">
          <input
            v-model="filter"
            type="search"
            data-testid="sftp-filter"
            class="w-full bg-surface border border-edge rounded px-2 py-0.5 text-xs focus:outline-none focus:border-accent"
            :placeholder="t('sftp.filter')"
          />
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
            v-else-if="visibleRemote.length === 0"
            class="px-3 py-2 text-xs text-on-surface-muted"
          >
            {{ remote.entries.length === 0 ? t("sftp.empty") : t("sftp.nothingMatches") }}
          </p>
          <button
            v-for="e in visibleRemote"
            v-else
            :key="e.path"
            type="button"
            draggable="true"
            data-testid="remote-entry"
            class="w-full flex items-center gap-2 px-3 py-1 text-left text-xs hover:bg-surface-alt"
            :class="
              isPicked(remoteSel.picked, e)
                ? 'bg-surface-alt text-on-surface'
                : 'text-on-surface-muted'
            "
            :title="[permString(e.permissions), formatModified(e.modified, locale)].filter(Boolean).join('  ')"
            :aria-selected="isPicked(remoteSel.picked, e)"
            @dragstart="onDragStart('remote', e)"
            @dragend="onDragEnd"
            @click="pickRemote(e, $event)"
            @dblclick="onRemoteDblClick(e)"
          >
            <component
              :is="iconFor(e.kind)"
              :size="14"
              class="shrink-0"
              :class="e.kind === 'dir' ? 'text-warning' : ''"
            />
            <span class="truncate">{{ e.name }}</span>
            <span class="ml-auto shrink-0 text-on-surface-muted tabular-nums">
              {{ e.kind === "file" ? formatBytes(e.size) : "" }}
            </span>
            <span
              class="hidden xl:inline shrink-0 w-24 text-right text-on-surface-muted tabular-nums"
              data-testid="remote-entry-modified"
            >
              {{ formatModified(e.modified, locale) }}
            </span>
            <span class="hidden xl:inline shrink-0 font-mono text-[10px] text-on-surface-muted">
              {{ permString(e.permissions) }}
            </span>
          </button>
        </div>
      </section>
    </div>

    <ConfirmDialog
      :open="toDelete !== null"
      :title="t('dialog.deleteTitle')"
      :description="deleteDescription"
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
