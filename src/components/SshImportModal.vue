<script setup lang="ts">
import { ref, computed, watch } from "vue";
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
import { Download, X, Loader2, CheckSquare, Square } from "lucide-vue-next";
import { sshApi, type SshConfigEntry } from "../api/ssh";
import { useHostsStore } from "../stores/hosts";
import { errorText } from "../lib/errors";

const props = defineProps<{
  open: boolean;
}>();

const emit = defineEmits<{
  "update:open": [value: boolean];
  imported: [];
}>();

const { t } = useI18n();
const hostsStore = useHostsStore();

const loading = ref(false);
const error = ref<string | null>(null);
const entries = ref<SshConfigEntry[]>([]);
const selected = ref<Set<number>>(new Set());

const allSelected = computed(
  () => entries.value.length > 0 && selected.value.size === entries.value.length,
);

const selectedCount = computed(() => selected.value.size);

async function loadPreview() {
  loading.value = true;
  error.value = null;
  try {
    const preview = await sshApi.configPreview();
    entries.value = preview.entries;
    selected.value = new Set(preview.entries.map((_, i) => i));
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
      loadPreview();
    }
  },
);

function toggleAll() {
  if (allSelected.value) {
    selected.value = new Set();
  } else {
    selected.value = new Set(entries.value.map((_, i) => i));
  }
}

function toggleOne(index: number) {
  const next = new Set(selected.value);
  if (next.has(index)) {
    next.delete(index);
  } else {
    next.add(index);
  }
  selected.value = next;
}

async function importSelected() {
  if (selected.value.size === 0) return;

  error.value = null;
  loading.value = true;
  try {
    for (const idx of selected.value) {
      const e = entries.value[idx];
      await hostsStore.createHost({
        name: e.alias,
        hostname: e.hostname ?? e.alias,
        port: e.port ?? 22,
        username: e.user ?? "",
        authMethod: e.identityFile ? "publickey" : "password",
        groupId: null,
        identityId: null,
        color: null,
        description: e.proxyJump ? `ProxyJump: ${e.proxyJump}` : null,
      });
    }
    emit("imported");
    emit("update:open", false);
  } catch (e) {
    error.value = errorText(e);
  } finally {
    loading.value = false;
  }
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(v) => emit('update:open', v)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 bg-scrim z-40" />
      <DialogContent
        class="fixed z-50 top-1/2 left-1/2 dialog-center bg-surface-card border border-edge rounded-lg shadow-xl w-full max-w-2xl max-h-[80vh] flex flex-col focus:outline-none"
      >
        <!-- Header -->
        <div class="flex items-center justify-between p-4 border-b border-edge">
          <div>
            <DialogTitle class="text-base font-semibold text-on-surface">
              {{ t("hosts.importFromSshConfig") }}
            </DialogTitle>
            <DialogDescription class="text-xs text-on-surface-faint mt-0.5">
              {{ t("hosts.importDescription") }}
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
        <div class="flex-1 overflow-auto p-4">
          <!-- Error -->
          <div
            v-if="error"
            class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2 mb-3"
          >
            {{ error }}
          </div>

          <!-- Loading -->
          <div
            v-if="loading && entries.length === 0"
            class="flex items-center justify-center gap-2 py-12 text-on-surface-muted"
          >
            <Loader2 :size="18" class="animate-spin" />
            <span class="text-sm">{{ t("common.loading") }}</span>
          </div>

          <!-- Empty -->
          <div
            v-else-if="entries.length === 0"
            class="flex flex-col items-center justify-center py-12 text-on-surface-faint"
          >
            <p class="text-sm">{{ t("hosts.importEmpty") }}</p>
          </div>

          <!-- List -->
          <table v-else class="w-full text-sm">
            <thead>
              <tr class="text-left text-on-surface-muted border-b border-edge">
                <th class="pb-2 pr-2 w-8">
                  <button
                    type="button"
                    class="inline-flex items-center text-on-surface-muted hover:text-on-surface"
                    @click="toggleAll"
                  >
                    <CheckSquare v-if="allSelected" :size="16" />
                    <Square v-else :size="16" />
                  </button>
                </th>
                <th class="pb-2 pr-4 font-medium">{{ t("hosts.form.name") }}</th>
                <th class="pb-2 pr-4 font-medium">{{ t("hosts.form.hostname") }}</th>
                <th class="pb-2 pr-4 font-medium">{{ t("hosts.form.username") }}</th>
                <th class="pb-2 font-medium">{{ t("hosts.form.port") }}</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="(e, idx) in entries"
                :key="idx"
                class="border-b border-edge/50 hover:bg-surface-alt/30"
              >
                <td class="py-2 pr-2">
                  <button
                    type="button"
                    class="inline-flex items-center text-on-surface-muted hover:text-on-surface"
                    @click="toggleOne(idx)"
                  >
                    <CheckSquare v-if="selected.has(idx)" :size="16" />
                    <Square v-else :size="16" />
                  </button>
                </td>
                <td class="py-2 pr-4 text-on-surface">{{ e.alias }}</td>
                <td class="py-2 pr-4 text-on-surface-muted">
                  {{ e.hostname ?? "—" }}
                </td>
                <td class="py-2 pr-4 text-on-surface-muted">
                  {{ e.user ?? "—" }}
                </td>
                <td class="py-2 text-on-surface-muted">
                  {{ e.port ?? 22 }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <!-- Footer -->
        <div
          v-if="entries.length > 0"
          class="flex items-center justify-between p-4 border-t border-edge"
        >
          <span class="text-xs text-on-surface-faint">
            {{ selectedCount }} / {{ entries.length }} {{ t("hosts.importSelectedCount") }}
          </span>
          <div class="flex items-center gap-2">
            <DialogClose as-child>
              <button
                type="button"
                class="px-3 py-1.5 text-sm rounded bg-surface-alt hover:bg-surface-alt"
              >
                {{ t("form.cancel") }}
              </button>
            </DialogClose>
            <button
              type="button"
              class="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60 disabled:cursor-not-allowed"
              :disabled="selectedCount === 0 || loading"
              @click="importSelected"
            >
              <Download :size="16" />
              {{ t("hosts.importSelected") }}
            </button>
          </div>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
