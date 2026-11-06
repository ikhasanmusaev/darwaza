<script setup lang="ts">
import { masterApi } from "../api/master";
import { ref, onMounted, computed } from "vue";
import { useI18n } from "vue-i18n";
import { ShieldCheck, ShieldOff, Lock, KeyRound } from "lucide-vue-next";
import { useMasterStore } from "../stores/master";
import { errorText } from "../lib/errors";
import RecoveryKeyDialog from "./RecoveryKeyDialog.vue";

/** Секция управления мастер-паролем в настройках (#111). */
const { t } = useI18n();
const store = useMasterStore();

onMounted(() => store.refresh());

type Mode = "enable" | "disable" | "change" | null;
const mode = ref<Mode>(null);
const password = ref("");
const passwordRepeat = ref("");
const newPassword = ref("");
const busy = ref(false);
const error = ref<string | null>(null);
const note = ref<string | null>(null);

const canSubmit = computed(() => {
  if (!password.value) return false;
  if (mode.value === "enable") {
    return password.value.length >= store.status.minLength && password.value === passwordRepeat.value;
  }
  if (mode.value === "change") {
    return newPassword.value.length >= store.status.minLength;
  }
  return true;
});

function reset() {
  mode.value = null;
  password.value = "";
  passwordRepeat.value = "";
  newPassword.value = "";
  error.value = null;
}

async function submit() {
  if (!canSubmit.value || busy.value) return;
  busy.value = true;
  error.value = null;
  note.value = null;
  try {
    if (mode.value === "enable") {
      const moved = await store.enable(password.value);
      note.value = t("master.enabled", { count: moved });
      // Ключ восстановления выпускаем сразу, пока человек занят именно
      // паролем: отложенный «потом выпущу» не выпускают никогда, а без ключа
      // забытый пароль означает потерю сохранённых секретов (#180b).
      shownKey.value = await store.issueRecoveryKey();
    } else if (mode.value === "disable") {
      const moved = await store.disable(password.value);
      note.value = t("master.disabled", { count: moved });
    } else if (mode.value === "change") {
      await store.changePassword(password.value, newPassword.value);
      note.value = t("master.changed");
    }
    reset();
  } catch (e) {
    error.value = errorText(e);
  } finally {
    busy.value = false;
    password.value = "";
    passwordRepeat.value = "";
    newPassword.value = "";
  }
}

/* ---- Ключ восстановления (#180b) ---- */

/** Ключ, который сейчас показывается. Показывается один раз. */
const shownKey = ref<string | null>(null);
/** Спросили подтверждение перевыпуска: старый ключ перестанет работать. */
const confirmReissue = ref(false);
const recoveryError = ref<string | null>(null);

async function issueRecoveryKey() {
  recoveryError.value = null;
  try {
    shownKey.value = await store.issueRecoveryKey();
    confirmReissue.value = false;
  } catch (e) {
    recoveryError.value = errorText(e);
  }
}

async function lockNow() {
  await store.lock();
  note.value = t("master.lockedNow");
}

/* ---- Второй фактор на вход (#153b) ---- */
const setup = ref<{ secret: string; uri: string } | null>(null);
const confirmCode = ref("");
const disableCode = ref("");
const totpError = ref<string | null>(null);

async function beginTotp() {
  totpError.value = null;
  try {
    setup.value = await masterApi.totpBegin();
  } catch (e) {
    totpError.value = errorText(e);
  }
}

async function confirmTotp() {
  if (!setup.value) return;
  totpError.value = null;
  try {
    await masterApi.totpConfirm(setup.value.secret, confirmCode.value);
    setup.value = null;
    confirmCode.value = "";
    await store.refresh();
  } catch (e) {
    totpError.value = errorText(e);
  }
}

async function disableTotp() {
  totpError.value = null;
  try {
    await masterApi.totpDisable(disableCode.value);
    disableCode.value = "";
    await store.refresh();
  } catch (e) {
    totpError.value = errorText(e);
  }
}
</script>

