<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { errorText } from "../lib/errors";
import { useI18n } from "vue-i18n";
import { RefreshCw, CloudOff, Cloud, LogOut, KeyRound } from "lucide-vue-next";
import RecoveryKeyDialog from "./RecoveryKeyDialog.vue";
import { useSyncStore } from "../stores/sync";

/**
 * Аккаунт и синхронизация в настройках (#190-195).
 *
 * Раздел устроен так, чтобы **без аккаунта он не мешал**: пока адрес сервера
 * не задан, здесь одно поле и объяснение. Приложение было локальным и
 * остаётся им — регистрация не условие запуска.
 */
const { t } = useI18n();
const store = useSyncStore();

const endpoint = ref("");
const email = ref("");
const password = ref("");
const totpCode = ref("");
const mode = ref<"login" | "register">("login");
/** Пока человек не тронул поле, показываем сохранённое значение. */
const endpointTouched = ref(false);
/** Адрес сервера скрыт: он нужен лишь для своего сервера и разработки. */
const showEndpoint = ref(false);

onMounted(async () => {
  await store.refresh();
  if (!endpointTouched.value) {
    endpoint.value = store.status?.endpoint ?? "";
    email.value = store.status?.email ?? "";
  }
});

/** Что сейчас делаем с ключом восстановления: выпускаем или восстанавливаем. */
const recoveryMode = ref<"issue" | "recover" | null>(null);
const recoveryKey = ref("");
const newPassword = ref("");
/** Ключ, который надо показать человеку один раз. */
const shownKey = ref<string | null>(null);

/** Выпустить ключ восстановления. Пароль подтверждает, что это владелец. */
async function issueRecovery() {
  const key = await store.issueRecoveryKey(password.value);
  password.value = "";
  if (key) {
    recoveryMode.value = null;
    shownKey.value = key;
  }
}

/**
 * Восстановить доступ ключом с бумаги.
 *
 * Новый ключ показывается только после ответа сервера: показанный заранее
 * окажется недействительным, если запрос не дойдёт, — а человек его уже
 * переписал.
 */
async function recover() {
  const next = await store.recover(email.value, recoveryKey.value, newPassword.value);
  recoveryKey.value = "";
  newPassword.value = "";
  if (next) {
    recoveryMode.value = null;
    shownKey.value = next;
  }
}

const configured = computed(() => (store.status?.endpoint ?? "") !== "");
const signedIn = computed(() => store.status?.signedIn === true);

async function saveEndpoint() {
  await store.setEndpoint(endpoint.value);
  endpointTouched.value = false;
}

async function submit() {
  if (mode.value === "register") {
    // Ключ восстановления выпускается вместе с аккаунтом и показывается
    // сразу: второго раза не будет, а отложенный ключ не выпускают никогда.
    const key = await store.register(email.value, password.value);
    if (key) {
      password.value = "";
      shownKey.value = key;
    }
    return;
  }

  const ok = await store.login(email.value, password.value, totpCode.value);
  if (ok) {
    // Введённое не держим в памяти дольше необходимого.
    password.value = "";
    totpCode.value = "";
  }
}
</script>

