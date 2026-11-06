<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { Plus, Pencil, Trash2, Play, Save, X, Terminal } from "lucide-vue-next";
import { useSnippetsStore } from "../stores/snippets";
import { useSessionsStore } from "../stores/sessions";
import { expandSnippet, SNIPPET_VARS } from "../lib/snippet-vars";
import type { Snippet } from "../api/snippets";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import PageHeader from "../components/PageHeader.vue";
import { errorText } from "../lib/errors";

const { t } = useI18n();
const store = useSnippetsStore();
const sessionsStore = useSessionsStore();

onMounted(async () => {
  await store.ensureLoaded();
  const first = store.snippets[0];
  if (first) startEdit(first);
});

const editingId = ref<number | null>(null);
const formName = ref("");
const formCommand = ref("");
const formDescription = ref("");
const formAgentAccess = ref(false);
const saving = ref(false);
const error = ref<string | null>(null);
const searchQuery = ref("");

const isEditing = computed(() => editingId.value !== null);
const canSave = computed(
  () => formName.value.trim().length > 0 && formCommand.value.trim().length > 0,
);
const filteredSnippets = computed(() => {
  const query = searchQuery.value.trim().toLowerCase();
  if (!query) return store.snippets;
  return store.snippets.filter(
    (snippet) =>
      snippet.name.toLowerCase().includes(query) || snippet.command.toLowerCase().includes(query),
  );
});
const targetSession = computed(() => {
  const active = sessionsStore.activeSession;
  return active && active.status === "CONNECTED" ? active : null;
});
const commandPreview = computed(() => expandSnippet(formCommand.value, store.activeContext()));

function varLabel(name: string): string {
  return `{{${name}}}`;
}

function resetForm() {
  editingId.value = null;
  formName.value = "";
  formCommand.value = "";
  formDescription.value = "";
  formAgentAccess.value = false;
  error.value = null;
}

function startEdit(s: Snippet) {
  editingId.value = s.id;
  formName.value = s.name;
  formCommand.value = s.command;
  formDescription.value = s.description ?? "";
  formAgentAccess.value = s.agentAccess;
  error.value = null;
}

