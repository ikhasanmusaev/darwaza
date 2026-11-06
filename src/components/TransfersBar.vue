<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { X, Upload, Download, Copy, Check, AlertTriangle } from "lucide-vue-next";
import { useTransfersStore } from "../stores/transfers";
import type { TransferProgress } from "../api/transfers";
import { errorText } from "../lib/errors";

/**
 * Панель передач снизу окна (#124).
 *
 * Именно снизу, а не модалкой: передача не должна перекрывать работу — пока
 * файл качается, терминалом продолжают пользоваться.
 */
const { t } = useI18n();
const store = useTransfersStore();

function percent(item: TransferProgress): number | null {
  if (!item.total) return null;
  return Math.min(100, Math.round((item.transferred / item.total) * 100));
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function sizeLabel(item: TransferProgress): string {
  return item.total
    ? `${formatBytes(item.transferred)} / ${formatBytes(item.total)}`
    : formatBytes(item.transferred);
}

/** Строка ошибки: код переводится, перед ней — файл, на котором остановились. */
function errorLine(item: TransferProgress): string {
  const raw = item.error ?? "";
  const text = raw.startsWith("errors.") ? errorText(raw) : raw;
  return item.currentPath ? `${item.currentPath}: ${text}` : text;
}

const hasFinished = computed(() => store.items.some((i) => i.status !== "RUNNING"));

async function onCancel(item: TransferProgress) {
  if (item.status === "RUNNING") {
    await store.cancel(item.id);
  } else {
    store.dismiss(item.id);
  }
}
</script>

<template>
  <div
    v-if="store.visible"
    data-testid="transfers-bar"
    class="shrink-0 border-t border-edge bg-surface-sidebar max-h-40 overflow-auto"
  >
    <div class="flex items-center gap-2 px-3 py-1 border-b border-edge">
      <span class="text-[11px] uppercase tracking-wide text-on-surface-faint">
        {{ t("transfers.title") }}
      </span>
      <button
        v-if="hasFinished"
        type="button"
        class="ml-auto text-[11px] text-on-surface-faint hover:text-on-surface"
        @click="store.dismissFinished()"
      >
        {{ t("transfers.clearFinished") }}
      </button>
    </div>

    <ul>
      <li
        v-for="item in store.items"
        :key="item.id"
        class="flex items-center gap-2 px-3 py-1.5"
        data-testid="transfer-row"
      >
        <component
          :is="item.kind === 'upload' ? Upload : item.kind === 'copy' ? Copy : Download"
          :size="13"
          class="shrink-0 text-on-surface-faint"
        />

        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <span class="text-xs text-on-surface truncate">{{ item.name }}</span>
            <span class="text-[11px] text-on-surface-faint shrink-0">
              {{ sizeLabel(item) }}
            </span>
            <Check v-if="item.status === 'DONE'" :size="12" class="text-success shrink-0" />
            <AlertTriangle
              v-else-if="item.status === 'FAILED'"
              :size="12"
              class="text-danger shrink-0"
              :title="item.error"
            />
            <span
              v-else-if="item.status === 'CANCELLED'"
              class="text-[10px] text-on-surface-faint shrink-0"
            >
              {{ t("transfers.cancelled") }}
            </span>
          </div>

          <!-- Полоса прогресса. Без известного размера показываем бегунок. -->
          <div class="mt-1 h-1 rounded bg-surface-alt overflow-hidden">
            <div
              class="h-full transition-all"
              :class="[
                item.status === 'FAILED' ? 'bg-danger' : 'bg-success',
                percent(item) === null && item.status === 'RUNNING' ? 'animate-pulse w-1/3' : '',
              ]"
              :style="percent(item) !== null ? { width: `${percent(item)}%` } : undefined"
            />
          </div>
          <p
            v-if="item.error"
            class="text-[11px] text-danger-text truncate mt-0.5"
            data-testid="transfer-error"
            :title="errorLine(item)"
          >
            {{ errorLine(item) }}
          </p>
          <p
            v-else-if="item.status === 'RUNNING' && item.currentPath"
            class="text-[11px] text-on-surface-faint truncate mt-0.5"
            data-testid="transfer-current"
          >
            {{ item.currentPath }}
          </p>
        </div>

        <button
          type="button"
          data-testid="transfer-cancel"
          class="p-1 rounded hover:bg-surface-alt text-on-surface-faint hover:text-on-surface shrink-0"
          :title="item.status === 'RUNNING' ? t('transfers.cancel') : t('transfers.dismiss')"
          @click="onCancel(item)"
        >
          <X :size="13" />
        </button>
      </li>
    </ul>
  </div>
</template>
