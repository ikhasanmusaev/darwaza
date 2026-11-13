<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import {
  ArrowUp,
  RefreshCw,
  Folder,
  File as FileIcon,
  Link2,
  FolderPlus,
  Pencil,
  Trash2,
  Loader2,
  Eye,
  EyeOff,
  X,
} from "lucide-vue-next";
import { useFilesStore, type FileEntry, type Side } from "../stores/files";
import { useHostsStore } from "../stores/hosts";
import ConfirmDialog from "./ConfirmDialog.vue";
import { useSftpStore } from "../stores/sftp";
import { formatBytes, formatModified, isHidden, permString } from "../lib/files";
import { isPicked } from "../lib/selection";
import { isMacOS } from "../lib/platform";

const props = defineProps<{ side: Side }>();
const { t, locale } = useI18n();
/** Настройка скрытых файлов общая с боковой панелью файлов. */
const sftp = useSftpStore();
const files = useFilesStore();
const hosts = useHostsStore();

const pane = computed(() => files.pane(props.side));

/** Значение выпадающего списка: «local», «host:<id>» или пусто. */
const sourceValue = computed(() => {
  const s = pane.value.source;
  if (!s) return "";
  return s.kind === "local" ? "local" : `host:${s.hostId}`;
});

function onSource(e: Event) {
  const v = (e.target as HTMLSelectElement).value;
  if (v === "") void files.setSource(props.side, null);
  else if (v === "local") void files.setSource(props.side, { kind: "local" });
  else void files.setSource(props.side, { kind: "host", hostId: Number(v.slice(5)) });
}

const visibleEntries = computed(() =>
  sftp.showHidden ? pane.value.entries : pane.value.entries.filter((e) => !isHidden(e.name)),
);

function iconFor(kind: FileEntry["kind"]) {
  if (kind === "dir") return Folder;
  if (kind === "symlink") return Link2;
  return FileIcon;
}

/* ---- Новая папка / переименование ---- */
const promptMode = ref<"mkdir" | "rename" | null>(null);
const promptValue = ref("");

function startPrompt(mode: "mkdir" | "rename") {
  promptMode.value = mode;
  promptValue.value = mode === "rename" ? (pane.value.selected?.name ?? "") : "";
}

async function submitPrompt() {
  const name = promptValue.value.trim();
  const mode = promptMode.value;
  if (!name || !mode) return;
  promptMode.value = null;
  if (mode === "mkdir") await files.mkdir(props.side, name);
  else if (pane.value.selected) await files.rename(props.side, pane.value.selected, name);
}

/* ---- Выбор ---- */
function onPick(e: FileEntry, ev: MouseEvent) {
  const toggle = isMacOS() ? ev.metaKey : ev.ctrlKey;
  files.pick(props.side, e, { toggle, range: ev.shiftKey }, visibleEntries.value);
}
const canRename = computed(() => pane.value.picked.length === 1);

/* ---- Удаление (с подтверждением) ---- */
const toDelete = ref<FileEntry[] | null>(null);

const deleteDescription = computed(() => {
  const list = toDelete.value;
  if (!list || list.length === 0) return t("dialog.deleteDescription");
  if (list.length === 1) return `${list[0].name} — ${t("dialog.deleteDescription")}`;
  return t("sftp.deleteMany", { count: list.length });
});

async function confirmDelete() {
  const targets = toDelete.value ?? [];
  toDelete.value = null;
  let dirFailed = false;
  let error: string | null = null;
  for (const target of targets) {
    const ok = await files.remove(props.side, target);
    if (ok) continue;
    if (target.kind === "dir") dirFailed = true;
    else error = pane.value.opError;
  }
  // Сервер на непустую папку отвечает общей ошибкой, без причины, а удаление
  // намеренно не рекурсивное — говорим, что делать.
  if (dirFailed) pane.value.opError = t("sftp.deleteDirFailed");
  else if (error) pane.value.opError = error;
}

/* ---- Перетаскивание между панелями ---- */
const dropHover = ref(false);

