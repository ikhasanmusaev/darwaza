<script setup lang="ts">
import { ref, watch, nextTick } from "vue";
import { useI18n } from "vue-i18n";
import { X } from "lucide-vue-next";
import HostForm from "./HostForm.vue";

const props = defineProps<{
  open: boolean;
  hostId?: number | null;
}>();

const emit = defineEmits<{
  "update:open": [value: boolean];
  saved: [];
}>();

const { t } = useI18n();
const panelRef = ref<HTMLElement | null>(null);

function close() {
  emit("update:open", false);
}

function onSaved() {
  emit("saved");
  close();
}

watch(
  () => props.open,
  (isOpen) => {
    if (isOpen) {
      nextTick(() => {
        panelRef.value?.focus();
      });
    }
  },
);

function onBackdropClick(e: MouseEvent) {
  if (e.target === e.currentTarget) {
    close();
  }
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    e.preventDefault();
    close();
  }
}
</script>

<template>
  <Teleport to="body">
    <Transition
      enter-active-class="transition-opacity duration-200"
      leave-active-class="transition-opacity duration-200"
      enter-from-class="opacity-0"
      leave-to-class="opacity-0"
    >
      <div v-if="open" class="fixed inset-0 z-40 bg-scrim" @click="onBackdropClick" />
    </Transition>

    <Transition
      enter-active-class="transition-transform duration-300 ease-out"
      leave-active-class="transition-transform duration-200 ease-in"
      enter-from-class="panel-enter-from"
      leave-to-class="panel-leave-to"
    >
      <div
        v-if="open"
        ref="panelRef"
        tabindex="-1"
        class="fixed right-0 top-0 bottom-0 z-50 w-full max-w-lg bg-surface border-l border-edge shadow-2xl outline-none overflow-y-auto"
        @keydown="onKeydown"
      >
        <!-- Header -->
        <div
          class="sticky top-0 bg-surface border-b border-edge px-6 py-4 flex items-center justify-between"
        >
          <h2 class="text-lg font-semibold">
            {{ hostId != null ? t("hosts.editTitle") : t("hosts.add") }}
          </h2>
          <button
            type="button"
            class="inline-flex items-center justify-center w-8 h-8 rounded hover:bg-surface-alt text-on-surface-muted"
            :title="t('form.cancel')"
            @click="close"
          >
            <X :size="18" />
          </button>
        </div>

        <!-- Form -->
        <div class="px-6 py-4">
          <HostForm :host-id="hostId" @save="onSaved" @cancel="close" />
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
