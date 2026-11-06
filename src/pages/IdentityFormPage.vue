<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter, useRoute } from "vue-router";
import { ArrowLeft, Save, Loader2 } from "lucide-vue-next";
import { useIdentitiesStore } from "../stores/identities";
import { useMasterStore } from "../stores/master";
import type { IdentityInput } from "../api/identities";
import { errorText } from "../lib/errors";

const props = defineProps<{
  id?: string;
}>();

const { t } = useI18n();
const router = useRouter();
const route = useRoute();
const store = useIdentitiesStore();
const master = useMasterStore();

const isEdit = computed(() => !!props.id);
const identityId = computed(() => (props.id ? Number(props.id) : null));

const label = ref("");
const initialKind = (route.query.kind as string) === "private_key" ? "private_key" : "password";
const kind = ref<"password" | "private_key" | "agent">(initialKind);
const keyPath = ref("");
const hasPassphrase = ref(false);
const password = ref("");
const passphrase = ref("");

const loading = ref(false);
const saving = ref(false);
const error = ref<string | null>(null);

async function loadIdentity() {
  if (!isEdit.value || identityId.value == null) return;
  loading.value = true;
  try {
    await store.ensureLoaded();
    const found = store.getById(identityId.value);
    if (!found) {
      error.value = t("identities.notFound");
      return;
    }
    label.value = found.label;
    kind.value = found.kind;
    keyPath.value = found.keyPath ?? "";
    hasPassphrase.value = found.hasPassphrase;
  } catch (e) {
    error.value = errorText(e);
  } finally {
    loading.value = false;
  }
}

/**
 * Спрашиваем про хранилище при открытии формы (#169).
 *
 * Раньше человек узнавал, что сохранять секрет некуда, уже после «Сохранить» —
 * технической строкой, и без единого предложения, что делать дальше.
 */
onMounted(() => {
  loadIdentity();
  master.refreshPlace();
});

/** Секрет сохранять некуда: сейфа нет, мастер-пароль выключен. */
const noStorage = computed(() => master.place?.target === "nowhere");

/** Форма вообще собирается хранить секрет? */
const wantsSecret = computed(
  () => kind.value === "password" || (kind.value === "private_key" && hasPassphrase.value),
);

/** Человек решил не сохранять секрет и вводить его при подключении (#170). */
const skipSecret = ref(false);

function goToMasterPassword() {
  router.push({ path: "/settings", query: { focus: "master" } });
}
watch(() => route.params.id, loadIdentity);

function cancel() {
  // Если пришли из другой страницы (HostFormPage передал ?return=...), вернуть туда.
  const ret = route.query.return as string | undefined;
  router.push(ret || "/identities");
}

