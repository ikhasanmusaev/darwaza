<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import {
  AlertDialogRoot,
  AlertDialogPortal,
  AlertDialogOverlay,
  AlertDialogContent,
  AlertDialogTitle,
  AlertDialogDescription,
} from "radix-vue";
import { Bot } from "lucide-vue-next";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { useAgentPlansStore, type PendingPlan } from "../stores/agent_plans";

const { t } = useI18n();
const store = useAgentPlansStore();
const plan = computed(() => store.current());
const goneReason = computed(() => {
  if (!plan.value) return undefined;
  return store.gone[plan.value.id] ?? (plan.value.expiresAt <= now.value ? "expired" : undefined);
});
const isGone = computed(() => goneReason.value !== undefined);
const rejectButton = ref<HTMLButtonElement | null>(null);
const now = ref(Date.now());
let unlisten: UnlistenFn | null = null;
let clock: ReturnType<typeof setInterval> | null = null;

onMounted(async () => {
  unlisten = await store.subscribe();
  clock = setInterval(() => (now.value = Date.now()), 1000);
});

onBeforeUnmount(() => {
  unlisten?.();
  if (clock) clearInterval(clock);
});

watch(
  () => plan.value?.id,
  async (id) => {
    if (!id) return;
    await nextTick();
    rejectButton.value?.focus();
  },
  { immediate: true },
);

/** Программа из таблицы запретов ядра → ключ перевода «что именно». */
const NEVER_WHAT: Record<string, string> = {
  mkfs: "disk_format",
  fdisk: "partition_table",
  parted: "partition_table",
  shutdown: "power_off",
  poweroff: "power_off",
  reboot: "reboot",
  halt: "halt",
  init: "runlevel",
  visudo: "sudo_rules",
};

/**
 * Причина запрета на языке интерфейса. Ядро присылает код и аргумент, а
 * русский текст оставляет запасным — для кода, которого здесь ещё нет.
 */
function blockedText(step: PendingPlan["steps"][number]): string {
  const arg = step.blockedArg ?? "";
  switch (step.blockedCode) {
    case "protected_path":
      return t("agentPlan.blockedWhy.protected_path", { path: arg });
    case "never": {
      const what = NEVER_WHAT[arg];
      return what
        ? t("agentPlan.blockedWhy.never", { what: t(`agentPlan.blockedWhat.${what}`) })
        : (step.blockedReason ?? "");
    }
    case "device_write":
    case "delete_root":
      return t(`agentPlan.blockedWhy.${step.blockedCode}`);
    default:
      return step.blockedReason ?? "";
  }
}

function nothingWillRun(plan: PendingPlan) {
  return plan.steps.every((step) => step.kind === "blocked" || step.kind === "skipped");
}

function lineClass(kind: string): string {
  if (kind === "added") return "bg-success-soft text-success-text";
  if (kind === "removed") return "bg-danger-soft text-danger-text";
  return "text-on-surface-muted";
}

function marker(kind: string): string {
  if (kind === "added") return "+";
  if (kind === "removed") return "−";
  return " ";
}

