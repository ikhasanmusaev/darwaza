<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { useI18n } from "vue-i18n";
import { Plus, Pencil, Trash2, Save, X, Network, ChevronDown } from "lucide-vue-next";
import { usePortForwardsStore } from "../stores/port_forwards";
import { useHostsStore } from "../stores/hosts";
import { useSessionsStore } from "../stores/sessions";
import type { PortForward, PortForwardKind } from "../api/port_forwards";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import PageHeader, { type PageFilter } from "../components/PageHeader.vue";
import { errorText } from "../lib/errors";

const { t } = useI18n();
const store = usePortForwardsStore();
const hostsStore = useHostsStore();
const sessionsStore = useSessionsStore();

onMounted(() => {
  store.ensureLoaded();
  hostsStore.ensureLoaded();
});

const editingId = ref<number | null>(null);
const formOpen = ref(false);
const formHostId = ref<number | null>(null);
const formKind = ref<PortForwardKind>("local");
const formLabel = ref("");
const formLocalHost = ref("127.0.0.1");
const formLocalPort = ref<number>(8080);
const formRemoteHost = ref("");
const formRemotePort = ref<number | null>(null);
const formAutoStart = ref(false);
const saving = ref(false);
const error = ref<string | null>(null);
const searchQuery = ref("");
const selectedKind = ref<"all" | PortForwardKind>("all");

const isEditing = computed(() => editingId.value !== null);
const filters = computed<PageFilter[]>(() => [
  { id: "all", label: t("forwards.filterAll") },
  { id: "local", label: t("forwards.kind.local") },
  { id: "remote", label: t("forwards.kind.remote") },
  { id: "dynamic", label: t("forwards.kind.dynamic") },
]);
const filteredForwards = computed(() => {
  const query = searchQuery.value.trim().toLowerCase();
  return store.forwards.filter((forward) => {
    if (selectedKind.value !== "all" && forward.kind !== selectedKind.value) return false;
    if (!query) return true;
    return (
      (forward.label?.toLowerCase().includes(query) ?? false) ||
      routeLabel(forward).toLowerCase().includes(query) ||
      hostName(forward.hostId).toLowerCase().includes(query)
    );
  });
});

watch(
  () => hostsStore.hosts,
  (hosts) => {
    if (formHostId.value == null && hosts.length > 0) formHostId.value = hosts[0].id;
  },
  { immediate: true },
);

const needsRemote = computed(() => formKind.value !== "dynamic");
const canSave = computed(() => {
  if (formHostId.value == null) return false;
  if (!formLocalPort.value && formLocalPort.value !== 0) return false;
  if (!needsRemote.value) return true;
  return formRemoteHost.value.trim().length > 0 && !!formRemotePort.value;
});

function resetForm() {
  editingId.value = null;
  formOpen.value = false;
  formHostId.value = hostsStore.hosts[0]?.id ?? null;
  formKind.value = "local";
  formLabel.value = "";
  formLocalHost.value = "127.0.0.1";
  formLocalPort.value = 8080;
  formRemoteHost.value = "";
  formRemotePort.value = null;
  formAutoStart.value = false;
  error.value = null;
}

function startAdd() {
  resetForm();
  formOpen.value = true;
}

function startEdit(f: PortForward) {
  editingId.value = f.id;
  formOpen.value = true;
  formHostId.value = f.hostId;
  formKind.value = f.kind;
  formLabel.value = f.label ?? "";
  formLocalHost.value = f.localHost;
  formLocalPort.value = f.localPort;
  formRemoteHost.value = f.remoteHost ?? "";
  formRemotePort.value = f.remotePort;
  formAutoStart.value = f.autoStart;
  error.value = null;
}

