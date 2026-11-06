<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { Loader2 } from "lucide-vue-next";
import { useMasterStore } from "../stores/master";
import { errorText } from "../lib/errors";
import RecoveryKeyDialog from "./RecoveryKeyDialog.vue";
import GateMark from "./GateMark.vue";

/**
 * Экран разблокировки при старте (#112).
 *
 * Перекрывает приложение целиком, пока мастер-пароль не введён: без ключа
 * секреты недоступны, и любое подключение всё равно упало бы с невнятной
 * ошибкой в середине процесса.
 */
const { t } = useI18n();
const store = useMasterStore();

/**
 * Коды, при которых пароль верен, а споткнулись на втором факторе.
 *
 * Стирать пароль в этом случае нельзя: человек набирал бы его заново из-за
 * опечатки в шести цифрах. Раньше это различалось по метке `totp:` в тексте
 * ошибки — теперь backend отдаёт код (#177), и разбирать текст не нужно.
 */
const CODE_PROBLEMS = ["errors.masterTotpRequired", "errors.masterTotpWrong"];

const password = ref("");
/** Код второго фактора — показываем поле, только если он включён (#153b). */
const code = ref("");
const busy = ref(false);
const error = ref<string | null>(null);

onMounted(() => store.refresh());

/* ---- Восстановление ключом (#180b) ---- */

/** Человек нажал «Забыли пароль?». */
const recovering = ref(false);
const recoveryKey = ref("");
const newPassword = ref("");
const newPasswordRepeat = ref("");
/** Новый ключ взамен использованного — показывается один раз. */
const replacementKey = ref<string | null>(null);

const canRecover = computed(
  () =>
    recoveryKey.value.trim().length > 0 &&
    newPassword.value.length >= store.status.minLength &&
    newPassword.value === newPasswordRepeat.value,
);

async function recover() {
  if (!canRecover.value || busy.value) return;
  busy.value = true;
  error.value = null;
  try {
    // Хранилище после этого открыто, но экран не уходит сам: он исчезает,
    // только когда человек закрыл окно с новым ключом. Иначе ключ пропал бы
    // с экрана раньше, чем его записали.
    replacementKey.value = await store.recover(recoveryKey.value, newPassword.value);
    recoveryKey.value = "";
  } catch (e) {
    error.value = errorText(e);
  } finally {
    busy.value = false;
    newPassword.value = "";
    newPasswordRepeat.value = "";
  }
}

function backToPassword() {
  recovering.value = false;
  error.value = null;
  recoveryKey.value = "";
  newPassword.value = "";
  newPasswordRepeat.value = "";
}

async function submit() {
  if (!password.value || busy.value) return;
  busy.value = true;
  error.value = null;
  try {
    await store.unlock(password.value, code.value || undefined);
    // Пароль в переменной не держим дольше необходимого.
    password.value = "";
    code.value = "";
  } catch (e) {
    const raw = e instanceof Error ? e.message : String(e);
    const codeProblem = CODE_PROBLEMS.includes(raw);
    error.value = errorText(e);
    if (!codeProblem) password.value = "";
    code.value = "";
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <div
    v-if="store.needsUnlock() || replacementKey"
    data-testid="master-unlock-gate"
    class="fixed inset-0 z-[100] bg-surface flex items-center justify-center p-6"
  >
    <div class="w-full max-w-sm bg-surface-card border border-edge rounded-r12 shadow-e2 p-6">
      <div class="flex items-center gap-2 mb-2 text-on-surface">
        <GateMark :size="24" class="text-accent" />
        <h1 class="text-dialog-title">{{ t("master.unlockTitle") }}</h1>
      </div>
      <p class="text-xs text-on-surface-muted mb-4">
        {{ t("master.unlockDescription") }}
      </p>

      <div
        v-if="error"
        class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2 mb-3"
      >
        {{ error }}
      </div>

      <!-- Восстановление ключом (#180b) -->
      <template v-if="recovering">
        <p class="text-xs text-on-surface-muted mb-3">{{ t("recovery.gateHint") }}</p>
        <textarea
          v-model="recoveryKey"
          rows="2"
          spellcheck="false"
          autocomplete="off"
          data-testid="recovery-input"
          class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm font-mono mb-3 focus:outline-none focus:border-accent"
          :placeholder="t('recovery.keyPlaceholder')"
        />
        <input
          v-model="newPassword"
          type="password"
          data-testid="recovery-new-password"
          class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm mb-3 focus:outline-none focus:border-accent"
          :placeholder="t('master.newPassword')"
        />
        <input
          v-model="newPasswordRepeat"
          type="password"
          data-testid="recovery-new-password-repeat"
          class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm mb-3 focus:outline-none focus:border-accent"
          :placeholder="t('master.repeatPassword')"
          @keydown.enter="recover"
        />
        <p class="text-xs text-on-surface-muted mb-3">{{ t("master.passwordRule", { count: store.status.minLength }) }}</p>
        <p
          v-if="newPasswordRepeat && newPassword !== newPasswordRepeat"
          class="text-xs text-warning-text mb-3"
        >
          {{ t("master.mismatch") }}
        </p>
        <button
          type="button"
          data-testid="recovery-submit"
          class="w-full inline-flex items-center justify-center gap-2 px-3 py-2 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60 mb-2"
          :disabled="!canRecover || busy"
          @click="recover"
        >
          <Loader2 v-if="busy" :size="16" class="animate-spin" />
          {{ t("recovery.submit") }}
        </button>
        <button
          type="button"
          class="w-full text-xs text-on-surface-muted hover:text-on-surface"
          @click="backToPassword"
        >
          {{ t("recovery.back") }}
        </button>
      </template>

      <template v-else>
        <input
          v-model="password"
          type="password"
          autofocus
          data-testid="master-unlock-input"
          class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm mb-3 focus:outline-none focus:border-accent"
          :placeholder="t('master.passwordPlaceholder')"
          @keydown.enter="submit"
        />

        <input
          v-if="store.status.totpEnabled"
          v-model="code"
          type="text"
          inputmode="numeric"
          autocomplete="one-time-code"
          data-testid="master-unlock-code"
          class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm mb-3 focus:outline-none focus:border-accent"
          :placeholder="t('master.codePlaceholder')"
          @keydown.enter="submit"
        />

        <button
          type="button"
          class="w-full inline-flex items-center justify-center gap-2 px-3 py-2 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60"
          :disabled="!password || busy"
          @click="submit"
        >
          <Loader2 v-if="busy" :size="16" class="animate-spin" />
          {{ busy ? t("master.unlocking") : t("master.unlock") }}
        </button>

        <button
          type="button"
          data-testid="recovery-start"
          class="w-full mt-2 text-xs text-on-surface-muted hover:text-on-surface"
          @click="
            recovering = true;
            error = null;
          "
        >
          {{ t("recovery.forgot") }}
        </button>
      </template>

      <p v-if="store.status.lockoutSecondsLeft > 0" class="mt-3 text-xs text-warning-text">
        {{ t("master.lockout", { seconds: store.status.lockoutSecondsLeft }) }}
      </p>
    </div>

    <RecoveryKeyDialog
      v-if="replacementKey"
      :recovery-key="replacementKey"
      @done="
        replacementKey = null;
        backToPassword();
      "
    />
  </div>
</template>
