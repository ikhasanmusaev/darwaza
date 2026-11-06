<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowLeft, Server, Trash2, Download, Loader2, Eraser, Fingerprint } from "lucide-vue-next";
import { useRouter } from "vue-router";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import { useKnownHostsStore } from "../stores/known_hosts";
import KnownHostsImportModal from "../components/KnownHostsImportModal.vue";
import type { KnownHost } from "../api/known_hosts";

const { t } = useI18n();
const router = useRouter();
const store = useKnownHostsStore();

const toDelete = ref<KnownHost | null>(null);
const importOpen = ref(false);
const clearOpen = ref(false);

onMounted(() => store.fetchAll());

function deleteHost(h: KnownHost) {
  toDelete.value = h;
}

async function onConfirmDelete() {
  if (!toDelete.value) return;
  try {
    await store.remove(toDelete.value.id);
  } finally {
    toDelete.value = null;
  }
}

async function onConfirmClear() {
  try {
    await store.clear();
  } finally {
    clearOpen.value = false;
  }
}

function back() {
  router.push("/settings");
}

function formatDate(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}
</script>

<template>
  <div class="p-6 max-w-3xl">
    <!-- Header -->
    <div class="flex items-center justify-between mb-5">
      <div class="flex items-center gap-3">
        <button
          class="inline-flex items-center justify-center w-8 h-8 rounded hover:bg-surface-alt text-on-surface-muted"
          :title="t('form.cancel')"
          @click="back"
        >
          <ArrowLeft :size="18" />
        </button>
        <h2 class="text-lg font-semibold">{{ t("knownHosts.title") }}</h2>
      </div>
      <button
        type="button"
        data-testid="known-hosts-import"
        class="ml-auto inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded bg-surface-alt hover:bg-surface-card text-on-surface"
        @click="importOpen = true"
      >
        <Download :size="14" />
        {{ t("knownHosts.import") }}
      </button>
      <button
        v-if="store.knownHosts.length > 0"
        type="button"
        class="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded border border-danger/40 bg-transparent text-danger-text hover:bg-danger-soft"
        @click="clearOpen = true"
      >
        <Eraser :size="14" />
        {{ t("knownHosts.clearAll") }}
      </button>
    </div>

    <p class="text-sm text-on-surface-faint mb-4">{{ t("knownHosts.description") }}</p>

    <!-- Loading -->
    <div
      v-if="store.loading"
      class="flex items-center justify-center gap-2 py-16 text-on-surface-muted"
    >
      <Loader2 :size="18" class="animate-spin" />
      <span class="text-sm">{{ t("common.loading") }}</span>
    </div>

    <!-- Error -->
    <div
      v-else-if="store.error"
      class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2 mb-4"
    >
      {{ store.error }}
    </div>

    <!-- Empty -->
    <div
      v-else-if="store.knownHosts.length === 0"
      class="flex flex-col items-center justify-center py-16 text-on-surface-faint"
    >
      <Server :size="40" class="mb-3 opacity-40" />
      <p class="text-sm">{{ t("knownHosts.empty") }}</p>
    </div>

    <!-- List -->
    <ul v-else class="grid grid-cols-1 gap-2">
      <li
        v-for="kh in store.knownHosts"
        :key="kh.id"
        class="bg-surface-card border border-edge rounded p-3 flex items-start gap-3"
      >
        <div
          class="w-8 h-8 rounded bg-surface-alt flex items-center justify-center text-on-surface-muted shrink-0"
        >
          <Fingerprint :size="16" />
        </div>
        <div class="flex-1 min-w-0">
          <div class="flex items-center gap-2 mb-0.5">
            <span class="font-mono text-sm text-on-surface truncate">
              {{ kh.hostname }}
            </span>
            <span
              class="inline-flex items-center text-[10px] text-on-surface-muted bg-surface-alt px-1.5 py-0.5 rounded font-mono"
            >
              {{ kh.keyType }}
            </span>
          </div>
          <code class="block text-xs text-on-surface-muted break-all font-mono leading-relaxed">
            {{ kh.fingerprint }}
          </code>
          <p class="text-[11px] text-on-surface-faint mt-1">
            {{ t("knownHosts.acceptedAt") }} {{ formatDate(kh.acceptedAt) }}
          </p>
        </div>
        <button
          class="inline-flex items-center justify-center w-8 h-8 rounded hover:bg-surface-alt text-on-surface-muted hover:text-danger-text shrink-0"
          :title="t('hosts.delete')"
          @click="deleteHost(kh)"
        >
          <Trash2 :size="16" />
        </button>
      </li>
    </ul>

    <ConfirmDialog
      :open="toDelete !== null"
      :title="t('knownHosts.deleteTitle')"
      :description="
        toDelete
          ? `${toDelete.hostname} (${toDelete.keyType}) — ${t('dialog.deleteDescription')}`
          : t('dialog.deleteDescription')
      "
      :confirm-label="t('form.delete')"
      :cancel-label="t('form.cancel')"
      variant="danger"
      @update:open="
        (v) => {
          if (!v) toDelete = null;
        }
      "
      @confirm="onConfirmDelete"
      @cancel="toDelete = null"
    />

    <ConfirmDialog
      :open="clearOpen"
      :title="t('knownHosts.clearTitle')"
      :description="t('knownHosts.clearDescription')"
      :confirm-label="t('knownHosts.clearAll')"
      :cancel-label="t('form.cancel')"
      variant="danger"
      @update:open="
        (v) => {
          if (!v) clearOpen = false;
        }
      "
      @confirm="onConfirmClear"
      @cancel="clearOpen = false"
    />
  </div>

  <KnownHostsImportModal v-model:open="importOpen" @imported="store.fetchAll()" />
</template>
