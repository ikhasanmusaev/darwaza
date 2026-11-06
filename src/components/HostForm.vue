<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { Save, Loader2, KeyRound } from "lucide-vue-next";
import { useHostsStore } from "../stores/hosts";
import { useIdentitiesStore } from "../stores/identities";
import type { HostInput } from "../api/hosts";
import { errorText } from "../lib/errors";
import ConfirmDialog from "./ConfirmDialog.vue";

const props = defineProps<{
  hostId?: number | null;
}>();

const emit = defineEmits<{
  save: [];
  cancel: [];
}>();

const { t } = useI18n();
const router = useRouter();
const store = useHostsStore();
const identitiesStore = useIdentitiesStore();

const isEdit = computed(() => props.hostId != null);

const loading = ref(false);
const saving = ref(false);

const defaultInput: HostInput = {
  name: "",
  hostname: "",
  port: 22,
  username: "",
  authMethod: "password",
  groupId: null,
  identityId: null,
  jumpHostId: null,
  forwardAgent: false,
  agentAccess: false,
  agentSudoRead: null as string | null,
  agentNotes: null as string | null,
  color: null,
  icon: null,
  description: null,
};

const input = ref<HostInput>({ ...defaultInput });

/**
 * Кандидаты в jump-хосты (#151). Себя исключаем: хост, прыгающий сам через
 * себя, даёт цикл, который backend всё равно отвергнет.
 */
const jumpHostCandidates = computed(() => store.hosts.filter((h) => h.id !== props.hostId));
const password = ref("");
const error = ref<string | null>(null);

async function loadHost() {
  if (!isEdit.value || props.hostId == null) return;
  loading.value = true;
  try {
    await store.ensureLoaded();
    const found = store.getHost(props.hostId);
    if (!found) {
      error.value = t("hosts.notFound");
      return;
    }
    input.value = {
      name: found.name,
      hostname: found.hostname,
      port: found.port,
      username: found.username,
      authMethod: found.authMethod,
      groupId: found.groupId,
      identityId: found.identityId,
      jumpHostId: found.jumpHostId,
      forwardAgent: found.forwardAgent,
      agentAccess: found.agentAccess,
      agentSudoRead: found.agentSudoRead,
      agentNotes: found.agentNotes,
      color: found.color,
      icon: found.icon,
      description: found.description,
    };
  } finally {
    loading.value = false;
  }
}

onMounted(async () => {
  await Promise.all([loadHost(), identitiesStore.ensureLoaded()]);
});
watch(() => props.hostId, loadHost);

const needsIdentity = computed(
  () => input.value.authMethod === "publickey" || input.value.authMethod === "password",
);

const filteredIdentities = computed(() => {
  if (input.value.authMethod === "publickey") {
    return identitiesStore.identities.filter((i) => i.kind === "private_key");
  }
  if (input.value.authMethod === "password") {
    return identitiesStore.identities.filter((i) => i.kind === "password");
  }
  return [];
});

const hasIdentities = computed(() => filteredIdentities.value.length > 0);

watch(
  () => [input.value.authMethod, filteredIdentities.value.length],
  () => {
    if (input.value.identityId == null) return;
    // Пока список не приехал, он пуст — и сброс «недействительного» выбора
    // стёр бы действительный. Форма грузит хост и идентификации
    // одновременно, так что открытие правки регулярно попадало в этот
    // промежуток: человек видел пустое поле там, где идентификация выбрана
    // (2026-08-28).
    if (!identitiesStore.initialized) return;
    const stillValid = filteredIdentities.value.some((i) => i.id === input.value.identityId);
    if (!stillValid) input.value.identityId = null;
  },
);

function goCreateIdentity() {
  router.push({
    path: "/identities/new",
    query: { kind: input.value.authMethod === "publickey" ? "private_key" : "password" },
  });
}

const agentConfirmOpen = ref(false);

