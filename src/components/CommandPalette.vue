<script setup lang="ts">
import { ref, computed, watch, nextTick } from "vue";
import { useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent } from "radix-vue";
import { Search, Server, Key, Zap, Terminal } from "lucide-vue-next";
import { useCommandPaletteStore } from "../stores/command-palette";
import { useHostsStore } from "../stores/hosts";
import { useIdentitiesStore } from "../stores/identities";
import { useCommandItems, fuzzyFilter, type CommandItem } from "../lib/command-registry";

const { t } = useI18n();
const router = useRouter();
const paletteStore = useCommandPaletteStore();

const query = ref("");
const selectedIndex = ref(0);
const inputRef = ref<HTMLInputElement | null>(null);

// Load stores data lazily when palette opens
const allItems = computed(() => useCommandItems(router));
const filtered = computed(() => fuzzyFilter(allItems.value, query.value));

const grouped = computed(() => {
  const map = new Map<string, CommandItem[]>();
  for (const item of filtered.value) {
    const list = map.get(item.section) ?? [];
    list.push(item);
    map.set(item.section, list);
  }
  return map;
});

const flatList = computed(() => filtered.value);

watch(
  () => paletteStore.isOpen,
  async (isOpen) => {
    if (isOpen) {
      query.value = "";
      selectedIndex.value = 0;
      // Ensure stores are loaded before generating command items
      await useHostsStore().ensureLoaded();
      await useIdentitiesStore().ensureLoaded();
      nextTick(() => inputRef.value?.focus());
    }
  },
);

watch(query, () => {
  selectedIndex.value = 0;
});

function selectNext() {
  selectedIndex.value = Math.min(selectedIndex.value + 1, flatList.value.length - 1);
}

function selectPrev() {
  selectedIndex.value = Math.max(selectedIndex.value - 1, 0);
}

function activate(index: number) {
  const item = flatList.value[index];
  if (!item) return;
  item.run();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "ArrowDown") {
    e.preventDefault();
    selectNext();
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    selectPrev();
  } else if (e.key === "Enter") {
    e.preventDefault();
    activate(selectedIndex.value);
  } else if (e.key === "Escape") {
    e.preventDefault();
    paletteStore.close();
  }
}

function sectionIcon(section: string) {
  switch (section) {
    case "hosts":
      return Server;
    case "identities":
      return Key;
    case "snippets":
      return Terminal;
    case "actions":
      return Zap;
    default:
      return Zap;
  }
}

function sectionLabel(section: string) {
  switch (section) {
    case "hosts":
      return t("palette.section.hosts");
    case "identities":
      return t("palette.section.identities");
    case "snippets":
      return t("palette.section.snippets");
    case "actions":
      return t("palette.section.actions");
    default:
      return section;
  }
}
</script>

<template>
  <DialogRoot
    :open="paletteStore.isOpen"
    @update:open="
      (v) => {
        if (!v) paletteStore.close();
      }
    "
  >
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 bg-scrim z-40" @click="paletteStore.close" />
      <DialogContent
        class="fixed z-50 top-[15%] left-1/2 dialog-center-x bg-surface-card border border-edge rounded-r12 shadow-e2 w-full max-w-2xl flex flex-col focus:outline-none"
        @keydown="onKeydown"
      >
        <div class="flex items-center gap-3 px-4 py-3 border-b border-edge-subtle">
          <Search :size="20" class="text-on-surface-muted shrink-0" />
          <input
            ref="inputRef"
            v-model="query"
            type="text"
            class="flex-1 min-w-0 bg-transparent text-on-surface text-body focus:outline-none"
            :placeholder="t('palette.placeholder')"
          />
          <kbd
            class="font-mono text-[10px] text-on-surface-muted bg-surface-alt px-1.5 py-0.5 rounded-r6"
            >ESC</kbd
          >
        </div>

        <div class="max-h-[60vh] overflow-auto p-2">
          <div
            v-if="flatList.length === 0"
            class="px-4 py-8 text-body text-on-surface-muted text-center"
          >
            {{ t("palette.empty") }}
          </div>

          <template v-for="[section, items] in grouped" :key="section">
            <div
              class="px-3 pb-1 pt-3 text-overline text-on-surface-muted uppercase flex items-center gap-1.5"
            >
              <component :is="sectionIcon(section)" :size="12" />
              {{ sectionLabel(section) }} ({{ items.length }})
            </div>

            <button
              v-for="item in items"
              :key="item.id"
              type="button"
              class="w-full min-h-10 text-left px-3 py-2 rounded-r8 flex items-center gap-2.5 text-body"
              :class="
                flatList.findIndex((i) => i.id === item.id) === selectedIndex
                  ? 'bg-surface-alt text-on-surface shadow-[inset_2px_0_0_rgb(var(--c-accent))]'
                  : 'text-on-surface-muted hover:bg-surface-alt'
              "
              @click="activate(flatList.findIndex((i) => i.id === item.id))"
            >
              <component :is="item.icon" :size="16" class="shrink-0 text-on-surface-muted" />
              <span class="min-w-0 break-words">{{ item.label }}</span>
              <span v-if="item.hint" class="ml-auto text-xs text-on-surface-muted truncate">
                {{ item.hint }}
              </span>
            </button>
          </template>
        </div>

        <div
          class="px-4 py-2 border-t border-edge-subtle text-[11px] text-on-surface-muted flex flex-wrap items-center gap-x-4 gap-y-1"
        >
          <span><kbd class="font-mono">↑↓</kbd> {{ t("palette.navigate") }}</span>
          <span><kbd class="font-mono">↵</kbd> {{ t("palette.select") }}</span>
          <span><kbd class="font-mono">ESC</kbd> {{ t("palette.close") }}</span>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
