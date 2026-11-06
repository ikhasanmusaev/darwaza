<script setup lang="ts">
import { computed, ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { Plus, Key, Pencil, Trash2, Loader2, FileKey, Fingerprint } from "lucide-vue-next";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import PageHeader, { type PageFilter } from "../components/PageHeader.vue";
import { useIdentitiesStore } from "../stores/identities";
import { identitiesApi, type Identity } from "../api/identities";
import { errorText } from "../lib/errors";

const { t } = useI18n();
const router = useRouter();
const store = useIdentitiesStore();

const identityToDelete = ref<Identity | null>(null);
const usedByHosts = ref<string[]>([]);
const localError = ref<string | null>(null);
const searchQuery = ref("");
const selectedKind = ref<"all" | "private_key" | "password">("all");

onMounted(() => store.ensureLoaded());

const filters = computed<PageFilter[]>(() => [
  { id: "all", label: t("identities.filterAll") },
  { id: "private_key", label: t("identities.filterPrivateKey") },
  { id: "password", label: t("identities.filterPassword") },
]);

const filteredIdentities = computed(() => {
  const query = searchQuery.value.trim().toLowerCase();
  return store.identities.filter((identity) => {
    if (selectedKind.value !== "all" && identity.kind !== selectedKind.value) return false;
    if (!query) return true;
    return (
      identity.label.toLowerCase().includes(query) ||
      (identity.keyPath?.toLowerCase().includes(query) ?? false)
    );
  });
});

function kindIcon(kind: Identity["kind"]) {
  if (kind === "private_key") return FileKey;
  if (kind === "password") return Fingerprint;
  return Key;
}

function kindLabel(kind: Identity["kind"]) {
  if (kind === "private_key") return t("identities.kindPrivateKey");
  if (kind === "password") return t("identities.kindPassword");
  return t("identities.kindAgent");
}

function usageCount(identity: Identity): number {
  return (identity as Identity & { usedBy?: number }).usedBy ?? 0;
}

function editIdentity(id: number) {
  router.push(`/identities/${id}/edit`);
}

async function deleteIdentity(id: number) {
  const identity = store.getById(id);
  if (!identity) return;
  localError.value = null;
  try {
    usedByHosts.value = await identitiesApi.usedBy(id);
    identityToDelete.value = identity;
  } catch (e) {
    localError.value = errorText(e);
  }
}

async function onConfirmDelete() {
  if (!identityToDelete.value) return;
  try {
    await store.remove(identityToDelete.value.id);
  } catch (e) {
    localError.value = errorText(e);
  } finally {
    identityToDelete.value = null;
  }
}

function onCancelDelete() {
  identityToDelete.value = null;
}
</script>

<template>
  <div class="p-6">
    <PageHeader
      v-model:search-query="searchQuery"
      v-model:selected-filter="selectedKind"
      :title="t('identities.title')"
      :description="t('identities.manageDescription')"
      :search-placeholder="t('hosts.search')"
      :filters="filters"
      :count-label="t('identities.count', { count: filteredIdentities.length })"
    >
      <RouterLink
        to="/identities/new"
        class="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-r8 bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent"
      >
        <Plus :size="16" />
        {{ t("identities.add") }}
      </RouterLink>
    </PageHeader>

    <div
      v-if="store.loading"
      class="flex items-center justify-center gap-2 py-16 text-on-surface-muted"
    >
      <Loader2 :size="18" class="animate-spin" />
      <span class="text-sm">{{ t("common.loading") }}</span>
    </div>

    <div
      v-else-if="store.error || localError"
      class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded-r8 px-3 py-2 mb-4"
    >
      {{ store.error || localError }}
    </div>

    <div
      v-else-if="filteredIdentities.length === 0"
      class="flex flex-col items-center justify-center py-16 text-on-surface-muted"
    >
      <Key :size="40" class="mb-3 opacity-40" />
      <p class="text-sm">{{ t("identities.empty") }}</p>
    </div>

    <div v-else class="overflow-x-auto">
      <table class="w-full min-w-[760px] border-collapse" data-testid="identities-table">
        <thead>
          <tr class="border-b border-edge-subtle text-left">
            <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">
              {{ t("identities.form.label") }}
            </th>
            <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">
              {{ t("identities.kind") }}
            </th>
            <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">
              {{ t("identities.source") }}
            </th>
            <th class="px-3 py-2 text-overline uppercase text-on-surface-muted text-right">
              {{ t("palette.section.actions") }}
            </th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="identity in filteredIdentities"
            :key="identity.id"
            class="h-16 border-b border-edge-subtle hover:bg-surface-alt"
          >
            <td class="px-3 py-3">
              <div class="flex items-center gap-3 min-w-0">
                <span
                  class="w-9 h-9 rounded-r8 bg-surface-alt flex items-center justify-center text-on-surface-muted shrink-0"
                >
                  <component :is="kindIcon(identity.kind)" :size="17" />
                </span>
                <div class="min-w-0">
                  <div class="text-body font-semibold text-on-surface break-words">
                    {{ identity.label }}
                  </div>
                  <div class="text-xs text-on-surface-muted break-words">
                    <span v-if="identity.keyPath" class="font-mono">{{ identity.keyPath }}</span>
                    <span v-else-if="usageCount(identity) > 0">
                      {{ t("identities.usageCount", { count: usageCount(identity) }) }}
                    </span>
                    <span v-if="identity.hasPassphrase"> · passphrase</span>
                  </div>
                </div>
              </div>
            </td>
            <td class="px-3 py-3">
              <span
                class="inline-flex px-2 py-0.5 rounded-full text-xs"
                :class="
                  identity.kind === 'private_key'
                    ? 'bg-info/10 text-info'
                    : identity.kind === 'password'
                      ? 'bg-warning-soft text-warning-text'
                      : 'bg-surface-alt text-on-surface-muted'
                "
              >
                {{ kindLabel(identity.kind) }}
              </span>
            </td>
            <td class="px-3 py-3 text-xs text-on-surface-muted">
              {{ identity.keyPath ? t("identities.sourceFile") : t("identities.sourceKeyring") }}
            </td>
            <td class="px-3 py-3">
              <div class="flex items-center justify-end gap-1">
                <button
                  type="button"
                  class="inline-flex items-center justify-center w-8 h-8 rounded-r8 hover:bg-surface-card text-on-surface-muted hover:text-on-surface"
                  :title="t('hosts.edit')"
                  @click="editIdentity(identity.id)"
                >
                  <Pencil :size="16" />
                </button>
                <button
                  type="button"
                  class="inline-flex items-center justify-center w-8 h-8 rounded-r8 hover:bg-danger-soft text-on-surface-muted hover:text-danger-text"
                  :title="t('hosts.delete')"
                  @click="deleteIdentity(identity.id)"
                >
                  <Trash2 :size="16" />
                </button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <p class="mt-5 text-xs text-on-surface-muted">{{ t("identities.manageDescription") }}</p>

    <ConfirmDialog
      :open="identityToDelete !== null"
      :title="t('dialog.deleteTitle')"
      :description="
        identityToDelete
          ? `${identityToDelete.label} — ${t('dialog.deleteDescription')}${usedByHosts.length ? ` ${t('identities.deleteUsage', { count: usedByHosts.length, names: usedByHosts.join(', ') })}` : ''}`
          : t('dialog.deleteDescription')
      "
      :confirm-label="t('form.delete')"
      :cancel-label="t('form.cancel')"
      variant="danger"
      @update:open="
        (v) => {
          if (!v) identityToDelete = null;
        }
      "
      @confirm="onConfirmDelete"
      @cancel="onCancelDelete"
    />
  </div>
</template>
