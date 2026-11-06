<script setup lang="ts">
import { ref, computed } from "vue";
import { useI18n } from "vue-i18n";
import {
  AlertDialogRoot,
  AlertDialogPortal,
  AlertDialogOverlay,
  AlertDialogContent,
  AlertDialogTitle,
  AlertDialogDescription,
} from "radix-vue";
import { KeyRound, Copy, Check, Download } from "lucide-vue-next";
import { onboardingApi } from "../api/onboarding";
import { errorText } from "../lib/errors";

const props = withDefaults(
  defineProps<{
    recoveryKey: string;
    kind?: "master" | "account";
  }>(),
  { kind: "master" },
);

const emit = defineEmits<{
  done: [];
}>();

const { t } = useI18n();
const saved = ref(false);
const understood = ref(false);
const copied = ref(false);
const saveOpen = ref(false);
const savePath = ref("");
const saveError = ref<string | null>(null);
const savedToFile = ref(false);
const saving = ref(false);
const savedCheckbox = ref<HTMLInputElement | null>(null);

const groups = computed(() => props.recoveryKey.split("-"));

function focusSaved(event: Event) {
  event.preventDefault();
  savedCheckbox.value?.focus();
}

async function copy() {
  try {
    await navigator.clipboard.writeText(props.recoveryKey);
    copied.value = true;
    setTimeout(() => (copied.value = false), 2000);
  } catch {
    // Буфер обмена недоступен — ключ всё равно на экране, переписать можно.
  }
}

async function saveToFile() {
  if (!savePath.value.trim() || saving.value) return;
  saving.value = true;
  saveError.value = null;
  savedToFile.value = false;
  try {
    await onboardingApi.saveRecoveryKey(savePath.value.trim(), props.recoveryKey);
    savedToFile.value = true;
  } catch (e) {
    saveError.value = errorText(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <AlertDialogRoot :open="true">
    <AlertDialogPortal>
      <AlertDialogOverlay class="fixed inset-0 z-[110] bg-scrim" />
      <AlertDialogContent
        data-testid="recovery-key-dialog"
        class="fixed z-[110] top-1/2 left-1/2 dialog-center w-full max-w-lg max-h-[90vh] overflow-y-auto bg-surface-card border border-edge rounded-r12 shadow-e2 p-6 focus:outline-none"
        @keydown.esc.prevent.stop
        @open-auto-focus="focusSaved"
      >
        <div class="flex items-center gap-2 mb-2 text-on-surface">
          <KeyRound :size="18" />
          <AlertDialogTitle class="text-dialog-title">
            {{ kind === "account" ? t("recovery.dialogTitleAccount") : t("recovery.dialogTitle") }}
          </AlertDialogTitle>
        </div>
        <AlertDialogDescription class="text-body text-on-surface-muted mb-4">
          {{ kind === "account" ? t("recovery.dialogBodyAccount") : t("recovery.dialogBody") }}
        </AlertDialogDescription>

        <div
          class="grid grid-cols-4 gap-2 font-mono text-recovery-key tracking-wider text-on-surface bg-surface border border-edge rounded-r8 p-4 mb-3 select-all"
          data-testid="recovery-key-value"
        >
          <span v-for="(group, i) in groups" :key="i" class="text-center">{{ group }}</span>
        </div>

        <div class="flex flex-wrap gap-2">
          <button
            type="button"
            class="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-r8 bg-surface-alt hover:bg-edge-subtle text-on-surface"
            @click="copy"
          >
            <Check v-if="copied" :size="14" />
            <Copy v-else :size="14" />
            {{ copied ? t("recovery.copied") : t("recovery.copy") }}
          </button>
          <button
            type="button"
            class="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-r8 bg-surface-alt hover:bg-edge-subtle text-on-surface"
            @click="saveOpen = !saveOpen"
          >
            <Download :size="14" />
            {{ t("recovery.saveToFile") }}
          </button>
        </div>
        <p class="mt-2 text-xs text-warning-text">{{ t("recovery.plainCopyWarning") }}</p>

        <div v-if="saveOpen" class="mt-3 rounded-r8 border border-edge bg-surface p-3">
          <div class="flex flex-wrap gap-2">
            <input
              v-model="savePath"
              type="text"
              data-testid="recovery-save-path"
              class="flex-1 min-w-[220px] bg-surface-input border border-edge rounded-r6 px-3 py-2 text-sm focus:outline-none focus:border-accent"
              :placeholder="t('recovery.savePathPlaceholder')"
              @keydown.enter="saveToFile"
            />
            <button
              type="button"
              data-testid="recovery-save-file"
              class="px-3 py-2 text-sm rounded-r6 bg-accent text-on-accent hover:bg-accent-hover disabled:opacity-45"
              :disabled="!savePath.trim() || saving"
              @click="saveToFile"
            >
              {{ t("form.save") }}
            </button>
          </div>
          <p v-if="savedToFile" class="mt-2 text-xs text-success-text">
            {{ t("recovery.savedToFile") }}
          </p>
          <p v-if="saveError" class="mt-2 text-xs text-danger-text">{{ saveError }}</p>
        </div>

        <ul class="mt-4 text-xs text-warning-text space-y-1 list-disc pl-4">
          <li>{{ t("recovery.warningOnce") }}</li>
          <li>
            {{ kind === "account" ? t("recovery.warningEqualsAccount") : t("recovery.warningEquals") }}
          </li>
          <li>
            {{ kind === "account" ? t("recovery.warningAccount") : t("recovery.warningLocal") }}
          </li>
        </ul>

        <div class="mt-4 space-y-3">
          <label class="flex items-start gap-2 text-sm text-on-surface cursor-pointer">
            <input
              ref="savedCheckbox"
              v-model="saved"
              type="checkbox"
              data-testid="recovery-key-saved"
              class="accent-accent mt-0.5"
            />
            <span>{{ t("recovery.savedConfirm") }}</span>
          </label>
          <label class="flex items-start gap-2 text-sm text-on-surface cursor-pointer">
            <input
              v-model="understood"
              type="checkbox"
              data-testid="recovery-key-understood"
              class="accent-accent mt-0.5"
            />
            <span>{{ t("recovery.copyWarningConfirm") }}</span>
          </label>
        </div>

        <button
          type="button"
          data-testid="recovery-key-done"
          class="mt-5 w-full px-3 py-2 text-sm rounded-r8 bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-45"
          :disabled="!saved || !understood"
          @click="emit('done')"
        >
          {{ t("recovery.done") }}
        </button>
      </AlertDialogContent>
    </AlertDialogPortal>
  </AlertDialogRoot>
</template>