async function save() {
  if (!canSave.value || formHostId.value == null) return;
  saving.value = true;
  error.value = null;
  const data = {
    hostId: formHostId.value,
    kind: formKind.value,
    label: formLabel.value.trim() || null,
    localHost: formLocalHost.value.trim() || "127.0.0.1",
    localPort: Number(formLocalPort.value),
    remoteHost: needsRemote.value ? formRemoteHost.value.trim() : null,
    remotePort: needsRemote.value ? Number(formRemotePort.value) : null,
    autoStart: formAutoStart.value,
  };
  try {
    if (editingId.value !== null) await store.update(editingId.value, data);
    else await store.create(data);
    resetForm();
  } catch (e) {
    error.value = errorText(e);
  } finally {
    saving.value = false;
  }
}

const opError = ref<string | null>(null);
const opNote = ref<string | null>(null);

function sessionForHost(hostId: number): string | null {
  return (
    sessionsStore.sessions.find((s) => s.hostId === hostId && s.status === "CONNECTED")
      ?.sessionId ?? null
  );
}

async function toggle(f: PortForward) {
  opError.value = null;
  opNote.value = null;
  try {
    if (store.activeById[f.id]) {
      await store.stop(f.id);
      return;
    }
    const sessionId = sessionForHost(f.hostId);
    if (!sessionId) {
      opError.value = t("forwards.needSession");
      return;
    }
    const bound = await store.start(f.id, sessionId);
    opNote.value = t("forwards.started", { bound });
  } catch (e) {
    opError.value = errorText(e);
  }
}

const toDelete = ref<PortForward | null>(null);

async function onConfirmDelete() {
  if (!toDelete.value) return;
  const id = toDelete.value.id;
  toDelete.value = null;
  try {
    await store.remove(id);
    if (editingId.value === id) resetForm();
  } catch (e) {
    error.value = errorText(e);
  }
}

function hostName(hostId: number): string {
  return hostsStore.hosts.find((h) => h.id === hostId)?.name ?? `#${hostId}`;
}

function routeLabel(f: PortForward): string {
  if (f.kind === "dynamic") return `${f.localHost}:${f.localPort} → SOCKS5`;
  if (f.kind === "remote") return `${f.remoteHost}:${f.remotePort} → ${f.localHost}:${f.localPort}`;
  return `${f.localHost}:${f.localPort} → ${f.remoteHost}:${f.remotePort}`;
}
</script>

