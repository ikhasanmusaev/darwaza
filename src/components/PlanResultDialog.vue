<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { CheckCircle2, XCircle, Ban, MinusCircle } from "lucide-vue-next";
import { useAgentPlansStore, type StepOutcome } from "../stores/agent_plans";

const { t } = useI18n();
const store = useAgentPlansStore();
const result = computed(() => store.results[0] ?? null);

function dismiss() {
  if (result.value) store.dismissResult(result.value.planId);
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Escape" && result.value) {
    event.preventDefault();
    dismiss();
  }
}

onMounted(() => window.addEventListener("keydown", onKeydown));
onBeforeUnmount(() => window.removeEventListener("keydown", onKeydown));

function outcomeIcon(outcome: StepOutcome) {
  if (outcome === "done") return CheckCircle2;
  if (outcome === "failed") return XCircle;
  if (outcome === "blocked") return Ban;
  return MinusCircle;
}

function outcomeClass(outcome: StepOutcome) {
  if (outcome === "done") return "text-success-text";
  if (outcome === "failed" || outcome === "blocked") return "text-danger-text";
  return "text-on-surface-muted";
}

function outcomeLabel(outcome: StepOutcome, exitCode: number | null): string {
  if (outcome === "failed" && exitCode !== null) return t("planResult.failedWithCode", { code: exitCode });
  return t(`planResult.${outcome}`);
}
</script>

<template>
  <Teleport to="body">
    <section
      v-if="result"
      data-testid="plan-result-dialog"
      role="dialog"
      aria-modal="false"
      class="fixed bottom-6 right-6 z-[80] w-[min(36rem,calc(100vw-3rem))] max-h-[80vh] overflow-y-auto rounded-r12 border border-edge bg-surface-card shadow-e2 p-5"
    >
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="text-dialog-title" :class="result.status === 'completed' ? 'text-success-text' : 'text-warning-text'">
            {{ result.status === "completed" ? t("planResult.completedTitle", { done: result.done, total: result.total }) : t("planResult.stoppedTitle") }}
          </h2>
          <p class="mt-1 text-body text-on-surface-muted">{{ result.host }}</p>
        </div>
        <button
          type="button"
          data-testid="plan-result-dismiss"
          class="px-3 py-1.5 rounded-r8 bg-surface-alt text-on-surface hover:bg-edge-subtle"
          @click="dismiss"
        >
          {{ t("planResult.dismiss") }}
        </button>
      </div>

      <p v-if="result.reason" class="mt-3 text-body text-on-surface-muted">{{ result.reason }}</p>

      <ol class="mt-4 divide-y divide-edge-subtle border-y border-edge-subtle">
        <li v-for="(step, index) in result.steps" :key="index" class="flex items-start gap-3 py-3">
          <component :is="outcomeIcon(step.outcome)" :size="17" class="mt-0.5 shrink-0" :class="outcomeClass(step.outcome)" />
          <div class="min-w-0 flex-1">
            <code class="block font-mono text-mono text-on-surface whitespace-pre-wrap break-words">{{ step.title }}</code>
            <p class="mt-1 text-xs" :class="outcomeClass(step.outcome)">
              {{ outcomeLabel(step.outcome, step.exitCode) }}
            </p>
          </div>
        </li>
      </ol>
    </section>
  </Teleport>
</template>