async function save() {
  if (!canSave.value) return;
  saving.value = true;
  error.value = null;
  const data = {
    name: formName.value.trim(),
    command: formCommand.value,
    description: formDescription.value.trim() || null,
    agentAccess: formAgentAccess.value,
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

const runError = ref<string | null>(null);

async function run(s: Snippet) {
  runError.value = null;
  try {
    const sent = await store.runOnActiveSession(s);
    if (!sent) runError.value = t("snippets.noActiveSession");
  } catch (e) {
    runError.value = errorText(e);
  }
}

async function runForm() {
  if (!formCommand.value.trim()) return;
  await run({
    id: editingId.value ?? -1,
    name: formName.value,
    command: formCommand.value,
    description: formDescription.value || null,
    agentAccess: formAgentAccess.value,
    sortOrder: 0,
    createdAt: "",
    updatedAt: "",
  });
}

const toDelete = ref<Snippet | null>(null);

function askDelete(s: Snippet) {
  toDelete.value = s;
}

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
</script>

<template>
  <div class="p-6">
    <PageHeader
      v-model:search-query="searchQuery"
      :title="t('snippets.title')"
      :description="t('snippets.description')"
      :search-placeholder="t('hosts.search')"
      :count-label="t('snippets.count', { count: filteredSnippets.length })"
    >
      <button
        type="button"
        class="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-r8 bg-accent text-on-accent hover:bg-accent-hover"
        @click="resetForm"
      >
        <Plus :size="16" />
        {{ t("snippets.addTitle") }}
      </button>
    </PageHeader>

    <div class="mb-3 text-xs text-on-surface-muted">
      <span v-if="targetSession" class="text-success-text">
        {{ t("snippets.target", { name: targetSession.hostLabel }) }}
      </span>
      <span v-else>{{ t("snippets.noActiveSession") }}</span>
    </div>
    <div v-if="runError" class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded-r8 px-3 py-2 mb-3">
      {{ runError }}
    </div>

    <div class="grid grid-cols-1 lg:grid-cols-[minmax(0,2fr)_minmax(0,3fr)] gap-6">
      <div>
        <div v-if="store.loading" class="text-sm text-on-surface-muted py-4">{{ t("common.loading") }}</div>
        <div v-else-if="filteredSnippets.length === 0" class="flex flex-col items-center gap-2 py-16 text-on-surface-muted">
          <Terminal :size="32" />
          <p class="text-sm">{{ t("snippets.empty") }}</p>
        </div>

        <ul v-else class="space-y-2">
          <li v-for="snippet in filteredSnippets" :key="snippet.id">
            <button
              type="button"
              class="w-full rounded-r12 border bg-surface-card px-4 py-3 text-left hover:bg-surface-alt"
              :class="editingId === snippet.id ? 'border-accent' : 'border-edge'"
              @click="startEdit(snippet)"
            >
              <span class="flex items-start justify-between gap-3">
                <span class="text-body font-semibold text-on-surface break-words">{{ snippet.name }}</span>
                <span v-if="snippet.agentAccess" class="shrink-0 px-1.5 py-0.5 rounded-full bg-agent-soft text-[9px] font-mono font-bold text-agent-text">AGENT</span>
              </span>
              <code class="mt-2 block text-xs text-on-surface-muted font-mono whitespace-pre-wrap break-words">{{ snippet.command }}</code>
              <span v-if="snippet.description" class="mt-1 block text-xs text-on-surface-muted">{{ snippet.description }}</span>
            </button>
            <div class="mt-1 flex justify-end gap-1">
              <button type="button" class="p-1.5 rounded-r6 hover:bg-surface-alt text-on-surface-muted" :title="t('hosts.edit')" @click="startEdit(snippet)"><Pencil :size="13" /></button>
              <button type="button" class="p-1.5 rounded-r6 hover:bg-danger-soft text-on-surface-muted hover:text-danger-text" :title="t('hosts.delete')" @click="askDelete(snippet)"><Trash2 :size="13" /></button>
            </div>
          </li>
        </ul>
      </div>

      <div class="rounded-r12 border border-edge bg-surface-card p-5 h-fit" data-testid="snippet-editor">
        <h3 class="text-dialog-title text-on-surface mb-4">{{ isEditing ? t("snippets.editTitle") : t("snippets.addTitle") }}</h3>
        <div v-if="error" class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded-r8 px-3 py-2 mb-3">{{ error }}</div>

        <label class="block text-xs text-on-surface-muted mb-1">{{ t("snippets.name") }}</label>
        <input v-model="formName" type="text" class="w-full bg-surface border border-edge rounded-r8 px-3 py-2 text-sm mb-3 focus:outline-none focus:border-accent" :placeholder="t('snippets.namePlaceholder')" />

        <label class="block text-xs text-on-surface-muted mb-1">{{ t("snippets.command") }}</label>
        <textarea v-model="formCommand" rows="6" class="w-full bg-surface border border-edge rounded-r8 px-3 py-2 text-sm font-mono mb-1 focus:outline-none focus:border-accent" :placeholder="t('snippets.commandPlaceholder')" />
        <p class="text-[11px] text-on-surface-muted mb-3">
          {{ t("snippets.varsHint") }}
          <code v-for="v in SNIPPET_VARS" :key="v" class="font-mono text-on-surface-muted ml-1">{{ varLabel(v) }}</code>
        </p>

        <label class="block text-xs text-on-surface-muted mb-1">{{ t("snippets.descriptionField") }}</label>
        <input v-model="formDescription" type="text" class="w-full bg-surface border border-edge rounded-r8 px-3 py-2 text-sm mb-3 focus:outline-none focus:border-accent" />

        <label class="flex items-center gap-2 text-sm text-accent-text cursor-pointer mb-1">
          <input v-model="formAgentAccess" type="checkbox" data-testid="snippet-agent-access" class="accent-accent" />
          {{ t("snippets.agentAccess") }}
        </label>
        <p class="mb-3 text-[11px] text-warning-text">{{ t("snippets.agentAccessWarning") }}</p>

        <div v-if="commandPreview !== formCommand" class="mb-3 rounded-r8 border border-edge bg-surface px-3 py-2">
          <div class="text-[11px] text-on-surface-muted mb-1">{{ t("snippets.preview") }}</div>
          <code class="block text-xs font-mono text-on-surface whitespace-pre-wrap break-words">{{ commandPreview }}</code>
        </div>

        <div class="flex flex-wrap justify-end gap-2">
          <button v-if="isEditing" type="button" class="inline-flex items-center gap-1.5 px-3 py-2 text-sm rounded-r8 bg-surface-alt hover:bg-edge-subtle text-on-surface" @click="resetForm"><X :size="16" />{{ t("form.cancel") }}</button>
          <button type="button" class="inline-flex items-center gap-1.5 px-3 py-2 text-sm rounded-r8 bg-surface-alt text-on-surface hover:bg-edge-subtle disabled:opacity-45" :disabled="!targetSession || !formCommand.trim()" :title="t('snippets.run')" @click="runForm"><Play :size="15" />{{ t("snippets.run") }}</button>
          <button type="button" class="inline-flex items-center gap-1.5 px-3 py-2 text-sm rounded-r8 bg-accent hover:bg-accent-hover text-on-accent disabled:opacity-45" :disabled="!canSave || saving" @click="save">
            <component :is="isEditing ? Save : Plus" :size="16" />{{ saving ? t("common.saving") : t("form.save") }}
          </button>
        </div>
      </div>
    </div>

    <ConfirmDialog
      :open="toDelete !== null"
      :title="t('dialog.deleteTitle')"
      :description="toDelete ? `${toDelete.name} — ${t('dialog.deleteDescription')}` : t('dialog.deleteDescription')"
      :confirm-label="t('form.delete')"
      :cancel-label="t('form.cancel')"
      variant="danger"
      @update:open="(v: boolean) => { if (!v) toDelete = null; }"
      @confirm="onConfirmDelete"
      @cancel="toDelete = null"
    />
  </div>
</template>