async function onSubmit() {
  error.value = null;

  if (!label.value.trim()) {
    error.value = t("identities.validation.labelRequired");
    return;
  }

  if (kind.value === "private_key" && !keyPath.value.trim()) {
    error.value = t("identities.validation.keyPathRequired");
    return;
  }

  if (kind.value === "agent") {
    error.value = t("identities.validation.agentBlocked");
    return;
  }

  // Человек сознательно отказался сохранять секрет (#170): пароль спросим при
  // подключении, и требовать его здесь неправильно.
  if (kind.value === "password" && !isEdit.value && !skipSecret.value && !password.value) {
    error.value = t("identities.validation.passwordRequired");
    return;
  }

  if (
    kind.value === "private_key" &&
    !isEdit.value &&
    hasPassphrase.value &&
    !skipSecret.value &&
    !passphrase.value
  ) {
    error.value = t("identities.validation.passphraseRequired");
    return;
  }

  const payload: IdentityInput = {
    label: label.value.trim(),
    kind: kind.value,
    keyPath: kind.value === "private_key" ? keyPath.value.trim() : null,
    hasPassphrase: kind.value === "private_key" ? hasPassphrase.value : false,
    publicKey: null,
    keyringRef: null,
  };

  saving.value = true;
  try {
    if (isEdit.value && identityId.value != null) {
      await store.update(identityId.value, payload);
      // Обновление пароля/passphrase в edit-режиме — только если пользователь ввёл новые значения.
      if (kind.value === "password" && password.value && !skipSecret.value) {
        await store.setPassword(identityId.value, password.value);
      }
      if (
        kind.value === "private_key" &&
        hasPassphrase.value &&
        passphrase.value &&
        !skipSecret.value
      ) {
        await store.setPassphrase(identityId.value, passphrase.value);
      }
    } else {
      const created = await store.create(payload);
      if (kind.value === "password" && password.value && !skipSecret.value) {
        await store.setPassword(created.id, password.value);
      }
      if (
        kind.value === "private_key" &&
        hasPassphrase.value &&
        passphrase.value &&
        !skipSecret.value
      ) {
        await store.setPassphrase(created.id, passphrase.value);
      }
    }
    const ret = route.query.return as string | undefined;
    router.push(ret || "/identities");
  } catch (e) {
    error.value = errorText(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="p-6 max-w-2xl">
    <!-- Header -->
    <div class="flex items-center gap-3 mb-6">
      <button
        class="inline-flex items-center justify-center w-8 h-8 rounded hover:bg-surface-alt text-on-surface-muted"
        :title="t('form.cancel')"
        @click="cancel"
      >
        <ArrowLeft :size="18" />
      </button>
      <h2 class="text-lg font-semibold">
        {{ isEdit ? t("identities.editTitle") : t("identities.add") }}
      </h2>
    </div>

    <!-- Loading -->
    <div v-if="loading" class="flex items-center gap-2 text-on-surface-muted py-8">
      <Loader2 :size="18" class="animate-spin" />
      <span class="text-sm">{{ t("common.loading") }}</span>
    </div>

    <!-- Form -->
    <form v-else class="space-y-4" @submit.prevent="onSubmit">
      <div
        v-if="error"
        class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2"
      >
        {{ error }}
      </div>

      <!-- Label -->
      <div>
        <label class="block text-xs text-on-surface-muted mb-1">
          {{ t("identities.form.label") }}
        </label>
        <input
          v-model="label"
          type="text"
          class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
          :placeholder="t('identities.form.labelPlaceholder')"
        />
      </div>

      <!-- Kind -->
      <div>
        <label class="block text-xs text-on-surface-muted mb-2">
          {{ t("identities.form.kind") }}
        </label>
        <div class="flex gap-4">
          <label class="flex items-center gap-2 text-sm text-on-surface cursor-pointer">
            <input v-model="kind" type="radio" value="password" class="accent-accent" />
            {{ t("identities.kindPassword") }}
          </label>
          <label class="flex items-center gap-2 text-sm text-on-surface cursor-pointer">
            <input v-model="kind" type="radio" value="private_key" class="accent-accent" />
            {{ t("identities.kindPrivateKey") }}
          </label>
          <label
            class="flex items-center gap-2 text-sm text-on-surface-muted cursor-not-allowed"
            :title="t('identities.agentBlocked')"
          >
            <input type="radio" value="agent" disabled class="accent-accent opacity-40" />
            {{ t("identities.kindAgent") }}
            <span class="text-[10px] text-on-surface-faint bg-surface-alt px-1 rounded"
              >BLOCKED</span
            >
          </label>
        </div>
      </div>

      <!-- Сохранять секрет некуда — говорим до ввода, а не после (#169, #170) -->
      <div
        v-if="noStorage && wantsSecret"
        class="border border-edge rounded p-3 space-y-2 bg-surface-card"
      >
        <p class="text-sm font-medium">{{ t("secretsPlace.formWarning.title") }}</p>
        <p class="text-sm text-on-surface-muted">
          {{ t("secretsPlace.formWarning.body") }}
        </p>
        <div class="flex flex-wrap gap-2 pt-1">
          <button
            type="button"
            class="text-sm px-3 py-1.5 rounded border border-accent text-accent hover:bg-accent/10"
            @click="goToMasterPassword"
          >
            {{ t("secretsPlace.formWarning.enableMaster") }}
          </button>
          <button
            type="button"
            class="text-sm px-3 py-1.5 rounded border border-edge hover:bg-surface-hover"
            :class="skipSecret ? 'border-accent text-accent' : ''"
            @click="skipSecret = !skipSecret"
          >
            {{ t("secretsPlace.formWarning.skip") }}
          </button>
        </div>
        <p v-if="skipSecret" class="text-xs text-on-surface-faint">
          {{ t("secretsPlace.formWarning.skipHint") }}
        </p>
      </div>

      <!-- Password (kind=password) -->
      <div v-if="kind === 'password' && !skipSecret">
        <label class="block text-xs text-on-surface-muted mb-1">
          {{ t("identities.form.password") }}
        </label>
        <input
          v-model="password"
          type="password"
          autocomplete="new-password"
          class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
          :placeholder="
            isEdit
              ? t('identities.form.passwordEditPlaceholder')
              : t('identities.form.passwordPlaceholder')
          "
        />
        <p class="text-xs text-on-surface-faint mt-1">
          {{ t("identities.form.passwordHint") }}
        </p>
      </div>

      <!-- Private key (kind=private_key) -->
      <div v-if="kind === 'private_key'" class="space-y-3">
        <div>
          <label class="block text-xs text-on-surface-muted mb-1">
            {{ t("identities.form.keyPath") }}
          </label>
          <input
            v-model="keyPath"
            type="text"
            class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
            :placeholder="t('identities.form.keyPathPlaceholder')"
          />
        </div>

        <label class="flex items-center gap-2 text-sm text-on-surface cursor-pointer">
          <input v-model="hasPassphrase" type="checkbox" class="accent-accent" />
          {{ t("identities.form.hasPassphrase") }}
        </label>

        <div v-if="hasPassphrase">
          <label class="block text-xs text-on-surface-muted mb-1">
            {{ t("identities.form.passphrase") }}
          </label>
          <input
            v-model="passphrase"
            type="password"
            autocomplete="new-password"
            class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
            :placeholder="
              isEdit
                ? t('identities.form.passphraseEditPlaceholder')
                : t('identities.form.passphrasePlaceholder')
            "
          />
          <p class="text-xs text-on-surface-faint mt-1">
            {{ t("identities.form.passphraseHint") }}
          </p>
        </div>
      </div>

      <!-- Actions -->
      <div class="flex items-center gap-3 pt-2">
        <button
          type="submit"
          class="inline-flex items-center gap-1.5 px-4 py-2 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60 disabled:cursor-not-allowed"
          :disabled="saving"
        >
          <Save :size="16" />
          {{ saving ? t("common.saving") : t("form.save") }}
        </button>
        <button
          type="button"
          class="px-4 py-2 text-sm rounded bg-surface-alt hover:bg-surface-alt"
          @click="cancel"
        >
          {{ t("form.cancel") }}
        </button>
      </div>
    </form>
  </div>
</template>