function onAgentAccessChange(event: Event) {
  const checkbox = event.target as HTMLInputElement;
  if (checkbox.checked) {
    checkbox.checked = false;
    agentConfirmOpen.value = true;
    return;
  }
  input.value.agentAccess = false;
}

function confirmAgentAccess() {
  input.value.agentAccess = true;
  agentConfirmOpen.value = false;
}

async function onSubmit() {
  error.value = null;

  if (!input.value.name.trim()) {
    error.value = t("hosts.validation.nameRequired");
    return;
  }
  if (!input.value.hostname.trim()) {
    error.value = t("hosts.validation.hostnameRequired");
    return;
  }
  if (!input.value.username.trim()) {
    error.value = t("hosts.validation.usernameRequired");
    return;
  }
  if (input.value.port == null || input.value.port < 1 || input.value.port > 65535) {
    error.value = t("hosts.validation.portInvalid");
    return;
  }
  if (input.value.authMethod === "publickey" && !input.value.identityId) {
    error.value = t("hosts.validation.identityRequired");
    return;
  }
  if (input.value.authMethod === "password" && !input.value.identityId && !password.value.trim()) {
    error.value = t("hosts.validation.identityOrPasswordRequired");
    return;
  }

  saving.value = true;
  try {
    const submitData: HostInput = {
      ...input.value,
      password: password.value || null,
    };
    if (isEdit.value && props.hostId != null) {
      await store.updateHost(props.hostId, submitData);
    } else {
      await store.createHost(submitData);
    }
    emit("save");
  } catch (e) {
    error.value = errorText(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div>
    <!-- Loading -->
    <div v-if="loading" class="flex items-center gap-2 text-on-surface-muted py-8">
      <Loader2 :size="18" class="animate-spin" />
      <span class="text-sm">{{ t("common.loading") }}</span>
    </div>

    <!-- Form -->
    <form v-else class="space-y-4" @submit.prevent="onSubmit">
      <!-- Error banner -->
      <div
        v-if="error"
        class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2"
      >
        {{ error }}
      </div>

      <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
        <!-- Name -->
        <div class="sm:col-span-2">
          <label class="block text-xs text-on-surface-muted mb-1">
            {{ t("hosts.form.name") }}
          </label>
          <input
            v-model="input.name"
            type="text"
            class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
            :placeholder="t('hosts.form.namePlaceholder')"
          />
        </div>

        <!-- Hostname -->
        <div>
          <label class="block text-xs text-on-surface-muted mb-1">
            {{ t("hosts.form.hostname") }}
          </label>
          <input
            v-model="input.hostname"
            type="text"
            class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
            :placeholder="t('hosts.form.hostnamePlaceholder')"
          />
        </div>

        <!-- Port -->
        <div>
          <label class="block text-xs text-on-surface-muted mb-1">
            {{ t("hosts.form.port") }}
          </label>
          <input
            v-model.number="input.port"
            type="number"
            min="1"
            max="65535"
            class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
          />
        </div>

        <!-- Username -->
        <div>
          <label class="block text-xs text-on-surface-muted mb-1">
            {{ t("hosts.form.username") }}
          </label>
          <input
            v-model="input.username"
            type="text"
            class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
            :placeholder="t('hosts.form.usernamePlaceholder')"
          />
        </div>

        <!-- Group -->
        <div>
          <label class="block text-xs text-on-surface-muted mb-1">
            {{ t("hosts.form.group") }}
          </label>
          <select
            v-model="input.groupId"
            data-testid="host-group-select"
            class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
          >
            <option :value="null">{{ t("hosts.form.noGroup") }}</option>
            <option v-for="g in store.groups" :key="g.id" :value="g.id">
              {{ g.name }}
            </option>
          </select>
        </div>

        <!-- Jump host (ProxyJump, #151) -->
        <div>
          <label class="block text-xs text-on-surface-muted mb-1">
            {{ t("hosts.form.jumpHost") }}
          </label>
          <select
            v-model="input.jumpHostId"
            data-testid="host-jump-select"
            class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
          >
            <option :value="null">{{ t("hosts.form.noJumpHost") }}</option>
            <option v-for="h in jumpHostCandidates" :key="h.id" :value="h.id">
              {{ h.name }} ({{ h.username }}@{{ h.hostname }}:{{ h.port }})
            </option>
          </select>
          <p class="mt-1 text-[11px] text-on-surface-faint">
            {{ t("hosts.form.jumpHostHint") }}
          </p>
        </div>

        <!-- Проброс ssh-agent (#150a) -->
        <div class="sm:col-span-2">
          <label class="flex items-center gap-2 text-sm text-on-surface cursor-pointer">
            <input
              v-model="input.forwardAgent"
              type="checkbox"
              data-testid="host-forward-agent"
              class="accent-accent"
            />
            {{ t("hosts.form.forwardAgent") }}
          </label>
          <p class="mt-1 text-[11px] text-warning-text/90">
            {{ t("hosts.form.forwardAgentWarning") }}
          </p>
        </div>

        <!-- Доступ агента (#201) -->
        <div class="sm:col-span-2">
          <label class="flex items-center gap-2 text-sm text-on-surface cursor-pointer">
            <input
              :checked="!!input.agentAccess"
              type="checkbox"
              data-testid="host-agent-access"
              class="accent-accent"
              @change="onAgentAccessChange"
            />
            {{ t("hosts.form.agentAccess") }}
          </label>
          <p class="mt-1 text-[11px] text-warning-text/90">
            {{ t("hosts.form.agentAccessWarning") }}
          </p>

          <!-- Читающие команды с sudo (#214) -->
          <div v-if="input.agentAccess" class="mt-3">
            <label class="block text-xs text-on-surface-muted mb-1">
              {{ t("hosts.form.agentSudoRead") }}
            </label>
            <textarea
              v-model="input.agentSudoRead"
              rows="3"
              spellcheck="false"
              data-testid="host-agent-sudo"
              class="w-full bg-surface-input border border-edge rounded px-3 py-2 text-sm font-mono focus:outline-none focus:border-accent"
              :placeholder="t('hosts.form.agentSudoReadPlaceholder')"
            />
            <p class="mt-1 text-[11px] text-warning-text/90">
              {{ t("hosts.form.agentSudoReadWarning") }}
            </p>
          </div>

          <!-- Заметки помощника (#217) -->
          <div v-if="input.agentNotes" class="mt-3" data-testid="host-agent-notes">
            <div class="flex items-center justify-between mb-1">
              <label class="block text-xs text-on-surface-muted">
                {{ t("hosts.form.agentNotes") }}
              </label>
              <button
                type="button"
                data-testid="host-agent-notes-clear"
                class="text-[11px] text-on-surface-faint hover:text-on-surface underline decoration-dotted"
                @click="input.agentNotes = ''"
              >
                {{ t("hosts.form.agentNotesClear") }}
              </button>
            </div>
            <p
              class="text-xs text-on-surface-muted whitespace-pre-wrap bg-surface border border-edge rounded px-3 py-2"
            >
              {{ input.agentNotes }}
            </p>
            <p class="mt-1 text-[11px] text-on-surface-faint">
              {{ t("hosts.form.agentNotesHint") }}
            </p>
          </div>
        </div>

        <!-- Color -->
        <div>
          <label class="block text-xs text-on-surface-muted mb-1">
            {{ t("hosts.form.color") }}
          </label>
          <div class="flex items-center gap-2">
            <input
              v-model="input.color"
              type="color"
              class="w-10 h-9 bg-transparent border-0 cursor-pointer"
            />
            <input
              v-model="input.color"
              type="text"
              class="flex-1 bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
            />
            <button
              type="button"
              class="text-xs text-on-surface-faint hover:text-on-surface-muted underline"
              @click="input.color = null"
            >
              {{ t("hosts.form.clearColor") }}
            </button>
          </div>
        </div>
      </div>

      <!-- Auth method -->
      <div>
        <label class="block text-xs text-on-surface-muted mb-2">
          {{ t("hosts.form.authMethod") }}
        </label>
        <div class="flex gap-4">
          <label class="flex items-center gap-2 text-sm text-on-surface cursor-pointer">
            <input v-model="input.authMethod" type="radio" value="password" class="accent-accent" />
            {{ t("hosts.form.authPassword") }}
          </label>
          <label class="flex items-center gap-2 text-sm text-on-surface cursor-pointer">
            <input
              v-model="input.authMethod"
              type="radio"
              value="publickey"
              class="accent-accent"
            />
            {{ t("hosts.form.authPublicKey") }}
          </label>
          <label
            class="flex items-center gap-2 text-sm text-on-surface cursor-pointer"
            :title="t('hosts.form.authAgentHint')"
          >
            <input v-model="input.authMethod" type="radio" value="agent" class="accent-accent" />
            {{ t("hosts.form.authAgent") }}
          </label>
        </div>
      </div>

      <!-- Inline password (shown when password auth selected and no identity chosen) -->
      <div v-if="input.authMethod === 'password' && !input.identityId" class="sm:col-span-2">
        <label class="block text-xs text-on-surface-muted mb-1">
          {{ t("hosts.form.password") }}
        </label>
        <input
          v-model="password"
          type="password"
          class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
          :placeholder="t('hosts.form.passwordPlaceholder')"
        />
        <p class="text-xs text-on-surface-faint mt-1">
          {{ t("hosts.form.passwordHint") }}
        </p>
      </div>

      <!-- Identity (для password и publickey) -->
      <div v-if="needsIdentity">
        <label class="block text-xs text-on-surface-muted mb-1">
          {{ t("hosts.form.identity") }}
        </label>
        <select
          v-if="hasIdentities"
          v-model="input.identityId"
          data-testid="host-identity-select"
          class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent"
        >
          <option :value="null">{{ t("hosts.form.selectIdentity") }}</option>
          <option v-for="i in filteredIdentities" :key="i.id" :value="i.id">
            {{ i.label }}
          </option>
        </select>

        <div
          v-else
          class="flex items-center justify-between gap-3 bg-surface-card border border-dashed border-edge rounded px-3 py-2 text-sm"
        >
          <span class="text-on-surface-faint">{{ t("hosts.form.noIdentities") }}</span>
          <button
            type="button"
            class="inline-flex items-center gap-1 text-accent-text hover:text-accent-hover"
            @click="goCreateIdentity"
          >
            <KeyRound :size="14" />
            {{ t("hosts.form.createIdentity") }}
          </button>
        </div>
      </div>

      <!-- Description -->
      <div>
        <label class="block text-xs text-on-surface-muted mb-1">
          {{ t("hosts.form.description") }}
        </label>
        <textarea
          v-model="input.description"
          rows="3"
          class="w-full bg-surface-card border border-edge rounded px-3 py-2 text-sm focus:outline-none focus:border-accent resize-none"
          :placeholder="t('hosts.form.descriptionPlaceholder')"
        />
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
          class="px-4 py-2 text-sm rounded bg-surface-alt hover:bg-surface-card"
          @click="$emit('cancel')"
        >
          {{ t("form.cancel") }}
        </button>
      </div>
    </form>

    <ConfirmDialog
      :open="agentConfirmOpen"
      :title="t('hosts.agentConfirmTitle', { name: input.name })"
      :description="t('hosts.form.agentAccessWarning')"
      :confirm-label="t('hosts.agentEnable')"
      :cancel-label="t('form.cancel')"
      @update:open="agentConfirmOpen = $event"
      @confirm="confirmAgentAccess"
      @cancel="agentConfirmOpen = false"
    />
  </div>
</template>
