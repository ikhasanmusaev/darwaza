<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import {
  AlertDialogRoot,
  AlertDialogPortal,
  AlertDialogOverlay,
  AlertDialogContent,
  AlertDialogTitle,
  AlertDialogDescription,
} from "radix-vue";
import { useConflictsStore, type ConflictChoice } from "../stores/conflicts";

/**
 * «Такое имя уже есть»: заменить, оставить оба или пропустить.
 *
 * Фокус по умолчанию — на «Оставить оба»: Enter не должен ничего затирать.
 * Esc и щелчок мимо окна — «пропустить».
 */
const { t } = useI18n();
const store = useConflictsStore();
const forAll = ref(false);
const keepBoth = ref<HTMLButtonElement | null>(null);

const q = computed(() => store.question);

watch(q, async (now) => {
  if (!now) return;
  forAll.value = false;
  await nextTick();
  keepBoth.value?.focus();
});

function choose(action: ConflictChoice["action"]) {
  store.answer({ action, forAll: forAll.value });
}
</script>

<template>
  <AlertDialogRoot :open="q !== null" @update:open="(v) => !v && q && choose('skip')">
    <AlertDialogPortal>
      <AlertDialogOverlay class="fixed inset-0 bg-scrim z-40" />
      <AlertDialogContent
        v-if="q"
        data-testid="conflict-dialog"
        class="fixed z-50 top-1/2 left-1/2 dialog-center bg-surface-card border border-edge rounded-r12 shadow-e2 w-full max-w-md p-6 focus:outline-none"
        @escape-key-down="choose('skip')"
      >
        <AlertDialogTitle class="text-dialog-title text-on-surface mb-2">
          {{ t(q.isDir ? "conflict.titleDir" : "conflict.titleFile") }}
        </AlertDialogTitle>
        <AlertDialogDescription class="text-body text-on-surface-muted mb-4">
          <span class="font-mono text-on-surface break-all">{{ q.name }}</span>
          — {{ t(q.isDir ? "conflict.explainDir" : "conflict.explainFile") }}
        </AlertDialogDescription>

        <label v-if="q.more > 0" class="flex items-center gap-2 mb-5 text-sm text-on-surface">
          <input v-model="forAll" type="checkbox" data-testid="conflict-for-all" />
          {{ t("conflict.forAll", { count: q.more }) }}
        </label>

        <div class="flex flex-wrap justify-end gap-2">
          <button
            type="button"
            data-testid="conflict-skip"
            class="px-3 py-1.5 text-sm rounded-r8 border border-edge bg-surface-alt hover:bg-edge-subtle text-on-surface"
            @click="choose('skip')"
          >
            {{ t("conflict.skip") }}
          </button>
          <button
            ref="keepBoth"
            type="button"
            data-testid="conflict-rename"
            class="px-3 py-1.5 text-sm rounded-r8 border border-accent text-accent-text hover:bg-accent-soft"
            @click="choose('rename')"
          >
            {{ t("conflict.keepBoth") }}
          </button>
          <button
            type="button"
            data-testid="conflict-replace"
            class="px-3 py-1.5 text-sm rounded-r8 border border-danger/40 text-danger-text hover:bg-danger-soft"
            @click="choose('replace')"
          >
            {{ t(q.isDir ? "conflict.merge" : "conflict.replace") }}
          </button>
        </div>
      </AlertDialogContent>
    </AlertDialogPortal>
  </AlertDialogRoot>
</template>