<template>
  <section class="bg-surface-card border border-edge rounded-lg p-4" data-testid="sync-section">
    <div class="flex items-center gap-2 mb-2">
      <component
        :is="signedIn ? Cloud : CloudOff"
        :size="18"
        :class="signedIn ? 'text-success-text' : 'text-on-surface-muted'"
      />
      <h3 class="text-sm font-medium text-on-surface">{{ t("sync.title") }}</h3>
    </div>

    <p class="text-xs text-on-surface-muted mb-3">{{ t("sync.description") }}</p>

    <div
      v-if="store.error"
      class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2 mb-3"
      data-testid="sync-error"
    >
      {{ store.error }}
    </div>

    <!-- Адрес сервера. Обычному человеку он не нужен: подставлен наш.
         Поле остаётся ради своего сервера и разработки, но не первым, что
         видно, — раньше оно требовало заполнить то, чего человек не знает. -->
    <div class="mb-3">
      <button
        type="button"
        class="text-xs text-on-surface-muted hover:text-on-surface underline decoration-dotted"
        data-testid="sync-endpoint-toggle"
        @click="showEndpoint = !showEndpoint"
      >
        {{ showEndpoint ? t("sync.endpointHide") : t("sync.endpointShow") }}
      </button>

      <div v-if="showEndpoint" class="mt-2">
        <label class="block text-xs text-on-surface-muted mb-1">{{ t("sync.endpoint") }}</label>
        <div class="flex gap-2">
          <input
            v-model="endpoint"
            data-testid="sync-endpoint"
            placeholder="https://api.darwaza.uz/api/v1"
            class="flex-1 bg-surface border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
            @input="endpointTouched = true"
          />
          <button
            type="button"
            data-testid="sync-endpoint-save"
            class="px-3 py-1.5 text-sm rounded bg-surface-alt hover:bg-surface text-on-surface"
            @click="saveEndpoint"
          >
            {{ t("common.save") }}
          </button>
        </div>
        <p class="text-xs text-on-surface-muted mt-1">{{ t("sync.endpointHint") }}</p>
        <p
          v-if="endpointTouched && endpoint.trim() !== (store.status?.endpoint ?? '')"
          class="mt-2 text-xs text-warning-text"
          data-testid="sync-endpoint-warning"
        >
          {{ t("sync.endpointChangeWarning", { current: store.status?.endpoint ?? '' }) }}
        </p>
      </div>
    </div>

    <template v-if="configured">
      <!-- Вошли: состояние и кнопка прохода. -->
      <div v-if="signedIn" data-testid="sync-signed-in">
        <p class="text-sm text-on-surface mb-1">{{ store.status?.email }}</p>
        <p class="text-xs text-on-surface-muted mb-3">
          {{ t("sync.pending", { count: store.status?.pending ?? 0 }) }}
        </p>

        <p
          v-if="store.status?.lastError"
          class="text-xs text-warning-text mb-3"
          data-testid="sync-last-error"
        >
          {{ t("sync.lastError", { error: errorText(store.status.lastError) }) }}
        </p>

        <p
          v-if="store.lastReport"
          class="text-xs text-on-surface-muted mb-3"
          data-testid="sync-report"
        >
          {{
            t("sync.report", {
              received: store.lastReport.received,
              sent: store.lastReport.sent,
              conflicts: store.lastReport.conflicts,
            })
          }}
        </p>

        <label class="flex items-center gap-2 text-xs text-on-surface-muted mb-3">
          <input
            type="checkbox"
            data-testid="sync-auto"
            :checked="store.auto"
            class="accent-accent"
            @change="store.setAuto(($event.target as HTMLInputElement).checked)"
          />
          {{ t("sync.auto") }}
        </label>

        <div class="flex gap-2">
          <button
            type="button"
            data-testid="sync-now"
            class="px-3 py-1.5 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60 inline-flex items-center gap-1.5"
            :disabled="store.busy"
            @click="store.syncNow()"
          >
            <RefreshCw :size="14" :class="store.busy ? 'animate-spin' : ''" />
            {{ store.busy ? t("sync.working") : t("sync.now") }}
          </button>
          <button
            type="button"
            data-testid="sync-logout"
            class="px-3 py-1.5 text-sm rounded bg-surface-alt hover:bg-surface text-on-surface inline-flex items-center gap-1.5"
            :disabled="store.busy"
            @click="store.logout()"
          >
            <LogOut :size="14" />
            {{ t("sync.logout") }}
          </button>
        </div>

        <!-- Ключ восстановления аккаунта (#180c) -->
        <div class="mt-4 pt-4 border-t border-edge">
          <div class="flex items-center gap-2 mb-1">
            <KeyRound :size="16" class="text-on-surface-muted" />
            <span class="text-sm text-on-surface">{{ t("sync.recoveryTitle") }}</span>
          </div>
          <p class="text-xs text-on-surface-muted mb-2">
            {{ t("sync.recoveryHint") }}
          </p>

          <template v-if="recoveryMode === 'issue'">
            <input
              v-model="password"
              type="password"
              data-testid="sync-recovery-password"
              :placeholder="t('sync.password')"
              class="w-full bg-surface border border-edge rounded px-3 py-2 text-sm mb-2 focus:outline-none focus:border-accent"
              @keydown.enter="issueRecovery"
            />
            <p class="text-xs text-warning-text mb-2">{{ t("sync.recoveryReplaces") }}</p>
            <div class="flex gap-2">
              <button
                type="button"
                data-testid="sync-recovery-confirm"
                class="px-3 py-1.5 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60"
                :disabled="store.busy || !password"
                @click="issueRecovery"
              >
                {{ t("sync.recoveryIssue") }}
              </button>
              <button
                type="button"
                class="px-3 py-1.5 text-sm rounded bg-surface-alt hover:bg-surface text-on-surface"
                @click="
                  recoveryMode = null;
                  password = '';
                "
              >
                {{ t("form.cancel") }}
              </button>
            </div>
          </template>
          <button
            v-else
            type="button"
            data-testid="sync-recovery-start"
            class="px-3 py-1.5 text-sm rounded bg-surface-alt hover:bg-surface text-on-surface"
            @click="recoveryMode = 'issue'"
          >
            {{ t("sync.recoveryIssue") }}
          </button>
        </div>
      </div>

      <!-- Не вошли: вход или регистрация. -->
      <div v-else data-testid="sync-signed-out">
        <div class="flex gap-2 mb-3 text-xs">
          <button
            type="button"
            data-testid="sync-mode-login"
            :class="mode === 'login' ? 'text-accent' : 'text-on-surface-muted'"
            @click="mode = 'login'"
          >
            {{ t("sync.login") }}
          </button>
          <span class="text-on-surface-muted">·</span>
          <button
            type="button"
            data-testid="sync-mode-register"
            :class="mode === 'register' ? 'text-accent' : 'text-on-surface-muted'"
            @click="mode = 'register'"
          >
            {{ t("sync.register") }}
          </button>
        </div>

        <input
          v-model="email"
          type="email"
          data-testid="sync-email"
          :placeholder="t('sync.email')"
          class="w-full bg-surface border border-edge rounded px-3 py-2 text-sm mb-2 focus:outline-none focus:border-accent"
        />
        <input
          v-model="password"
          type="password"
          data-testid="sync-password"
          :placeholder="t('sync.password')"
          class="w-full bg-surface border border-edge rounded px-3 py-2 text-sm mb-2 focus:outline-none focus:border-accent"
          @keydown.enter="submit"
        />
        <input
          v-if="mode === 'login'"
          v-model="totpCode"
          data-testid="sync-totp"
          :placeholder="t('sync.totpOptional')"
          class="w-full bg-surface border border-edge rounded px-3 py-2 text-sm mb-2 focus:outline-none focus:border-accent"
          @keydown.enter="submit"
        />

        <!-- Говорим это до, а не после: восстановления пароля нет и не будет. -->
        <p
          v-if="mode === 'register'"
          class="text-xs text-warning-text mb-3"
          data-testid="sync-no-recovery"
        >
          {{ t("sync.noRecovery") }}
        </p>

        <button
          type="button"
          data-testid="sync-submit"
          class="px-3 py-1.5 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60"
          :disabled="store.busy || !email || !password"
          @click="submit"
        >
          {{ mode === "register" ? t("sync.register") : t("sync.login") }}
        </button>

        <!-- Забыт пароль: восстановление ключом с бумаги (#180c) -->
        <div v-if="mode === 'login'" class="mt-4 pt-4 border-t border-edge">
          <template v-if="recoveryMode === 'recover'">
            <p class="text-xs text-on-surface-muted mb-2">
              {{ t("sync.recoverHint") }}
            </p>
            <input
              v-model="recoveryKey"
              data-testid="sync-recovery-key"
              :placeholder="t('sync.recoveryKeyPlaceholder')"
              class="w-full bg-surface border border-edge rounded px-3 py-2 text-sm mb-2 font-mono focus:outline-none focus:border-accent"
            />
            <input
              v-model="newPassword"
              type="password"
              data-testid="sync-recovery-new-password"
              :placeholder="t('sync.newPassword')"
              class="w-full bg-surface border border-edge rounded px-3 py-2 text-sm mb-2 focus:outline-none focus:border-accent"
              @keydown.enter="recover"
            />
            <p class="text-xs text-warning-text mb-2">{{ t("sync.recoverWarning") }}</p>
            <div class="flex gap-2">
              <button
                type="button"
                data-testid="sync-recover-confirm"
                class="px-3 py-1.5 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60"
                :disabled="store.busy || !email || !recoveryKey || !newPassword"
                @click="recover"
              >
                {{ t("sync.recoverAction") }}
              </button>
              <button
                type="button"
                class="px-3 py-1.5 text-sm rounded bg-surface-alt hover:bg-surface text-on-surface"
                @click="
                  recoveryMode = null;
                  recoveryKey = '';
                  newPassword = '';
                "
              >
                {{ t("form.cancel") }}
              </button>
            </div>
          </template>
          <button
            v-else
            type="button"
            data-testid="sync-recover-start"
            class="text-xs text-on-surface-muted hover:text-on-surface"
            @click="recoveryMode = 'recover'"
          >
            {{ t("sync.forgotPassword") }}
          </button>
        </div>
      </div>
    </template>

    <RecoveryKeyDialog
      v-if="shownKey"
      :recovery-key="shownKey"
      kind="account"
      @done="shownKey = null"
    />
  </section>
</template>