/** Тащат выбранную запись — едут все выбранные; невыбранную — только она. */
function onDragStart(entry: FileEntry) {
  const picked = pane.value.picked;
  files.dragging = { side: props.side, entries: isPicked(picked, entry) ? [...picked] : [entry] };
}
function onDragEnd() {
  files.dragging = null;
  dropHover.value = false;
}
function canDrop(): boolean {
  const d = files.dragging;
  return !!d && d.side !== props.side && files.canTransfer(d.side, d.entries);
}
function onDragOver(e: DragEvent) {
  if (!canDrop()) return;
  e.preventDefault();
  dropHover.value = true;
}
async function onDrop() {
  const d = files.dragging;
  dropHover.value = false;
  files.dragging = null;
  if (d && d.side !== props.side) await files.transfer(d.side, d.entries);
}
</script>

<template>
  <section
    class="flex flex-col min-h-0 min-w-0 border border-edge rounded-r8 bg-surface-card"
    :data-testid="`files-pane-${side}`"
  >
    <div class="flex items-center gap-2 px-2 py-1.5 border-b border-edge">
      <select
        class="bg-surface border border-edge rounded-r6 px-2 py-1 text-xs text-on-surface focus:outline-none focus:border-accent max-w-[55%]"
        :data-testid="`files-source-${side}`"
        :value="sourceValue"
        :aria-label="t('files.pickSource')"
        @change="onSource"
      >
        <option value="" disabled>{{ t("files.chooseServer") }}</option>
        <option value="local">{{ t("sftp.local") }}</option>
        <option v-for="h in hosts.hosts" :key="h.id" :value="`host:${h.id}`">{{ h.name }}</option>
      </select>
      <template v-if="pane.source && !pane.connecting">
        <button
          type="button"
          class="p-1 rounded hover:bg-surface-alt text-on-surface-muted"
          :title="t('sftp.up')"
          :data-testid="`files-up-${side}`"
          @click="files.up(side)"
        >
          <ArrowUp :size="14" />
        </button>
        <button
          type="button"
          class="p-1 rounded hover:bg-surface-alt text-on-surface-muted"
          :title="t('sftp.refresh')"
          @click="files.refresh(side)"
        >
          <RefreshCw :size="14" />
        </button>
        <button
          type="button"
          class="p-1 rounded hover:bg-surface-alt text-on-surface-muted"
          :title="t('sftp.newFolder')"
          :data-testid="`files-mkdir-${side}`"
          @click="startPrompt('mkdir')"
        >
          <FolderPlus :size="14" />
        </button>
        <button
          type="button"
          class="p-1 rounded hover:bg-surface-alt text-on-surface-muted disabled:opacity-40"
          :title="t('sftp.rename')"
          :disabled="!canRename"
          :data-testid="`files-rename-${side}`"
          @click="startPrompt('rename')"
        >
          <Pencil :size="14" />
        </button>
        <button
          type="button"
          class="p-1 rounded hover:bg-danger-soft text-on-surface-muted hover:text-danger-text disabled:opacity-40"
          :title="t('sftp.delete')"
          :disabled="pane.picked.length === 0"
          :data-testid="`files-delete-${side}`"
          @click="toDelete = [...pane.picked]"
        >
          <Trash2 :size="14" />
        </button>
        <button
          type="button"
          class="p-1 rounded hover:bg-surface-alt text-on-surface-muted"
          :title="t('sftp.hiddenFiles')"
          :aria-pressed="sftp.showHidden"
          :data-testid="`files-hidden-${side}`"
          @click="sftp.setShowHidden(!sftp.showHidden)"
        >
          <component :is="sftp.showHidden ? Eye : EyeOff" :size="14" />
        </button>
      </template>
    </div>

    <div v-if="pane.source && pane.path" class="px-3 py-1.5 border-b border-edge-subtle">
      <code class="text-[14px] text-on-surface-muted truncate block" :title="pane.path" :data-testid="`files-path-${side}`">
        {{ pane.path }}
      </code>
    </div>

    <div
      v-if="pane.opError"
      class="mx-2 mt-2 flex items-start gap-2 bg-danger-soft border border-danger/40 text-danger-text text-xs rounded px-2 py-1.5"
      :data-testid="`files-op-error-${side}`"
    >
      <span class="flex-1">{{ pane.opError }}</span>
      <button type="button" class="shrink-0" :title="t('form.cancel')" @click="pane.opError = null">
        <X :size="12" />
      </button>
    </div>

    <form
      v-if="promptMode"
      class="flex items-center gap-2 px-2 py-1.5 border-b border-edge"
      @submit.prevent="submitPrompt"
    >
      <input
        v-model="promptValue"
        :data-testid="`files-prompt-${side}`"
        class="flex-1 bg-surface border border-edge rounded-r6 px-2 py-1 text-xs focus:outline-none focus:border-accent"
        :placeholder="promptMode === 'mkdir' ? t('sftp.newFolderPrompt') : t('sftp.renamePrompt')"
        @keydown.esc.prevent="promptMode = null"
      />
      <button type="submit" class="px-2 py-1 text-xs rounded-r6 bg-accent text-on-accent" :disabled="!promptValue.trim()">
        OK
      </button>
    </form>

    <div
      class="flex-1 overflow-auto min-h-0 transition-colors"
      :class="dropHover ? 'bg-accent/10 ring-1 ring-inset ring-accent' : ''"
      :data-testid="`files-list-${side}`"
      @dragover="onDragOver"
      @dragleave="dropHover = false"
      @drop.prevent="onDrop"
    >
      <p v-if="!pane.source" class="px-3 py-6 text-xs text-on-surface-muted text-center">
        {{ t("files.emptyPane") }}
      </p>
      <p v-else-if="pane.connecting" class="px-3 py-6 text-xs text-on-surface-muted text-center flex items-center justify-center gap-2" :data-testid="`files-connecting-${side}`">
        <Loader2 :size="14" class="animate-spin" /> {{ t("files.connecting") }}
      </p>
      <p v-else-if="pane.loading" class="px-3 py-2 text-xs text-on-surface-muted">{{ t("common.loading") }}</p>
      <p v-else-if="pane.error" class="px-3 py-2 text-xs text-danger-text" :data-testid="`files-error-${side}`">{{ pane.error }}</p>
      <template v-else>
        <p v-if="visibleEntries.length === 0" class="px-3 py-2 text-xs text-on-surface-muted">{{ t("sftp.empty") }}</p>
        <button
          v-for="e in visibleEntries"
          :key="e.path"
          type="button"
          draggable="true"
          :data-testid="`files-entry-${side}`"
          class="w-full flex items-center gap-2 px-3 py-1.5 text-left text-[15px] leading-5 hover:bg-surface-alt"
          :class="isPicked(pane.picked, e) ? 'bg-surface-alt text-on-surface' : 'text-on-surface-muted'"
          :aria-selected="isPicked(pane.picked, e)"
          :title="[permString(e.permissions), formatModified(e.modified, locale)].filter(Boolean).join('  ')"
          @click="onPick(e, $event)"
          @dblclick="files.enter(side, e)"
          @dragstart="onDragStart(e)"
          @dragend="onDragEnd"
        >
          <component :is="iconFor(e.kind)" :size="16" class="shrink-0" :class="e.kind === 'dir' ? 'text-warning' : ''" />
          <span class="truncate">{{ e.name }}</span>
          <span class="ml-auto shrink-0 text-on-surface-muted tabular-nums">{{ e.kind === "file" ? formatBytes(e.size) : "" }}</span>
          <span class="hidden lg:inline shrink-0 w-32 text-right text-[13px] text-on-surface-muted tabular-nums">{{ formatModified(e.modified, locale) }}</span>
        </button>
      </template>
    </div>

    <ConfirmDialog
      :open="toDelete !== null"
      :title="t('dialog.deleteTitle')"
      :description="deleteDescription"
      :confirm-label="t('form.delete')"
      :cancel-label="t('form.cancel')"
      variant="danger"
      @update:open="(v: boolean) => { if (!v) toDelete = null; }"
      @confirm="confirmDelete"
      @cancel="toDelete = null"
    />
  </section>
</template>
