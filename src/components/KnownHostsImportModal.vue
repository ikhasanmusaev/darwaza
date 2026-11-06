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
import { X, Download, AlertTriangle, Check } from "lucide-vue-next";
import { errorText } from "../lib/errors";
import {
  knownHostsImportApi,
  type KnownHostsImportItem,
  type KnownHostsImportPreview,
} from "../api/known_hosts";

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ "update:open": [value: boolean]; imported: [] }>();

const { t } = useI18n();

const preview = ref<KnownHostsImportPreview | null>(null);
const loading = ref(false);
const importing = ref(false);
const error = ref<string | null>(null);
/** Ключ выбора — hostname+keyType: именно эта пара уникальна в нашей таблице. */
const selected = ref<Set<string>>(new Set());

function keyOf(i: KnownHostsImportItem): string {
  return `${i.hostname}|${i.keyType}`;
}

async function load() {
  loading.value = true;
  error.value = null;
  preview.value = null;
  try {
    const data = await knownHostsImportApi.preview();
    preview.value = data;
    // По умолчанию отмечаем только новые записи. Конфликтующие — намеренно нет:
    // их импорт перезапишет ключ, принятый через TOFU, и это должно быть
    // осознанным действием, а не следствием нажатия "выбрать всё".
    selected.value = new Set(data.items.filter((i) => !i.alreadyKnown && !i.conflicts).map(keyOf));
  } catch (e) {
    error.value = errorText(e);
  } finally {
    loading.value = false;
  }
}

watch(
  () => props.open,
  (open) => {
    if (open) load();
  },
  { immediate: true },
);

function toggle(item: KnownHostsImportItem) {
  const key = keyOf(item);
  const next = new Set(selected.value);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  selected.value = next;
}

const selectedCount = computed(() => selected.value.size);

const skippedTotal = computed(() => {
  const p = preview.value;
  if (!p) return 0;
  return p.skippedHashed + p.skippedWildcard + p.skippedMarker + p.skippedUnparsable;
});

async function runImport() {
  const p = preview.value;
  if (!p || selectedCount.value === 0) return;
  importing.value = true;
  error.value = null;
  try {
    const entries = p.items
      .filter((i) => selected.value.has(keyOf(i)))
      .map((i) => ({
        hostname: i.hostname,
        keyType: i.keyType,
        fingerprint: i.fingerprint,
      }));
    await knownHostsImportApi.run(entries);
    emit("imported");
    emit("update:open", false);
  } catch (e) {
    error.value = errorText(e);
  } finally {
    importing.value = false;
  }
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(v) => emit('update:open', v)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 bg-scrim z-40" />
      <DialogContent
        class="fixed z-50 top-1/2 left-1/2 dialog-center bg-surface-card border border-edge rounded-lg shadow-xl w-full max-w-2xl flex flex-col max-h-[80vh] focus:outline-none"
      >
        <div class="flex items-start justify-between p-4 border-b border-edge">
          <div class="min-w-0">
            <DialogTitle class="text-base font-semibold text-on-surface">
              {{ t("knownHosts.importTitle") }}
            </DialogTitle>
            <DialogDescription class="text-xs text-on-surface-faint mt-0.5 truncate">
              {{ preview?.path ?? t("knownHosts.importDescription") }}
            </DialogDescription>
          </div>
          <DialogClose as-child>
            <button
              type="button"
              class="inline-flex items-center justify-center w-8 h-8 rounded hover:bg-surface-alt text-on-surface-muted shrink-0"
            >
              <X :size="18" />
            </button>
          </DialogClose>
        </div>

        <div class="p-4 overflow-auto">
          <div
            v-if="error"
            class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2 mb-3"
          >
            {{ error }}
          </div>

          <p v-if="loading" class="text-sm text-on-surface-faint py-4">
            {{ t("common.loading") }}
          </p>

          <template v-else-if="preview">
            <p v-if="preview.items.length === 0" class="text-sm text-on-surface-faint py-4">
              {{ t("knownHosts.importEmpty") }}
            </p>

            <ul v-else class="space-y-1" data-testid="known-hosts-import-list">
              <li
                v-for="item in preview.items"
                :key="keyOf(item)"
                class="flex items-start gap-2 px-2 py-1.5 rounded hover:bg-surface-alt"
              >
                <input
                  type="checkbox"
                  class="mt-1 accent-accent"
                  :checked="selected.has(keyOf(item))"
                  :disabled="item.alreadyKnown"
                  @change="toggle(item)"
                />
                <div class="min-w-0 flex-1">
                  <div class="flex items-center gap-2">
                    <span class="text-sm text-on-surface truncate">{{ item.hostname }}</span>
                    <span class="text-[10px] uppercase text-on-surface-faint">
                      {{ item.keyType }}
                    </span>
                    <span
                      v-if="item.alreadyKnown"
                      class="inline-flex items-center gap-1 text-[10px] text-success-text"
                    >
                      <Check :size="11" />{{ t("knownHosts.importAlready") }}
                    </span>
                    <span
                      v-else-if="item.conflicts"
                      class="inline-flex items-center gap-1 text-[10px] text-danger-text"
                    >
                      <AlertTriangle :size="11" />{{ t("knownHosts.importConflict") }}
                    </span>
                  </div>
                  <code class="block text-[11px] font-mono text-on-surface-faint truncate">
                    {{ item.fingerprint }}
                  </code>
                  <code
                    v-if="item.conflicts && item.currentFingerprint"
                    class="block text-[11px] font-mono text-danger-text/80 truncate"
                  >
                    {{ t("knownHosts.importCurrent") }}: {{ item.currentFingerprint }}
                  </code>
                </div>
              </li>
            </ul>

            <p v-if="skippedTotal > 0" class="mt-3 text-[11px] text-on-surface-faint">
              {{ t("knownHosts.importSkipped", { count: skippedTotal }) }}:
              <span v-if="preview.skippedHashed">
                {{ t("knownHosts.skipHashed", { count: preview.skippedHashed }) }}
              </span>
              <span v-if="preview.skippedWildcard">
                · {{ t("knownHosts.skipWildcard", { count: preview.skippedWildcard }) }}
              </span>
              <span v-if="preview.skippedMarker">
                · {{ t("knownHosts.skipMarker", { count: preview.skippedMarker }) }}
              </span>
              <span v-if="preview.skippedUnparsable">
                · {{ t("knownHosts.skipUnparsable", { count: preview.skippedUnparsable }) }}
              </span>
            </p>
          </template>
        </div>

        <div class="flex items-center gap-2 p-4 border-t border-edge">
          <span class="text-xs text-on-surface-faint">
            {{ selectedCount }} {{ t("hosts.importSelectedCount") }}
          </span>
          <button
            type="button"
            data-testid="known-hosts-import-run"
            class="ml-auto inline-flex items-center gap-1.5 px-3 py-2 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60"
            :disabled="selectedCount === 0 || importing"
            @click="runImport"
          >
            <Download :size="16" />
            {{ importing ? t("common.saving") : t("hosts.importSelected") }}
          </button>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
