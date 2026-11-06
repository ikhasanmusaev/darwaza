<script setup lang="ts">
import { ref, computed } from "vue";
import { useI18n } from "vue-i18n";
import {
  DialogRoot,
  DialogPortal,
  DialogOverlay,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogClose,
} from "radix-vue";
import { X, Plus, Pencil, Trash2, Save } from "lucide-vue-next";
import { useHostsStore } from "../stores/hosts";
import ConfirmDialog from "./ConfirmDialog.vue";
import { errorText } from "../lib/errors";

const props = defineProps<{
  open: boolean;
}>();

const emit = defineEmits<{
  "update:open": [value: boolean];
}>();

const { t } = useI18n();
const store = useHostsStore();

const newGroupName = ref("");
const saving = ref(false);
const error = ref<string | null>(null);

const editingId = ref<number | null>(null);
const editingName = ref("");

const groupToDelete = ref<{ id: number; name: string } | null>(null);

const groupsSorted = computed(() => [...store.groups].sort((a, b) => a.name.localeCompare(b.name)));

async function createGroup() {
  const name = newGroupName.value.trim();
  if (!name) return;
  saving.value = true;
  error.value = null;
  try {
    await store.createGroup({ name });
    newGroupName.value = "";
  } catch (e) {
    error.value = errorText(e);
  } finally {
    saving.value = false;
  }
}

function startEdit(id: number, name: string) {
  editingId.value = id;
  editingName.value = name;
}

function cancelEdit() {
  editingId.value = null;
  editingName.value = "";
}

async function saveEdit(id: number) {
  const name = editingName.value.trim();
  if (!name) return;
  saving.value = true;
  error.value = null;
  try {
    await store.updateGroup(id, { name });
    editingId.value = null;
  } catch (e) {
    error.value = errorText(e);
  } finally {
    saving.value = false;
  }
}

function askDelete(id: number, name: string) {
  groupToDelete.value = { id, name };
}

async function onConfirmDelete() {
  if (!groupToDelete.value) return;
  error.value = null;
  try {
    await store.removeGroup(groupToDelete.value.id);
  } catch (e) {
    error.value = errorText(e);
  } finally {
    groupToDelete.value = null;
  }
}

function onCancelDelete() {
  groupToDelete.value = null;
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(v) => emit('update:open', v)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 bg-scrim z-40" />
      <DialogContent
        class="fixed z-50 top-1/2 left-1/2 dialog-center bg-surface-card border border-edge rounded-lg shadow-xl w-full max-w-md flex flex-col focus:outline-none"
      >
        <!-- Header -->
        <div class="flex items-center justify-between p-4 border-b border-edge">
          <div>
            <DialogTitle class="text-base font-semibold text-on-surface">
              {{ t("groups.title") }}
            </DialogTitle>
            <DialogDescription class="text-xs text-on-surface-faint mt-0.5">
              {{ t("groups.description") }}
            </DialogDescription>
          </div>
          <DialogClose as-child>
            <button
              type="button"
              class="inline-flex items-center justify-center w-8 h-8 rounded hover:bg-surface-alt text-on-surface-muted"
            >
              <X :size="18" />
            </button>
          </DialogClose>
        </div>

        <!-- Body -->
        <div class="p-4">
          <!-- Error -->
          <div
            v-if="error"
            class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2 mb-3"
          >
            {{ error }}
          </div>

          <!-- New group -->
          <div class="flex gap-2 mb-4">
            <input
              v-model="newGroupName"
              type="text"
              class="flex-1 bg-surface border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
              :placeholder="t('groups.newName')"
              @keydown.enter="createGroup"
            />
            <button
              type="button"
              class="inline-flex items-center gap-1 px-3 py-2 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60"
              :disabled="!newGroupName.trim() || saving"
              @click="createGroup"
            >
              <Plus :size="16" />
              {{ t("groups.add") }}
            </button>
          </div>

          <!-- List -->
          <div
            v-if="groupsSorted.length === 0"
            class="text-sm text-on-surface-faint py-4 text-center"
          >
            {{ t("groups.empty") }}
          </div>

          <ul v-else class="space-y-1 max-h-64 overflow-auto">
            <li
              v-for="g in groupsSorted"
              :key="g.id"
              class="flex items-center gap-2 px-2 py-1.5 rounded hover:bg-surface-alt/50"
            >
              <template v-if="editingId === g.id">
                <input
                  v-model="editingName"
                  type="text"
                  class="flex-1 bg-surface border border-edge rounded px-2 py-1 text-sm focus:outline-none focus:border-accent"
                  @keydown.enter="saveEdit(g.id)"
                  @keydown.esc="cancelEdit"
                />
                <button
                  type="button"
                  class="inline-flex items-center justify-center w-7 h-7 rounded hover:bg-surface-alt text-accent-text"
                  @click="saveEdit(g.id)"
                >
                  <Save :size="14" />
                </button>
                <button
                  type="button"
                  class="inline-flex items-center justify-center w-7 h-7 rounded hover:bg-surface-alt text-on-surface-muted"
                  @click="cancelEdit"
                >
                  <X :size="14" />
                </button>
              </template>

              <template v-else>
                <span class="flex-1 text-sm text-on-surface truncate">
                  {{ g.name }}
                </span>
                <button
                  type="button"
                  class="inline-flex items-center justify-center w-7 h-7 rounded hover:bg-surface-alt text-on-surface-muted"
                  @click="startEdit(g.id, g.name)"
                >
                  <Pencil :size="14" />
                </button>
                <button
                  type="button"
                  class="inline-flex items-center justify-center w-7 h-7 rounded hover:bg-surface-alt text-on-surface-muted hover:text-danger-text"
                  @click="askDelete(g.id, g.name)"
                >
                  <Trash2 :size="14" />
                </button>
              </template>
            </li>
          </ul>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>

  <ConfirmDialog
    :open="groupToDelete !== null"
    :title="t('dialog.deleteTitle')"
    :description="
      groupToDelete
        ? `${groupToDelete.name} — ${t('dialog.deleteDescription')}`
        : t('dialog.deleteDescription')
    "
    :confirm-label="t('form.delete')"
    :cancel-label="t('form.cancel')"
    variant="danger"
    @update:open="
      (v) => {
        if (!v) groupToDelete = null;
      }
    "
    @confirm="onConfirmDelete"
    @cancel="onCancelDelete"
  />
</template>
