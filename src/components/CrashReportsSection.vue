<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { Bug, Trash2, Eye, X, Send, Save } from "lucide-vue-next";
import { crashApi, type CrashReport } from "../api/crash";
import { errorText } from "../lib/errors";

/** Секция отчётов о сбоях в настройках (#171). */
const { t } = useI18n();

const enabled = ref(false);
const reports = ref<CrashReport[]>([]);
const dir = ref("");
const error = ref<string | null>(null);
const viewing = ref<{ name: string; text: string } | null>(null);

/* ---- Отправка на свой адрес (#171a) ---- */
const endpoint = ref("");
const endpointSaved = ref(false);
/**
 * Трогал ли человек поле адреса.
 *
 * Начальная загрузка приходит асинхронно и раньше затирала уже набранное:
 * пользователь начинал печатать до её завершения, а потом поле пустело само
 * собой. Ошибка редкая и оттого неприятная — выглядит как «приложение съело
 * ввод».
 */
const endpointTouched = ref(false);
const busyPath = ref<string | null>(null);
const note = ref<string | null>(null);

async function refresh() {
  error.value = null;
  try {
    enabled.value = await crashApi.enabled();
    reports.value = await crashApi.list();
    dir.value = await crashApi.dir();
    // Не перетираем то, что человек уже начал набирать.
    if (!endpointTouched.value) {
      endpoint.value = (await crashApi.endpointGet()) ?? "";
    }
  } catch (e) {
    error.value = errorText(e);
  }
}

onMounted(refresh);

async function toggle() {
  try {
    await crashApi.setEnabled(!enabled.value);
    await refresh();
  } catch (e) {
    error.value = errorText(e);
  }
}

async function saveEndpoint() {
  error.value = null;
  endpointSaved.value = false;
  try {
    await crashApi.endpointSet(endpoint.value);
    endpointSaved.value = true;
  } catch (e) {
    error.value = errorText(e);
  }
}

/** Отправка идёт только по нажатию — сами мы не отправляем ничего и никогда. */
async function send(r: CrashReport) {
  error.value = null;
  note.value = null;
  busyPath.value = r.path;
  try {
    const status = await crashApi.send(r.path);
    note.value = t("crash.sent", { status });
  } catch (e) {
    error.value = errorText(e);
  } finally {
    busyPath.value = null;
  }
}

async function exportCopy(r: CrashReport) {
  error.value = null;
  note.value = null;
  try {
    note.value = t("crash.exported", { path: await crashApi.export(r.path) });
  } catch (e) {
    error.value = errorText(e);
  }
}

async function view(r: CrashReport) {
  try {
    viewing.value = { name: r.name, text: await crashApi.read(r.path) };
  } catch (e) {
    error.value = errorText(e);
  }
}

async function remove(r: CrashReport) {
  try {
    await crashApi.remove(r.path);
    reports.value = reports.value.filter((x) => x.path !== r.path);
  } catch (e) {
    error.value = errorText(e);
  }
}

function formatDate(unix: number | null): string {
  return unix ? new Date(unix * 1000).toLocaleString() : "—";
}
</script>

<template>
  <section class="bg-surface-card border border-edge rounded-lg p-4 mb-4">
    <div class="flex items-center gap-2 mb-1">
      <Bug :size="18" class="text-on-surface-muted" />
      <h3 class="text-sm font-medium text-on-surface">{{ t("crash.title") }}</h3>
    </div>
    <p class="text-xs text-on-surface-faint mb-3">{{ t("crash.description") }}</p>

    <div
      v-if="error"
      class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2 mb-3"
    >
      {{ error }}
    </div>

    <label class="flex items-center gap-2 text-sm text-on-surface cursor-pointer mb-3">
      <input
        type="checkbox"
        :checked="enabled"
        data-testid="crash-toggle"
        class="accent-accent"
        @change="toggle"
      />
      {{ t("crash.enable") }}
    </label>

    <div class="mb-3" data-testid="crash-section-ready">
      <label class="block text-xs text-on-surface-muted mb-1">
        {{ t("crash.endpoint") }}
      </label>
      <div class="flex gap-2">
        <input
          v-model="endpoint"
          type="url"
          data-testid="crash-endpoint"
          @input="endpointTouched = true"
          class="flex-1 bg-surface border border-edge rounded px-3 py-1.5 text-sm focus:outline-none focus:border-accent"
          :placeholder="t('crash.endpointPlaceholder')"
          @blur="saveEndpoint"
        />
      </div>
      <p class="text-[11px] text-on-surface-faint mt-1">
        {{ endpointSaved ? t("crash.endpointSaved") : t("crash.endpointHint") }}
      </p>
    </div>

    <p v-if="note" class="text-xs text-success-text mb-2" data-testid="crash-note">
      {{ note }}
    </p>

    <p v-if="dir" class="text-[11px] text-on-surface-faint mb-2">
      <code class="font-mono">{{ dir }}</code>
    </p>

    <p v-if="reports.length === 0" class="text-xs text-on-surface-faint">
      {{ t("crash.empty") }}
    </p>

    <ul v-else class="space-y-1" data-testid="crash-list">
      <li
        v-for="r in reports"
        :key="r.path"
        class="group flex items-center gap-2 px-2 py-1.5 rounded border border-edge"
      >
        <div class="min-w-0 flex-1">
          <p class="text-xs text-on-surface truncate">{{ r.name }}</p>
          <p class="text-[11px] text-on-surface-faint">{{ formatDate(r.modified) }}</p>
        </div>
        <button
          type="button"
          class="p-1 rounded hover:bg-surface-alt text-on-surface-muted"
          :title="t('crash.view')"
          @click="view(r)"
        >
          <Eye :size="14" />
        </button>
        <button
          type="button"
          data-testid="crash-export"
          class="p-1 rounded hover:bg-surface-alt text-on-surface-muted"
          :title="t('crash.export')"
          @click="exportCopy(r)"
        >
          <Save :size="14" />
        </button>
        <button
          v-if="endpoint.trim()"
          type="button"
          data-testid="crash-send"
          class="p-1 rounded hover:bg-surface-alt text-on-surface-muted hover:text-accent-text disabled:opacity-40"
          :title="t('crash.send')"
          :disabled="busyPath === r.path"
          @click="send(r)"
        >
          <Send :size="14" />
        </button>
        <button
          type="button"
          class="p-1 rounded hover:bg-danger-soft text-on-surface-muted hover:text-danger-text"
          :title="t('hosts.delete')"
          @click="remove(r)"
        >
          <Trash2 :size="14" />
        </button>
      </li>
    </ul>

    <!-- Просмотр отчёта: пользователь должен видеть, что именно приложит к багрепорту -->
    <div
      v-if="viewing"
      data-testid="crash-viewer"
      class="mt-3 border border-edge rounded bg-surface"
    >
      <div class="flex items-center gap-2 px-2 py-1.5 border-b border-edge">
        <span class="text-xs text-on-surface truncate">{{ viewing.name }}</span>
        <button
          type="button"
          class="ml-auto p-1 rounded hover:bg-surface-alt text-on-surface-muted"
          @click="viewing = null"
        >
          <X :size="13" />
        </button>
      </div>
      <pre
        class="p-2 text-[11px] font-mono text-on-surface-muted overflow-auto max-h-64 whitespace-pre-wrap"
        >{{ viewing.text }}</pre
      >
    </div>
  </section>
</template>