<template>
  <section class="bg-surface-card border border-edge rounded-lg p-4 mb-4">
    <div class="flex items-center gap-2 mb-1">
      <component
        :is="store.status.enabled ? ShieldCheck : ShieldOff"
        :size="18"
        :class="store.status.enabled ? 'text-success-text' : 'text-on-surface-muted'"
      />
      <h3 class="text-sm font-medium text-on-surface">{{ t("master.title") }}</h3>
      <span
        v-if="store.status.enabled"
        class="text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded"
        :class="
          store.status.unlocked
            ? 'bg-success-soft text-success-text'
            : 'bg-warning-soft text-warning-text'
        "
      >
        {{ store.status.unlocked ? t("master.unlocked") : t("master.locked") }}
      </span>
    </div>
    <p class="text-xs text-on-surface-faint mb-3">{{ t("master.description") }}</p>

    <div
      v-if="error"
      class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2 mb-3"
    >
      {{ error }}
    </div>
    <div
      v-else-if="note"
      class="bg-success-soft border border-success/30 text-success-text text-sm rounded px-3 py-2 mb-3"
    >
      {{ note }}
    </div>

    <!-- Форма -->
    <div v-if="mode" class="mb-3 space-y-2 max-w-sm">
      <input
        v-model="password"
        type="password"
        data-testid="master-password"
        class="w-full bg-surface border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
        :placeholder="
          mode === 'enable' ? t('master.passwordPlaceholder') : t('master.currentPassword')
        "
        @keydown.enter="submit"
      />
      <input
        v-if="mode === 'enable'"
        v-model="passwordRepeat"
        type="password"
        data-testid="master-password-repeat"
        class="w-full bg-surface border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
        :placeholder="t('master.repeatPassword')"
        @keydown.enter="submit"
      />
      <input
        v-if="mode === 'change'"
        v-model="newPassword"
        type="password"
        data-testid="master-new-password"
        class="w-full bg-surface border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
        :placeholder="t('master.newPassword')"
        @keydown.enter="submit"
      />
      <p v-if="mode === 'enable' || mode === 'change'" class="text-xs text-on-surface-muted">
        {{ t("master.passwordRule", { count: store.status.minLength }) }}
      </p>
      <p
        v-if="(mode === 'enable' || mode === 'change') && store.status.kdf.memoryKib > 0"
        class="text-xs text-on-surface-muted"
      >
        {{
          t("master.kdf", {
            memory: Math.round(store.status.kdf.memoryKib / 1024),
            iterations: store.status.kdf.iterations,
            parallelism: store.status.kdf.parallelism,
          })
        }}
      </p>
      <p
        v-if="mode === 'enable' && passwordRepeat && password !== passwordRepeat"
        class="text-xs text-warning-text"
      >
        {{ t("master.mismatch") }}
      </p>
      <p v-if="mode === 'enable'" class="text-[11px] text-warning-text/90">
        {{ t("master.forgotWarning") }}
      </p>

      <div class="flex gap-2">
        <button
          type="button"
          class="px-3 py-2 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60"
          :disabled="!canSubmit || busy"
          data-testid="master-submit"
          @click="submit"
        >
          {{ busy ? t("common.saving") : t("form.save") }}
        </button>
        <button
          type="button"
          class="px-3 py-2 text-sm rounded bg-surface-alt hover:bg-surface text-on-surface"
          @click="reset"
        >
          {{ t("form.cancel") }}
        </button>
      </div>
    </div>

    <!-- Действия -->
    <div v-else class="flex flex-wrap gap-2">
      <button
        v-if="!store.status.enabled"
        type="button"
        class="inline-flex items-center gap-1.5 px-3 py-2 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent"
        data-testid="master-enable"
        @click="mode = 'enable'"
      >
        <ShieldCheck :size="16" />
        {{ t("master.enable") }}
      </button>

      <template v-else>
        <button
          type="button"
          class="inline-flex items-center gap-1.5 px-3 py-2 text-sm rounded bg-surface-alt hover:bg-surface text-on-surface"
          data-testid="master-change"
          @click="mode = 'change'"
        >
          <KeyRound :size="16" />
          {{ t("master.change") }}
        </button>
        <button
          v-if="store.status.unlocked"
          type="button"
          class="inline-flex items-center gap-1.5 px-3 py-2 text-sm rounded bg-surface-alt hover:bg-surface text-on-surface"
          @click="lockNow"
        >
          <Lock :size="16" />
          {{ t("master.lockNow") }}
        </button>
        <button
          type="button"
          class="inline-flex items-center gap-1.5 px-3 py-2 text-sm rounded bg-surface-alt hover:bg-danger-soft text-on-surface hover:text-danger-text"
          data-testid="master-disable"
          @click="mode = 'disable'"
        >
          <ShieldOff :size="16" />
          {{ t("master.disable") }}
        </button>
      </template>
    </div>

    <!-- Ключ восстановления (#180b) -->
    <div
      v-if="store.status.enabled && store.status.unlocked"
      class="mt-4 pt-4 border-t border-edge"
      data-testid="recovery-section"
    >
      <div class="flex items-center gap-2 mb-1">
        <KeyRound :size="16" class="text-on-surface-muted" />
        <span class="text-sm text-on-surface">{{ t("recovery.title") }}</span>
        <span
          v-if="store.status.recoveryKeySet"
          class="text-[10px] px-1.5 py-0.5 rounded bg-success-soft text-success-text"
        >
          {{ t("recovery.issued") }}
        </span>
        <span v-else class="text-[10px] px-1.5 py-0.5 rounded bg-warning-soft text-warning-text">
          {{ t("recovery.notIssued") }}
        </span>
      </div>
      <p class="text-xs text-on-surface-faint mb-3">
        {{
          store.status.recoveryKeySet
            ? t("recovery.descriptionIssued")
            : t("recovery.descriptionMissing")
        }}
      </p>

      <p v-if="recoveryError" class="text-xs text-danger-text mb-2">{{ recoveryError }}</p>

      <button
        v-if="!store.status.recoveryKeySet"
        type="button"
        data-testid="recovery-issue"
        class="px-3 py-1.5 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent"
        @click="issueRecoveryKey"
      >
        {{ t("recovery.issue") }}
      </button>

      <template v-else>
        <button
          v-if="!confirmReissue"
          type="button"
          data-testid="recovery-reissue"
          class="px-3 py-1.5 text-sm rounded bg-surface-alt hover:bg-surface text-on-surface"
          @click="confirmReissue = true"
        >
          {{ t("recovery.reissue") }}
        </button>
        <div v-else class="space-y-2">
          <p class="text-xs text-warning-text">{{ t("recovery.reissueWarning") }}</p>
          <div class="flex gap-2">
            <button
              type="button"
              data-testid="recovery-reissue-confirm"
              class="px-3 py-1.5 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent"
              @click="issueRecoveryKey"
            >
              {{ t("recovery.reissueConfirm") }}
            </button>
            <button
              type="button"
              class="px-3 py-1.5 text-sm rounded bg-surface-alt hover:bg-surface text-on-surface"
              @click="confirmReissue = false"
            >
              {{ t("form.cancel") }}
            </button>
          </div>
        </div>
      </template>
    </div>

    <RecoveryKeyDialog v-if="shownKey" :recovery-key="shownKey" @done="shownKey = null" />
  </section>

  <!-- Второй фактор на вход (#153b) -->
  <div v-if="store.status.enabled && store.status.unlocked" class="mt-4 pt-4 border-t border-edge">
    <div class="flex items-center gap-2 mb-1">
      <ShieldCheck :size="16" class="text-on-surface-muted" />
      <span class="text-sm text-on-surface">{{ t("master.totpTitle") }}</span>
      <span
        v-if="store.status.totpEnabled"
        class="text-[10px] px-1.5 py-0.5 rounded bg-success-soft text-success-text"
      >
        {{ t("master.totpOn") }}
      </span>
    </div>
    <p class="text-xs text-on-surface-faint mb-3">{{ t("master.totpDescription") }}</p>

    <p v-if="totpError" class="text-xs text-danger-text mb-2">{{ totpError }}</p>

    <!-- Включение: секрет показываем, но не сохраняем, пока не сойдётся код -->
    <div v-if="!store.status.totpEnabled">
      <button
        v-if="!setup"
        type="button"
        data-testid="totp-begin"
        class="px-3 py-1.5 text-sm rounded bg-surface-alt hover:bg-surface text-on-surface"
        @click="beginTotp"
      >
        {{ t("master.totpEnable") }}
      </button>

      <div v-else class="space-y-2">
        <p class="text-xs text-on-surface-muted">{{ t("master.totpScanHint") }}</p>
        <code
          data-testid="totp-secret"
          class="block text-xs font-mono bg-surface border border-edge rounded px-2 py-1.5 break-all"
          >{{ setup.secret }}</code
        >
        <code class="block text-[11px] font-mono text-on-surface-faint break-all">{{
          setup.uri
        }}</code>

        <div class="flex gap-2">
          <input
            v-model="confirmCode"
            type="text"
            inputmode="numeric"
            data-testid="totp-confirm-code"
            class="flex-1 bg-surface border border-edge rounded px-3 py-1.5 text-sm focus:outline-none focus:border-accent"
            :placeholder="t('master.codePlaceholder')"
          />
          <button
            type="button"
            data-testid="totp-confirm"
            class="px-3 py-1.5 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60"
            :disabled="!confirmCode"
            @click="confirmTotp"
          >
            {{ t("master.totpConfirm") }}
          </button>
        </div>
        <p class="text-[11px] text-warning-text">{{ t("master.totpConfirmWarning") }}</p>
      </div>
    </div>

    <!-- Выключение: тоже по коду -->
    <div v-else class="flex gap-2">
      <input
        v-model="disableCode"
        type="text"
        inputmode="numeric"
        data-testid="totp-disable-code"
        class="flex-1 bg-surface border border-edge rounded px-3 py-1.5 text-sm focus:outline-none focus:border-accent"
        :placeholder="t('master.codePlaceholder')"
      />
      <button
        type="button"
        data-testid="totp-disable"
        class="px-3 py-1.5 text-sm rounded border border-danger/40 bg-transparent text-danger-text hover:bg-danger-soft disabled:opacity-60"
        :disabled="!disableCode"
        @click="disableTotp"
      >
        {{ t("master.totpDisable") }}
      </button>
    </div>
  </div>
</template>
