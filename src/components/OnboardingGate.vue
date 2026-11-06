<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Cloud, Laptop } from "lucide-vue-next";
import { onboardingApi } from "../api/onboarding";
import { useMasterStore } from "../stores/master";
import { useSyncStore } from "../stores/sync";
import { errorText } from "../lib/errors";
import { setLocale, type Locale } from "../i18n";
import GateMark from "./GateMark.vue";
import RecoveryKeyDialog from "./RecoveryKeyDialog.vue";

const { t, locale } = useI18n();
const masterStore = useMasterStore();
const syncStore = useSyncStore();
const needed = ref(false);
const step = ref<"choice" | "account" | "master">("choice");
const email = ref("");
const password = ref("");
const passwordRepeat = ref("");
const shownKey = ref<string | null>(null);
const shownKind = ref<"master" | "account">("master");
const busy = ref(false);
const error = ref<string | null>(null);

onMounted(async () => {
  needed.value = await onboardingApi.needed().catch(() => false);
  if (needed.value) await masterStore.refresh();
});

function changeLanguage(event: Event) {
  setLocale((event.target as HTMLSelectElement).value as Locale);
}

async function registerAccount() {
  if (busy.value) return;
  busy.value = true;
  error.value = null;
  try {
    const key = await syncStore.register(email.value, password.value);
    password.value = "";
    if (key) {
      shownKind.value = "account";
      shownKey.value = key;
    } else {
      error.value = syncStore.error;
    }
  } catch (e) {
    error.value = errorText(e);
  } finally {
    busy.value = false;
  }
}

async function enableMaster() {
  if (busy.value || password.value.length < masterStore.status.minLength || password.value !== passwordRepeat.value) return;
  busy.value = true;
  error.value = null;
  try {
    await masterStore.enable(password.value);
    password.value = "";
    passwordRepeat.value = "";
    const key = await masterStore.issueRecoveryKey();
    shownKind.value = "master";
    shownKey.value = key;
  } catch (e) {
    error.value = errorText(e);
  } finally {
    busy.value = false;
  }
}

async function finish() {
  error.value = null;
  try {
    await onboardingApi.done();
    shownKey.value = null;
    needed.value = false;
  } catch (e) {
    error.value = errorText(e);
  }
}
</script>

