<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { FileText, FolderOpen, RefreshCw } from "lucide-vue-next";
import { logsApi, type AgentLogEntry } from "../api/logs";
import { errorText } from "../lib/errors";

const { t, te } = useI18n();

const dir = ref("");
const entries = ref<AgentLogEntry[]>([]);
const error = ref<string | null>(null);

async function refresh() {
  error.value = null;
  try {
    dir.value = await logsApi.dir();
    entries.value = await logsApi.agentLog(200);
  } catch (e) {
    error.value = errorText(e);
  }
}

async function openDir() {
  error.value = null;
  try {
    await logsApi.openDir();
  } catch (e) {
    error.value = errorText(e);
  }
}

function outcomeText(outcome: string): string {
  const key = `logs.${outcome}`;
  return te(key) ? t(key) : outcome;
}

const rows = computed(() =>
  entries.value.map((e) => ({ ...e, time: new Date(e.at).toLocaleString() })),
);

onMounted(refresh);
</script>

<template>
  <section
    class="bg-surface-card border border-edge rounded-lg p-4 mb-4"
    data-testid="logs-section"
  >
    <div class="flex items-center gap-2 mb-1">
      <FileText :size="18" class="text-on-surface-muted" />
      <h3 class="text-sm font-medium text-on-surface">{{ t("logs.title") }}</h3>
    </div>
    <p class="text-xs text-on-surface-faint mb-3">{{ t("logs.description") }}</p>

    <div
      v-if="error"
      class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2 mb-3"
    >
      {{ error }}
    </div>

    <div class="flex items-center gap-2 mb-1">
      <code
        class="flex-1 min-w-0 font-mono text-[11px] text-on-surface-faint break-all"
        data-testid="logs-dir"
      >{{ dir }}</code>
      <button
        type="button"
        class="shrink-0 inline-flex items-center gap-1.5 px-3 py-1.5 rounded border border-edge text-xs text-on-surface hover:bg-surface-alt"
        data-testid="logs-open-dir"
        @click="openDir"
      >
        <FolderOpen :size="14" />
        {{ t("logs.openDir") }}
      </button>
    </div>
    <p class="text-[11px] text-on-surface-faint mb-4">{{ t("logs.connectionsHint") }}</p>

    <div class="flex items-center justify-between mb-2">
      <h4 class="text-xs font-medium text-on-surface-muted">{{ t("logs.agentTitle") }}</h4>
      <button
        type="button"
        class="inline-flex items-center gap-1 text-xs text-on-surface-muted hover:text-on-surface"
        data-testid="logs-refresh"
        @click="refresh"
      >
        <RefreshCw :size="12" />
        {{ t("logs.refresh") }}
      </button>
    </div>

    <p v-if="!rows.length" class="text-xs text-on-surface-faint" data-testid="logs-empty">
      {{ t("logs.agentEmpty") }}
    </p>
    <ul v-else class="max-h-80 overflow-y-auto divide-y divide-edge" data-testid="logs-agent-list">
      <li v-for="row in rows" :key="row.id" class="py-2 text-xs" data-testid="logs-agent-row">
        <div class="flex flex-wrap items-center gap-x-2 text-on-surface-faint">
          <span>{{ row.time }}</span>
          <span v-if="row.host" class="text-on-surface">{{ row.host }}</span>
          <span>{{ row.action }}</span>
          <span :class="row.outcome === 'ok' ? 'text-on-surface-muted' : 'text-danger-text'">
            {{ outcomeText(row.outcome) }}
          </span>
        </div>
        <code v-if="row.detail" class="block mt-0.5 font-mono text-[11px] text-on-surface-muted break-all">{{ row.detail }}</code>
      </li>
    </ul>
  </section>
</template>