<template>
  <div class="p-6">
    <PageHeader
      v-model:search-query="searchQuery"
      v-model:selected-filter="selectedKind"
      :title="t('forwards.title')"
      :description="t('forwards.description')"
      :search-placeholder="t('hosts.search')"
      :filters="filters"
      :count-label="t('forwards.count', { count: filteredForwards.length })"
    >
      <button
        type="button"
        class="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-r8 bg-accent text-on-accent hover:bg-accent-hover"
        @click="startAdd"
      >
        <Plus :size="16" />
        {{ t("forwards.addTitle") }}
      </button>
    </PageHeader>

    <div v-if="opError" class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded-r8 px-3 py-2 mb-3">
      {{ opError }}
    </div>
    <div v-else-if="opNote" class="bg-success-soft border border-success/30 text-success-text text-sm rounded-r8 px-3 py-2 mb-3">
      {{ opNote }}
    </div>

    <div v-if="store.loading" class="text-sm text-on-surface-muted py-4">{{ t("common.loading") }}</div>
    <div v-else-if="filteredForwards.length === 0" class="flex flex-col items-center gap-2 py-16 text-on-surface-muted">
      <Network :size="32" />
      <p class="text-sm">{{ t("forwards.empty") }}</p>
    </div>

    <div v-else class="overflow-x-auto">
      <table class="w-full min-w-[860px] border-collapse" data-testid="forwards-table">
        <thead>
          <tr class="border-b border-edge-subtle text-left">
            <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">{{ t("forwards.label") }}</th>
            <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">{{ t("forwards.route") }}</th>
            <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">{{ t("forwards.status") }}</th>
            <th class="px-3 py-2 text-overline uppercase text-on-surface-muted text-right">{{ t("palette.section.actions") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="forward in filteredForwards" :key="forward.id" class="h-16 border-b border-edge-subtle hover:bg-surface-alt">
            <td class="px-3 py-3">
              <div class="flex items-center gap-3">
                <span class="w-9 h-9 rounded-r8 bg-surface-alt flex items-center justify-center text-on-surface-muted">
                  <Network :size="16" />
                </span>
                <div>
                  <div class="text-body font-semibold text-on-surface">{{ forward.label || t(`forwards.kind.${forward.kind}`) }}</div>
                  <div class="text-xs text-on-surface-muted">{{ hostName(forward.hostId) }} · {{ t(`forwards.kind.${forward.kind}`) }}</div>
                </div>
              </div>
            </td>
            <td class="px-3 py-3 font-mono text-mono text-on-surface-muted break-all">{{ routeLabel(forward) }}</td>
            <td class="px-3 py-3">
              <button
                type="button"
                class="inline-flex items-center gap-2 text-xs"
                :class="store.activeById[forward.id] ? 'text-success-text' : 'text-on-surface-muted'"
                :title="store.activeById[forward.id] ? t('forwards.stop') : t('forwards.start')"
                @click="toggle(forward)"
              >
                <span class="w-2 h-2 rounded-full" :class="store.activeById[forward.id] ? 'bg-success' : 'border border-edge'" />
                {{ store.activeById[forward.id] ? t("forwards.active") : t("forwards.inactive") }}
              </button>
            </td>
            <td class="px-3 py-3">
              <div class="flex justify-end gap-1">
                <button type="button" class="p-2 rounded-r8 hover:bg-surface-card text-on-surface-muted" :title="t('hosts.edit')" @click="startEdit(forward)">
                  <Pencil :size="15" />
                </button>
                <button type="button" class="p-2 rounded-r8 hover:bg-danger-soft text-on-surface-muted hover:text-danger-text" :title="t('hosts.delete')" @click="toDelete = forward">
                  <Trash2 :size="15" />
                </button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <Teleport to="body">
      <div v-if="formOpen" class="fixed inset-0 z-50 bg-scrim flex items-center justify-center p-6" @keydown.esc="resetForm">
        <div role="dialog" aria-modal="true" data-testid="forward-form-dialog" class="w-full max-w-lg max-h-[90vh] overflow-y-auto rounded-r12 border border-edge bg-surface-card shadow-e2 p-6">
          <div class="flex items-center justify-between mb-4">
            <h3 class="text-dialog-title text-on-surface">{{ isEditing ? t("forwards.editTitle") : t("forwards.addTitle") }}</h3>
            <button type="button" class="p-1 rounded-r6 text-on-surface-muted hover:bg-surface-alt" :title="t('form.cancel')" @click="resetForm"><X :size="17" /></button>
          </div>

          <div v-if="error" class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded-r8 px-3 py-2 mb-3">{{ error }}</div>

          <label class="block text-xs text-on-surface-muted mb-1">{{ t("forwards.host") }}</label>
          <div class="relative mb-3">
            <select v-model="formHostId" data-testid="forward-host-select" class="w-full appearance-none bg-surface border border-edge rounded-r8 px-3 py-2 pr-9 text-sm focus:outline-none focus:border-accent">
              <option v-for="host in hostsStore.hosts" :key="host.id" :value="host.id">{{ host.name }} ({{ host.username }}@{{ host.hostname }})</option>
            </select>
            <ChevronDown :size="14" class="absolute right-3 top-1/2 icon-center-y pointer-events-none text-on-surface-muted" />
          </div>

          <label class="block text-xs text-on-surface-muted mb-1">{{ t("forwards.kindLabel") }}</label>
          <div class="relative mb-1">
            <select v-model="formKind" data-testid="forward-kind-select" class="w-full appearance-none bg-surface border border-edge rounded-r8 px-3 py-2 pr-9 text-sm focus:outline-none focus:border-accent">
              <option value="local">{{ t("forwards.kind.local") }}</option>
              <option value="dynamic">{{ t("forwards.kind.dynamic") }}</option>
              <option value="remote">{{ t("forwards.kind.remote") }}</option>
            </select>
            <ChevronDown :size="14" class="absolute right-3 top-1/2 icon-center-y pointer-events-none text-on-surface-muted" />
          </div>
          <p class="text-[11px] text-on-surface-muted mb-3">{{ t(`forwards.hint.${formKind}`) }}</p>

          <label class="block text-xs text-on-surface-muted mb-1">{{ t("forwards.label") }}</label>
          <input v-model="formLabel" type="text" class="w-full bg-surface border border-edge rounded-r8 px-3 py-2 text-sm mb-3 focus:outline-none focus:border-accent" :placeholder="t('forwards.labelPlaceholder')" />

          <div class="grid grid-cols-2 gap-2 mb-3">
            <div>
              <label class="block text-xs text-on-surface-muted mb-1">{{ t("forwards.localHost") }}</label>
              <input v-model="formLocalHost" type="text" data-testid="forward-local-host" class="w-full bg-surface border border-edge rounded-r8 px-3 py-2 text-sm focus:outline-none focus:border-accent" />
              <p v-if="formLocalHost.trim() !== '127.0.0.1'" class="mt-1 text-xs text-warning-text" data-testid="forward-network-warning">{{ t("forwards.networkWarning") }}</p>
            </div>
            <div>
              <label class="block text-xs text-on-surface-muted mb-1">{{ t("forwards.localPort") }}</label>
              <input v-model.number="formLocalPort" type="number" min="0" max="65535" data-testid="forward-local-port" class="w-full bg-surface border border-edge rounded-r8 px-3 py-2 text-sm focus:outline-none focus:border-accent" />
            </div>
          </div>

          <div v-if="needsRemote" class="grid grid-cols-2 gap-2 mb-3">
            <div>
              <label class="block text-xs text-on-surface-muted mb-1">{{ t("forwards.remoteHost") }}</label>
              <input v-model="formRemoteHost" type="text" data-testid="forward-remote-host" class="w-full bg-surface border border-edge rounded-r8 px-3 py-2 text-sm focus:outline-none focus:border-accent" placeholder="127.0.0.1" />
            </div>
            <div>
              <label class="block text-xs text-on-surface-muted mb-1">{{ t("forwards.remotePort") }}</label>
              <input v-model.number="formRemotePort" type="number" min="1" max="65535" data-testid="forward-remote-port" class="w-full bg-surface border border-edge rounded-r8 px-3 py-2 text-sm focus:outline-none focus:border-accent" />
            </div>
          </div>

          <label class="flex items-center gap-2 text-sm text-on-surface cursor-pointer mb-4">
            <input v-model="formAutoStart" type="checkbox" class="accent-accent" />
            {{ t("forwards.autoStartLabel") }}
          </label>

          <div class="flex justify-end gap-2">
            <button type="button" class="inline-flex items-center gap-1.5 px-3 py-2 text-sm rounded-r8 bg-surface-alt hover:bg-edge-subtle text-on-surface" @click="resetForm"><X :size="16" />{{ t("form.cancel") }}</button>
            <button type="button" class="inline-flex items-center gap-1.5 px-3 py-2 text-sm rounded-r8 bg-accent hover:bg-accent-hover text-on-accent disabled:opacity-45" :disabled="!canSave || saving" @click="save">
              <component :is="isEditing ? Save : Plus" :size="16" />{{ saving ? t("common.saving") : t("form.save") }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <ConfirmDialog
      :open="toDelete !== null"
      :title="t('dialog.deleteTitle')"
      :description="toDelete ? `${toDelete.label || routeLabel(toDelete)} — ${t('dialog.deleteDescription')}` : t('dialog.deleteDescription')"
      :confirm-label="t('form.delete')"
      :cancel-label="t('form.cancel')"
      variant="danger"
      @update:open="(v: boolean) => { if (!v) toDelete = null; }"
      @confirm="onConfirmDelete"
      @cancel="toDelete = null"
    />
  </div>
</template>
