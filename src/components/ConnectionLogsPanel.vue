<script setup lang="ts">
import { ref, watch, nextTick } from "vue";
import { useI18n } from "vue-i18n";
import { X, Clock, AlertCircle, CheckCircle2, Loader2 } from "lucide-vue-next";
import { connectionLogsApi, type ConnectionLogEntry } from "../api/connection-logs";
import { errorText } from "../lib/errors";

const props = defineProps<{
  open: boolean;
  hostId: number;
  hostName: string;
}>();

const emit = defineEmits<{
  "update:open": [value: boolean];
}>();

const { t } = useI18n();
const logs = ref<ConnectionLogEntry[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);
const panelRef = ref<HTMLElement | null>(null);

async function loadLogs() {
  if (!props.hostId) return;
  loading.value = true;
  error.value = null;
  try {
    logs.value = await connectionLogsApi.list(props.hostId, 100);
  } catch (e) {
    error.value = errorText(e);
  } finally {
    loading.value = false;
  }
}

watch(
  () => props.open,
  (isOpen) => {
    if (isOpen) {
      loadLogs();
      nextTick(() => panelRef.value?.focus());
    }
  },
);

function close() {
  emit("update:open", false);
}

function onBackdropClick(e: MouseEvent) {
  if (e.target === e.currentTarget) {
    close();
  }
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    e.preventDefault();
    close();
  }
}

function formatDate(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleString();
}

function durationMs(started: string, ended: string | null): string | null {
  if (!ended) return null;
  const ms = new Date(ended).getTime() - new Date(started).getTime();
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60000) return `${Math.floor(ms / 1000)}s`;
  return `${Math.floor(ms / 60000)}m ${Math.floor((ms % 60000) / 1000)}s`;
}

function statusIcon(status: string) {
  switch (status) {
    case "connected":
    case "success":
      return CheckCircle2;
    case "failed":
      return AlertCircle;
    case "connecting":
      return Loader2;
    default:
      return Clock;
  }
}

function statusClass(status: string): string {
  switch (status) {
    case "connected":
    case "success":
      return "text-success-text";
    case "failed":
      return "text-danger-text";
    case "connecting":
      return "text-warning-text";
    default:
      return "text-on-surface-muted";
  }
}
</script>

<template>
  <Teleport to="body">
    <Transition
      enter-active-class="transition-opacity duration-200"
      leave-active-class="transition-opacity duration-200"
      enter-from-class="opacity-0"
      leave-to-class="opacity-0"
    >
      <div v-if="open" class="fixed inset-0 z-40 bg-scrim" @click="onBackdropClick" />
    </Transition>

    <Transition
      enter-active-class="transition-transform duration-300 ease-out"
      leave-active-class="transition-transform duration-200 ease-in"
      enter-from-class="panel-enter-from"
      leave-to-class="panel-leave-to"
    >
      <div
        v-if="open"
        ref="panelRef"
        tabindex="-1"
        class="fixed right-0 top-0 bottom-0 z-50 w-full max-w-xl bg-surface border-l border-edge shadow-2xl outline-none overflow-y-auto"
        @keydown="onKeydown"
      >
        <!-- Header -->
        <div
          class="sticky top-0 bg-surface border-b border-edge px-6 py-4 flex items-center justify-between"
        >
          <div>
            <h2 class="text-lg font-semibold">{{ t("connectionLogs.title") }}</h2>
            <p class="text-xs text-on-surface-faint">{{ hostName }}</p>
          </div>
          <button
            type="button"
            class="inline-flex items-center justify-center w-8 h-8 rounded hover:bg-surface-alt text-on-surface-muted"
            :title="t('form.cancel')"
            @click="close"
          >
            <X :size="18" />
          </button>
        </div>

        <!-- Content -->
        <div class="px-6 py-4">
          <div v-if="loading" class="flex items-center gap-2 text-on-surface-muted py-8">
            <Loader2 :size="18" class="animate-spin" />
            <span class="text-sm">{{ t("common.loading") }}</span>
          </div>

          <div
            v-else-if="error"
            class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2 mb-4"
          >
            {{ error }}
          </div>

          <div
            v-else-if="logs.length === 0"
            class="flex flex-col items-center justify-center py-16 text-on-surface-faint"
          >
            <Clock :size="40" class="mb-3 opacity-40" />
            <p class="text-sm">{{ t("connectionLogs.empty") }}</p>
          </div>

          <div v-else class="space-y-2">
            <div
              v-for="log in logs"
              :key="log.id"
              class="bg-surface-card border border-edge rounded-lg p-3 flex items-start gap-3"
            >
              <component
                :is="statusIcon(log.status)"
                :size="18"
                :class="['shrink-0 mt-0.5', statusClass(log.status)]"
              />
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2 text-sm">
                  <span class="font-medium text-on-surface">{{ log.status }}</span>
                  <span v-if="log.error" class="text-xs text-danger-text truncate">{{
                    log.error
                  }}</span>
                </div>
                <div class="flex items-center gap-3 text-xs text-on-surface-faint mt-1">
                  <span>{{ formatDate(log.startedAt) }}</span>
                  <span v-if="log.endedAt">
                    → {{ formatDate(log.endedAt) }}
                    <span class="text-on-surface-muted"
                      >({{ durationMs(log.startedAt, log.endedAt) }})</span
                    >
                  </span>
                  <span v-else class="text-warning-text">{{ t("connectionLogs.ongoing") }}</span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
