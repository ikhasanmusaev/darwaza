<script setup lang="ts">
/**
 * Где лежат ваши секреты (#171).
 *
 * Раздел отвечает на вопрос «где мой пароль» без чтения исходников. Вместе с
 * ответом — то, о чём человек знать обязан: разблокированный системный сейф
 * доступен любой программе его сессии. Это устройство десктопа, а не наша
 * уязвимость, но именно этот факт объясняет, зачем мастер-пароль нужен и там,
 * где сейф есть.
 */
import { onMounted, computed } from "vue";
import { useI18n } from "vue-i18n";
import { ShieldCheck, KeyRound, ShieldAlert } from "lucide-vue-next";
import { useMasterStore } from "../stores/master";

const { t } = useI18n();
const master = useMasterStore();

onMounted(() => {
  if (!master.place) master.refreshPlace();
});

/** Куда пойдёт следующий секрет. */
const target = computed(() => master.place?.target ?? null);
</script>

<template>
  <section class="bg-surface-card border border-edge rounded-lg p-4 mb-4">
    <h2 class="text-sm font-semibold mb-3">{{ t("secretsPlace.title") }}</h2>

    <div v-if="!master.place" class="text-sm text-muted">
      {{ t("secretsPlace.checking") }}
    </div>

    <template v-else>
      <!-- Где лежат прямо сейчас -->
      <div class="flex items-start gap-3 mb-3">
        <ShieldCheck v-if="target === 'master'" class="w-5 h-5 shrink-0 mt-0.5" />
        <KeyRound v-else-if="target === 'keyring'" class="w-5 h-5 shrink-0 mt-0.5" />
        <ShieldAlert v-else class="w-5 h-5 shrink-0 mt-0.5" />
        <div>
          <p class="text-sm font-medium">
            {{ t(`secretsPlace.where.${target}`) }}
          </p>
          <p class="text-sm text-muted mt-1">
            {{ t(`secretsPlace.whereHint.${target}`) }}
          </p>
        </div>
      </div>

      <!-- Что видно системе. Показываем только когда это правда. -->
      <p
        v-if="target === 'keyring'"
        class="text-sm text-muted border-l-2 border-edge pl-3 mb-3"
      >
        {{ t("secretsPlace.sessionNote") }}
      </p>

      <!-- Состояние самого сейфа -->
      <p class="text-xs text-muted">
        {{
          master.place.keyringAvailable
            ? t("secretsPlace.keyringFound")
            : t("secretsPlace.keyringMissing")
        }}
      </p>
    </template>
  </section>
</template>
