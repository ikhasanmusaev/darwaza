<script setup lang="ts">
import { nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import {
  AlertDialogRoot,
  AlertDialogPortal,
  AlertDialogOverlay,
  AlertDialogContent,
  AlertDialogTitle,
  AlertDialogDescription,
} from "radix-vue";
import { ShieldCheck, ShieldAlert, Fingerprint } from "lucide-vue-next";
import { useKnownHostsStore } from "../stores/known_hosts";
import { errorText } from "../lib/errors";

const { t } = useI18n();
const store = useKnownHostsStore();

const busy = ref(false);
const error = ref<string | null>(null);
const rejectButton = ref<HTMLButtonElement | null>(null);

watch(
  () => store.activePrompt?.sessionId,
  async () => {
    error.value = null;
    busy.value = false;
    await nextTick();
    if (store.activePrompt?.kind === "changed") rejectButton.value?.focus();
  },
);

async function accept() {
  if (!store.activePrompt) return;
  busy.value = true;
  error.value = null;
  try {
    await store.accept(store.activePrompt.sessionId);
  } catch (e) {
    error.value = errorText(e);
  } finally {
    busy.value = false;
  }
}

async function reject() {
  if (!store.activePrompt) return;
  busy.value = true;
  error.value = null;
  try {
    await store.reject(store.activePrompt.sessionId);
  } catch (e) {
    error.value = errorText(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <AlertDialogRoot :open="store.activePrompt !== null">
    <AlertDialogPortal>
      <AlertDialogOverlay class="fixed inset-0 z-40 bg-scrim" />
      <AlertDialogContent
        class="fixed left-1/2 top-1/2 z-50 max-w-2xl w-[92vw] dialog-center rounded-lg border bg-surface p-6 shadow-2xl focus:outline-none"
        :class="store.activePrompt?.kind === 'changed' ? 'border-danger/40' : 'border-edge'"
      >
        <div v-if="store.activePrompt" class="flex flex-col gap-4">
          <!-- Header -->
          <div class="flex items-start gap-3">
            <div
              class="w-10 h-10 rounded-full flex items-center justify-center shrink-0"
              :class="
                store.activePrompt.kind === 'changed'
                  ? 'bg-danger-soft text-danger'
                  : 'bg-warning-soft text-warning'
              "
            >
              <ShieldAlert v-if="store.activePrompt.kind === 'changed'" :size="22" />
              <ShieldCheck v-else :size="22" />
            </div>
            <div class="flex-1 min-w-0">
              <AlertDialogTitle
                class="text-base font-semibold"
                :class="
                  store.activePrompt.kind === 'changed' ? 'text-danger-text' : 'text-on-surface'
                "
              >
                {{
                  store.activePrompt.kind === "changed"
                    ? t("tofu.title.changed")
                    : t("tofu.title.new")
                }}
              </AlertDialogTitle>
              <AlertDialogDescription class="text-sm text-on-surface-muted mt-1">
                {{
                  store.activePrompt.kind === "changed"
                    ? t("tofu.description.changed")
                    : t("tofu.description.new")
                }}
              </AlertDialogDescription>
            </div>
          </div>

          <!-- MITM warning for changed key -->
          <div
            v-if="store.activePrompt.kind === 'changed'"
            class="bg-danger-soft border border-danger/40 text-danger-text text-xs rounded px-3 py-2 leading-relaxed"
          >
            {{ t("tofu.mitmWarning") }}
          </div>

          <!-- Details -->
          <div class="space-y-2 text-sm">
            <div class="flex items-baseline gap-2">
              <span class="text-xs text-on-surface-muted w-24 shrink-0">{{
                t("tofu.field.host")
              }}</span>
              <code class="font-mono text-on-surface break-all">{{
                store.activePrompt.hostname
              }}</code>
            </div>
            <div class="flex items-baseline gap-2">
              <span class="text-xs text-on-surface-muted w-24 shrink-0">{{
                t("tofu.field.keyType")
              }}</span>
              <code class="font-mono text-on-surface">{{ store.activePrompt.keyType }}</code>
            </div>
            <div class="flex items-start gap-2">
              <span class="text-xs text-on-surface-muted w-24 shrink-0 flex items-center gap-1">
                <Fingerprint :size="12" />
                {{ t("tofu.field.fingerprint") }}
              </span>
              <code
                class="font-mono text-xs break-all leading-relaxed"
                :class="
                  store.activePrompt.kind === 'changed' ? 'text-danger-text' : 'text-success-text'
                "
              >
                {{ store.activePrompt.fingerprint }}
              </code>
            </div>
            <div v-if="store.activePrompt.previousFingerprint" class="flex items-start gap-2">
              <span class="text-xs text-on-surface-muted w-24 shrink-0">
                {{ t("tofu.field.previousFingerprint") }}
              </span>
              <code
                class="font-mono text-xs break-all leading-relaxed text-on-surface-muted line-through"
              >
                {{ store.activePrompt.previousFingerprint }}
              </code>
            </div>
          </div>

          <!-- Error -->
          <div
            v-if="error"
            class="bg-danger-soft border border-danger/40 text-danger-text text-xs rounded px-3 py-2"
          >
            {{ error }}
          </div>

          <!-- Actions -->
          <div class="flex justify-end gap-2 pt-2">
            <button
              ref="rejectButton"
              type="button"
              class="px-4 py-2 text-sm rounded bg-surface-alt hover:bg-surface-card disabled:opacity-60"
              :disabled="busy"
              @click="reject"
            >
              {{ t("tofu.action.reject") }}
            </button>
            <button
              type="button"
              class="px-4 py-2 text-sm rounded disabled:opacity-60"
              :class="
                store.activePrompt.kind === 'changed'
                  ? 'border border-danger/40 bg-transparent text-danger-text hover:bg-danger-soft'
                  : 'bg-accent text-on-accent hover:bg-accent-hover active:bg-accent-active'
              "
              :disabled="busy"
              @click="accept"
            >
              {{
                store.activePrompt.kind === "changed"
                  ? t("tofu.action.acceptChanged")
                  : t("tofu.action.accept")
              }}
            </button>
          </div>
        </div>
      </AlertDialogContent>
    </AlertDialogPortal>
  </AlertDialogRoot>
</template>
