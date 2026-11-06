<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { Film, Trash2, RefreshCw } from "lucide-vue-next";
import { useRecordingsStore } from "../stores/recordings";
import type { RecordingFile } from "../api/recordings";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import PageHeader from "../components/PageHeader.vue";
import { errorText } from "../lib/errors";

const { t } = useI18n();
const store = useRecordingsStore();

onMounted(() => store.refresh());

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function formatDate(unix: number | null): string {
  if (!unix) return "—";
  return new Date(unix * 1000).toLocaleString();
}

function formatDuration(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  const remaining = Math.floor(seconds % 60);
  return `${String(minutes).padStart(2, "0")}:${String(remaining).padStart(2, "0")}`;
}

const toDelete = ref<RecordingFile | null>(null);
const error = ref<string | null>(null);

async function onConfirmDelete() {
  const target = toDelete.value;
  toDelete.value = null;
  if (!target) return;
  try {
    await store.remove(target.path);
  } catch (e) {
    error.value = errorText(e);
  }
}
</script>

<template>
  <div class="p-6">
    <PageHeader :title="t('recordings.title')" :description="t('recordings.description')">
      <button
        type="button"
        class="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-r8 bg-surface-alt hover:bg-surface-card text-on-surface"
        :title="t('sftp.refresh')"
        @click="store.refresh()"
      >
        <RefreshCw :size="16" />
        {{ t("sftp.refresh") }}
      </button>
    </PageHeader>

    <p v-if="store.dir" class="mb-4 font-mono text-mono text-on-surface-muted">
      {{ store.dir }}
    </p>

    <div
      v-if="error"
      class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded-r8 px-3 py-2 mb-3"
    >
      {{ error }}
    </div>

    <div v-if="store.loading" class="text-sm text-on-surface-muted py-4">
      {{ t("common.loading") }}
    </div>

    <div
      v-else-if="store.files.length === 0"
      class="flex flex-col items-center gap-2 py-16 text-on-surface-muted"
    >
      <Film :size="32" />
      <p class="text-sm">{{ t("recordings.empty") }}</p>
      <p class="text-xs">{{ t("recordings.emptyHint") }}</p>
    </div>

    <div v-else class="overflow-x-auto">
      <table class="w-full min-w-[820px] border-collapse" data-testid="recordings-table">
        <thead>
          <tr class="border-b border-edge-subtle text-left">
            <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">
              {{ t("recordings.session") }}
            </th>
            <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">
              {{ t("recordings.date") }}
            </th>
            <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">
              {{ t("recordings.duration") }}
            </th>
            <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">
              {{ t("recordings.size") }}
            </th>
            <th class="px-3 py-2 text-overline uppercase text-on-surface-muted text-right">
              {{ t("palette.section.actions") }}
            </th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="file in store.files"
            :key="file.path"
            class="h-16 border-b border-edge-subtle hover:bg-surface-alt"
          >
            <td class="px-3 py-3">
              <div class="flex items-center gap-3">
                <span
                  class="w-9 h-9 rounded-r8 bg-surface-alt flex items-center justify-center text-on-surface-muted"
                >
                  <Film :size="16" />
                </span>
                <div>
                  <div class="text-body font-semibold text-on-surface break-words">
                    {{ file.title || file.name }}
                  </div>
                  <div v-if="file.active" class="flex items-center gap-1.5 text-xs text-danger-text">
                    <span class="w-2 h-2 rounded-full bg-danger" />
                    {{ t("recordings.active") }}
                  </div>
                </div>
              </div>
            </td>
            <td class="px-3 py-3 font-mono text-mono text-on-surface-muted">
              {{ formatDate(file.modified) }}
            </td>
            <td class="px-3 py-3 font-mono text-mono text-on-surface-muted">
              {{ typeof file.durationSeconds === "number" ? formatDuration(file.durationSeconds) : "—" }}
            </td>
            <td class="px-3 py-3 font-mono text-mono text-on-surface-muted">
              {{ formatBytes(file.size) }}
            </td>
            <td class="px-3 py-3">
              <div class="flex justify-end">
                <button
                  type="button"
                  class="p-2 rounded-r8 hover:bg-danger-soft text-on-surface-muted hover:text-danger-text disabled:opacity-45"
                  :title="t('hosts.delete')"
                  :disabled="file.active"
                  @click="toDelete = file"
                >
                  <Trash2 :size="15" />
                </button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <p class="mt-5 text-xs text-on-surface-muted">{{ t("recordings.playHint") }}</p>

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
