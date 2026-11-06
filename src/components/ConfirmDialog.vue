<script setup lang="ts">
import { nextTick, ref, watch } from "vue";
import {
  AlertDialogRoot,
  AlertDialogPortal,
  AlertDialogOverlay,
  AlertDialogContent,
  AlertDialogTitle,
  AlertDialogDescription,
} from "radix-vue";

const props = defineProps<{
  open: boolean;
  title: string;
  description: string;
  confirmLabel: string;
  cancelLabel: string;
  variant?: "danger" | "default";
}>();

const emit = defineEmits<{
  "update:open": [value: boolean];
  confirm: [];
  cancel: [];
}>();

const cancelButton = ref<HTMLButtonElement | null>(null);
watch(
  () => props.open,
  async (open) => {
    if (!open) return;
    await nextTick();
    cancelButton.value?.focus();
  },
);

/**
 * Кнопки — обычные `<button>`, а не `AlertDialogAction` / `AlertDialogCancel`.
 *
 * У radix-vue обработчик на `AlertDialogAction` закрывает диалог сам и срабатывает
 * РАНЬШЕ нашего `@click`: родитель успевал получить `update:open=false` и обнулить
 * `xToDelete` до того, как до него доходил `confirm` — обработчик подтверждения
 * упирался в `if (!xToDelete) return` и удаление молча не выполнялось (хосты,
 * группы, identities, known_hosts). Закрывая диалог сами, мы контролируем порядок:
 * сначала `confirm`, потом закрытие.
 *
 * Esc и клик по оверлею по-прежнему обрабатывает Root через `update:open`.
 */
function onConfirm() {
  emit("confirm");
  emit("update:open", false);
}

function onCancel() {
  emit("cancel");
  emit("update:open", false);
}
</script>

<template>
  <AlertDialogRoot :open="props.open" @update:open="(v) => emit('update:open', v)">
    <AlertDialogPortal>
      <AlertDialogOverlay class="fixed inset-0 bg-scrim z-40" />
      <AlertDialogContent
        class="fixed z-50 top-1/2 left-1/2 dialog-center bg-surface-card border border-edge rounded-r12 shadow-e2 w-full max-w-md p-6 focus:outline-none"
      >
        <AlertDialogTitle class="text-dialog-title text-on-surface mb-2">
          {{ title }}
        </AlertDialogTitle>

        <AlertDialogDescription class="text-body text-on-surface-muted mb-5">
          {{ description }}
        </AlertDialogDescription>

        <div class="flex flex-wrap justify-end gap-2">
          <button
            ref="cancelButton"
            type="button"
            class="px-3 py-1.5 text-sm rounded-r8 border border-edge bg-surface-alt hover:bg-edge-subtle text-on-surface"
            @click="onCancel"
          >
            {{ cancelLabel }}
          </button>

          <button
            type="button"
            class="px-3 py-1.5 text-sm rounded-r8 border bg-transparent"
            :class="
              variant === 'danger'
                ? 'border-danger/40 text-danger-text hover:bg-danger-soft'
                : 'border-accent text-accent-text hover:bg-accent-soft'
            "
            @click="onConfirm"
          >
            {{ confirmLabel }}
          </button>
        </div>
      </AlertDialogContent>
    </AlertDialogPortal>
  </AlertDialogRoot>
</template>
