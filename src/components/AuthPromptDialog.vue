<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { useI18n } from "vue-i18n";
import {
  AlertDialogRoot,
  AlertDialogPortal,
  AlertDialogOverlay,
  AlertDialogContent,
  AlertDialogTitle,
  AlertDialogDescription,
} from "radix-vue";
import { ShieldCheck } from "lucide-vue-next";
import { useAuthPromptStore } from "../stores/auth_prompt";
import { errorText } from "../lib/errors";

/**
 * Диалог интерактивной аутентификации (#153).
 *
 * Пока он открыт, SSH-рукопожатие ждёт ответа на стороне backend, поэтому
 * закрыть его «просто так» нельзя — только ответить или отменить подключение.
 */
const { t } = useI18n();
const store = useAuthPromptStore();

const answers = ref<string[]>([]);
const busy = ref(false);
const error = ref<string | null>(null);

const prompt = computed(() => store.current());

watch(
  prompt,
  (p) => {
    answers.value = p ? p.prompts.map(() => "") : [];
    error.value = null;
  },
  { immediate: true },
);

async function submit() {
  const p = prompt.value;
  if (!p || busy.value) return;
  busy.value = true;
  error.value = null;
  try {
    await store.submit(p.sessionId, [...answers.value]);
  } catch (e) {
    error.value = errorText(e);
  } finally {
    busy.value = false;
    // Введённое не держим в памяти дольше необходимого.
    answers.value = answers.value.map(() => "");
  }
}

async function cancel() {
  const p = prompt.value;
  if (!p) return;
  try {
    await store.cancel(p.sessionId);
  } catch (e) {
    error.value = errorText(e);
  }
}
</script>

<template>
  <AlertDialogRoot :open="prompt !== null">
    <AlertDialogPortal>
      <AlertDialogOverlay class="fixed inset-0 bg-scrim z-40" />
      <AlertDialogContent
        v-if="prompt"
        data-testid="auth-prompt-dialog"
        class="fixed z-50 top-1/2 left-1/2 dialog-center bg-surface-card border border-edge rounded-lg shadow-xl w-full max-w-md p-5 focus:outline-none"
      >
        <div class="flex items-center gap-2 mb-2">
          <ShieldCheck :size="18" class="text-success" />
          <AlertDialogTitle class="text-base font-semibold text-on-surface">
            {{ prompt.name || t("authPrompt.title") }}
          </AlertDialogTitle>
        </div>

        <AlertDialogDescription class="text-xs text-on-surface-muted mb-4">
          {{ prompt.instructions || t("authPrompt.description", { host: prompt.hostname }) }}
        </AlertDialogDescription>

        <div
          v-if="error"
          class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2 mb-3"
        >
          {{ error }}
        </div>

        <div v-for="(field, i) in prompt.prompts" :key="i" class="mb-3">
          <label class="block text-xs text-on-surface-muted mb-1">
            {{ field.prompt }}
          </label>
          <input
            v-model="answers[i]"
            :type="field.echo ? 'text' : 'password'"
            :autofocus="i === 0"
            :data-testid="`auth-prompt-input-${i}`"
            class="w-full bg-surface border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
            @keydown.enter="submit"
          />
        </div>

        <div class="flex justify-end gap-2">
          <button
            type="button"
            class="px-3 py-1.5 text-sm rounded bg-surface-alt hover:bg-surface text-on-surface"
            @click="cancel"
          >
            {{ t("form.cancel") }}
          </button>
          <button
            type="button"
            data-testid="auth-prompt-submit"
            class="px-3 py-1.5 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60"
            :disabled="busy"
            @click="submit"
          >
            {{ busy ? t("common.saving") : t("authPrompt.submit") }}
          </button>
        </div>
      </AlertDialogContent>
    </AlertDialogPortal>
  </AlertDialogRoot>
</template>
