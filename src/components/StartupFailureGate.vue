<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { AlertTriangle } from "lucide-vue-next";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getStartupFailure, listenStartupFailure, type StartupFailure } from "../api/startup";

/**
 * Экран отказа на старте (#168).
 *
 * Перекрывает приложение целиком: без базы нет ни хостов, ни ключей, и любая
 * страница всё равно покажет ошибку — но такую, по которой причину понять
 * нельзя. Лучше один честный экран, чем работающее на вид окно.
 *
 * Слой выше экрана мастер-пароля: спрашивать пароль, чтобы открыть хранилище,
 * которое всё равно не откроется, — издевательство.
 */
const { t } = useI18n();
const failure = ref<StartupFailure | null>(null);
let unlisten: UnlistenFn | null = null;

onMounted(async () => {
  // Порядок важен: сначала подписка, потом запрос. Наоборот — отказ,
  // случившийся между двумя вызовами, потерялся бы.
  try {
    unlisten = await listenStartupFailure((f) => {
      failure.value ??= f;
    });
    const known = await getStartupFailure();
    if (known) failure.value ??= known;
  } catch (e) {
    // Не смогли спросить про отказ — это ещё не отказ. Закрывать приложение
    // экраном на основании собственной неудачи нельзя: так человек лишится
    // работающей программы из-за пустяка.
    console.error("не удалось узнать состояние старта", e);
  }
});

onUnmounted(() => unlisten?.());
</script>

<template>
  <div
    v-if="failure"
    data-testid="startup-failure-gate"
    class="fixed inset-0 z-[200] bg-surface flex items-center justify-center p-6"
  >
    <div class="w-full max-w-xl">
      <div class="flex items-center gap-2 mb-2 text-danger">
        <AlertTriangle :size="20" />
        <h1 class="text-base font-semibold">{{ t("startup.failedTitle") }}</h1>
      </div>

      <p class="text-sm text-on-surface-muted mb-4">{{ t("startup.failedBody") }}</p>

      <div class="mb-3">
        <div class="text-xs uppercase tracking-wide text-on-surface-muted mb-1">
          {{ t("startup.failedReason") }}
        </div>
        <pre
          data-testid="startup-failure-message"
          class="text-xs whitespace-pre-wrap break-words bg-surface-alt rounded p-3 text-on-surface"
          >{{ failure.message }}</pre
        >
      </div>

      <div class="mb-4">
        <div class="text-xs uppercase tracking-wide text-on-surface-muted mb-1">
          {{ t("startup.failedLog") }}
        </div>
        <code class="text-xs text-on-surface break-all">{{ failure.logPath }}</code>
      </div>

      <p class="text-xs text-on-surface-muted">{{ t("startup.failedHint") }}</p>
    </div>
  </div>
</template>