function remaining(expiresAt: number): string {
  const seconds = Math.max(0, Math.ceil((expiresAt - now.value) / 1000));
  return `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
}

function rejectCurrent() {
  if (!plan.value || store.busy) return;
  if (isGone.value) {
    store.dismissGone(plan.value.id);
    return;
  }
  void store.answer(plan.value.id, false);
}
</script>

<template>
  <AlertDialogRoot :open="plan !== null">
    <AlertDialogPortal>
      <AlertDialogOverlay class="fixed inset-0 bg-scrim z-40" />
      <AlertDialogContent
        v-if="plan"
        data-testid="agent-plan-dialog"
        class="fixed z-50 top-1/2 left-1/2 dialog-center bg-surface-card border border-edge rounded-r12 shadow-e2 w-full max-w-3xl max-h-[90vh] p-6 flex flex-col focus:outline-none"
        @keydown.esc.prevent.stop="rejectCurrent"
      >
        <div class="flex items-start justify-between gap-4 mb-3">
          <div class="flex items-center gap-2 text-agent-text">
            <Bot :size="18" />
            <span class="text-[10px] font-mono font-bold tracking-wider">AGENT</span>
            <span class="text-xs text-on-surface-muted">
              {{ t("agentPlan.clientSelfReported", { client: plan.client, session: plan.session }) }}
            </span>
          </div>
          <span class="text-xs text-on-surface-muted shrink-0">{{ t("agentPlan.journal") }}</span>
        </div>

        <AlertDialogTitle class="text-dialog-title text-on-surface">
          {{ t("agentPlan.title", { host: plan.host }) }}
        </AlertDialogTitle>
        <p class="mt-1 font-mono text-mono text-on-surface-muted">
          {{ plan.hostUser }}@{{ plan.hostAddress }}
          · {{ t("agentPlan.steps", { count: plan.steps.length }) }}
          · {{ t("agentPlan.blockedCount", { count: plan.steps.filter((step) => step.kind === "blocked").length }) }}
          · {{ t("agentPlan.skippedCount", { count: plan.steps.filter((step) => step.kind === "skipped").length }) }}
        </p>

        <AlertDialogDescription
          class="mt-3 text-body text-on-surface"
          data-testid="agent-plan-intent"
        >
          {{ plan.intent }}
        </AlertDialogDescription>
        <p class="mt-1 text-xs text-on-surface-muted">
          {{ t("agentPlan.executionHint") }}
        </p>

        <p
          v-if="isGone"
          data-testid="agent-plan-gone"
          class="mt-3 rounded-r8 border border-warning/40 bg-warning-soft px-3 py-2 text-sm text-warning-text"
        >
          {{ t(`agentPlan.gone.${goneReason}`) }}
        </p>

        <div
          v-if="store.error"
          class="mt-3 bg-danger-soft border border-danger/40 text-danger-text text-sm rounded-r8 px-3 py-2"
        >
          {{ store.error }}
        </div>

        <ol
          data-testid="agent-plan-steps"
          class="mt-4 min-h-0 overflow-y-auto border border-edge rounded-r8 divide-y divide-edge-subtle"
        >
          <li v-for="(step, i) in plan.steps" :key="i" class="px-3 py-3">
            <div class="flex items-start gap-3">
              <span class="w-5 text-right font-mono text-mono text-on-surface-muted select-none">
                {{ i + 1 }}
              </span>
              <div class="min-w-0 flex-1">
                <div class="flex items-start justify-between gap-3">
                  <code
                    class="font-mono text-mono whitespace-pre-wrap break-words"
                    :class="
                      step.kind === 'blocked'
                        ? 'text-danger-text line-through'
                        : step.kind === 'skipped'
                          ? 'text-on-surface-muted line-through'
                          : 'text-on-surface'
                    "
                    >{{ step.title }}</code
                  >
                  <span
                    class="shrink-0 px-2 py-0.5 rounded-full border text-[9px] font-mono font-bold uppercase"
                    :class="
                      step.kind === 'blocked'
                        ? 'border-danger/40 text-danger-text'
                        : step.kind === 'skipped'
                          ? 'border-edge text-on-surface-muted'
                          : 'border-edge text-on-surface-muted'
                    "
                  >
                    {{ t(`agentPlan.kind.${step.kind}`) }}
                  </span>
                </div>
                <p v-if="step.blockedReason" class="mt-1 text-xs text-danger-text">
                  {{ blockedText(step) }}
                </p>
                <p v-if="step.kind === 'skipped'" class="mt-1 text-xs text-on-surface-muted">
                  {{ t("agentPlan.skippedReason") }}
                </p>
                <div
                  v-if="step.diff?.length"
                  data-testid="agent-plan-diff"
                  class="mt-2 max-h-64 overflow-auto text-xs font-mono bg-surface border border-edge rounded-r6"
                >
                  <div
                    v-for="(line, lineIndex) in step.diff"
                    :key="lineIndex"
                    :class="lineClass(line.kind)"
                    class="px-3 whitespace-pre-wrap break-words"
                  >
                    {{ marker(line.kind) }} {{ line.text }}
                  </div>
                </div>
              </div>
            </div>
          </li>
        </ol>

        <div class="mt-4 flex flex-wrap items-center justify-between gap-3">
          <button
            type="button"
            data-testid="agent-plan-reject-all"
            class="px-3 py-1.5 text-sm rounded-r8 text-danger-text hover:bg-danger-soft disabled:opacity-45"
            :disabled="store.busy"
            @click="store.rejectAll()"
          >
            {{ t("agentPlan.rejectAll") }}
          </button>
          <div class="flex flex-wrap gap-2">
            <button
              ref="rejectButton"
              type="button"
              data-testid="agent-plan-reject"
              class="px-4 py-2 text-sm rounded-r8 border border-edge bg-surface-alt text-on-surface hover:bg-edge-subtle disabled:opacity-45"
              :disabled="store.busy || isGone"
              @click="rejectCurrent"
            >
              {{ t("agentPlan.reject") }}
            </button>
            <button
              type="button"
              data-testid="agent-plan-approve"
              class="px-4 py-2 text-sm rounded-r8 border border-accent bg-accent text-on-accent hover:bg-accent-hover active:bg-accent-active disabled:opacity-45"
              :disabled="store.busy || nothingWillRun(plan) || isGone"
              @click="store.answer(plan.id, true)"
            >
              {{ t("agentPlan.approve") }}
            </button>
            <button
              v-if="isGone"
              type="button"
              data-testid="agent-plan-dismiss"
              class="px-4 py-2 text-sm rounded-r8 border border-edge bg-surface-alt text-on-surface hover:bg-edge-subtle"
              @click="store.dismissGone(plan.id)"
            >
              {{ t("agentPlan.dismiss") }}
            </button>
          </div>
        </div>

        <div class="mt-3 flex flex-wrap items-center justify-between gap-2 text-xs text-on-surface-muted">
          <span>{{ t("agentPlan.autoReject", { time: remaining(plan.expiresAt) }) }}</span>
          <kbd class="font-mono text-[11px] bg-surface-alt rounded-r6 px-2 py-1">
            {{ t("agentPlan.keyboardHint") }}
          </kbd>
        </div>
      </AlertDialogContent>
    </AlertDialogPortal>
  </AlertDialogRoot>
</template>