<template>
  <div
    v-if="needed"
    data-testid="onboarding-gate"
    class="fixed inset-0 z-[90] bg-canvas flex items-center justify-center p-6"
  >
    <div class="w-full max-w-xl bg-surface-card border border-edge rounded-r12 shadow-e2 p-8">
      <div class="flex items-center gap-3">
        <GateMark :size="32" class="text-accent" />
        <div>
          <h1 class="text-display text-on-surface">{{ t("app.title") }}</h1>
          <p class="text-body text-on-surface-muted">{{ t("onboarding.subtitle") }}</p>
        </div>
        <select
          :value="locale"
          class="ml-auto bg-surface-input border border-edge rounded-r6 px-2 py-1 text-xs"
          :aria-label="t('settings.language')"
          @change="changeLanguage"
        >
          <option value="uz">O'zbekcha</option>
          <option value="ru">Русский</option>
          <option value="en">English</option>
          <option value="kk">Қазақша</option>
          <option value="ky">Кыргызча</option>
          <option value="tr">Türkçe</option>
        </select>
      </div>

      <p v-if="error" class="mt-4 rounded-r8 bg-danger-soft px-3 py-2 text-sm text-danger-text">
        {{ error }}
      </p>

      <div v-if="step === 'choice'" class="mt-8 grid gap-3 sm:grid-cols-2">
        <button
          type="button"
          data-testid="onboarding-local"
          class="rounded-r12 border border-edge bg-surface p-5 text-left hover:border-accent"
          @click="step = 'master'"
        >
          <Laptop :size="24" class="text-accent" />
          <span class="mt-3 block text-dialog-title text-on-surface">{{ t("onboarding.local") }}</span>
          <span class="mt-1 block text-body text-on-surface-muted">{{ t("onboarding.localHint") }}</span>
        </button>
        <button
          type="button"
          data-testid="onboarding-account"
          class="rounded-r12 border border-edge bg-surface p-5 text-left hover:border-accent"
          @click="step = 'account'"
        >
          <Cloud :size="24" class="text-accent" />
          <span class="mt-3 block text-dialog-title text-on-surface">{{ t("onboarding.account") }}</span>
          <span class="mt-1 block text-body text-on-surface-muted">{{ t("onboarding.accountHint") }}</span>
        </button>
      </div>

      <div v-else-if="step === 'account'" class="mt-8">
        <h2 class="text-dialog-title text-on-surface">{{ t("onboarding.accountPassword") }}</h2>
        <p class="mt-1 text-body text-on-surface-muted">{{ t("sync.noRecovery") }}</p>
        <div class="mt-4 space-y-3">
          <input
            v-model="email"
            type="email"
            data-testid="onboarding-email"
            class="w-full bg-surface-input border border-edge rounded-r8 px-3 py-2 text-sm"
            :placeholder="t('sync.email')"
          />
          <input
            v-model="password"
            type="password"
            data-testid="onboarding-account-password"
            class="w-full bg-surface-input border border-edge rounded-r8 px-3 py-2 text-sm"
            :placeholder="t('sync.password')"
            @keydown.enter="registerAccount"
          />
        </div>
        <div class="mt-5 flex flex-wrap gap-2">
          <button
            type="button"
            class="px-4 py-2 rounded-r8 bg-surface-alt text-on-surface"
            @click="step = 'choice'"
          >
            {{ t("onboarding.back") }}
          </button>
          <button
            type="button"
            data-testid="onboarding-register"
            class="px-4 py-2 rounded-r8 bg-accent text-on-accent disabled:opacity-45"
            :disabled="busy || !email || !password"
            @click="registerAccount"
          >
            {{ t("sync.register") }}
          </button>
        </div>
      </div>

      <div v-else class="mt-8">
        <h2 class="text-dialog-title text-on-surface">{{ t("onboarding.masterOptional") }}</h2>
        <p class="mt-1 text-body text-on-surface-muted">{{ t("master.forgotWarning") }}</p>
        <p class="mt-2 text-xs text-on-surface-muted">{{ t("master.passwordRule", { count: masterStore.status.minLength }) }}</p>
        <p v-if="masterStore.status.kdf.memoryKib > 0" class="mt-1 text-xs text-on-surface-muted">
          {{
            t("master.kdf", {
              memory: Math.round(masterStore.status.kdf.memoryKib / 1024),
              iterations: masterStore.status.kdf.iterations,
              parallelism: masterStore.status.kdf.parallelism,
            })
          }}
        </p>
        <div class="mt-4 space-y-3">
          <input
            v-model="password"
            type="password"
            data-testid="onboarding-master-password"
            class="w-full bg-surface-input border border-edge rounded-r8 px-3 py-2 text-sm"
            :placeholder="t('master.passwordPlaceholder')"
          />
          <input
            v-model="passwordRepeat"
            type="password"
            data-testid="onboarding-master-repeat"
            class="w-full bg-surface-input border border-edge rounded-r8 px-3 py-2 text-sm"
            :placeholder="t('master.repeatPassword')"
          />
          <p v-if="passwordRepeat && password !== passwordRepeat" class="text-xs text-warning-text">
            {{ t("master.mismatch") }}
          </p>
        </div>
        <div class="mt-5 flex flex-wrap gap-2">
          <button
            type="button"
            data-testid="onboarding-without-master"
            class="px-4 py-2 rounded-r8 bg-surface-alt text-on-surface"
            @click="finish"
          >
            {{ t("onboarding.withoutMaster") }}
          </button>
          <button
            type="button"
            data-testid="onboarding-master-enable"
            class="px-4 py-2 rounded-r8 bg-accent text-on-accent disabled:opacity-45"
            :disabled="busy || password.length < masterStore.status.minLength || password !== passwordRepeat"
            @click="enableMaster"
          >
            {{ t("master.enable") }}
          </button>
        </div>
      </div>
    </div>

    <RecoveryKeyDialog
      v-if="shownKey"
      :recovery-key="shownKey"
      :kind="shownKind"
      @done="finish"
    />
  </div>
</template>
