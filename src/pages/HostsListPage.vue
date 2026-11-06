<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import {
  Plus,
  Server,
  Pencil,
  Trash2,
  LogIn,
  Search,
  Folder,
  Loader2,
  Download,
  Clock,
  MoreHorizontal,
} from "lucide-vue-next";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import SshImportModal from "../components/SshImportModal.vue";
import GroupManageModal from "../components/GroupManageModal.vue";
import HostFormPanel from "../components/HostFormPanel.vue";
import ConnectionLogsPanel from "../components/ConnectionLogsPanel.vue";
import { useHostsStore } from "../stores/hosts";
import { useIdentitiesStore } from "../stores/identities";
import { useSessionsStore } from "../stores/sessions";
import { hostsApi, type Host, type HostInput, type Reachability } from "../api/hosts";
import { errorText } from "../lib/errors";

const { t } = useI18n();
const store = useHostsStore();
const identitiesStore = useIdentitiesStore();
const sessionsStore = useSessionsStore();
const connectError = ref<string | null>(null);
const agentError = ref<string | null>(null);

const hostToDelete = ref<Host | null>(null);
const agentHost = ref<Host | null>(null);
const importModalOpen = ref(false);
const groupManageOpen = ref(false);
const searchQuery = ref("");
const selectedGroupId = ref<number | "all">("all");
const panelOpen = ref(false);
const panelHostId = ref<number | null>(null);
const logsPanelOpen = ref(false);
const logsHostId = ref<number | null>(null);
const logsHostName = ref("");
const probeStates = ref<Record<number, Reachability>>({});

onMounted(async () => {
  await Promise.all([store.ensureLoaded(), identitiesStore.ensureLoaded()]);
  if (store.hosts.length === 0) return;
  try {
    const result = await hostsApi.probe(store.hosts.map((host) => host.id));
    probeStates.value = Object.fromEntries(
      result.map((item) => [item.hostId, item.state]),
    ) as Record<number, Reachability>;
  } catch {
    probeStates.value = {};
  }
});

const filteredHosts = computed(() => {
  let result = store.hosts;

  if (selectedGroupId.value !== "all") {
    result = result.filter((h) => h.groupId === selectedGroupId.value);
  }

  const q = searchQuery.value.trim().toLowerCase();
  if (q) {
    result = result.filter(
      (h) =>
        h.name.toLowerCase().includes(q) ||
        h.hostname.toLowerCase().includes(q) ||
        h.username.toLowerCase().includes(q),
    );
  }

  return result;
});

function editHost(id: number) {
  panelHostId.value = id;
  panelOpen.value = true;
}

function addHost() {
  panelHostId.value = null;
  panelOpen.value = true;
}

function openLogs(host: Host) {
  logsHostId.value = host.id;
  logsHostName.value = host.name;
  logsPanelOpen.value = true;
}

function identityName(id: number | null): string {
  return id === null ? "—" : (identitiesStore.getById(id)?.label ?? "—");
}

function probeState(hostId: number): Reachability {
  return probeStates.value[hostId] ?? "unknown";
}

function statusDotClass(hostId: number): string {
  const state = probeState(hostId);
  if (state === "reachable") return "bg-success";
  if (state === "unreachable") return "bg-on-surface-faint";
  return "bg-transparent border border-edge";
}

async function connectHost(id: number) {
  connectError.value = null;
  const h = store.hosts.find((x) => x.id === id);
  if (!h) return;
  try {
    await sessionsStore.open(id, h.name);
    // setActive уже произошёл внутри open() — Layout переключится автоматически.
  } catch (e) {
    connectError.value = errorText(e);
  }
}

function deleteHost(id: number) {
  const h = store.hosts.find((x) => x.id === id) ?? null;
  hostToDelete.value = h;
}

async function onConfirmDelete() {
  if (!hostToDelete.value) return;
  try {
    await store.removeHost(hostToDelete.value.id);
  } finally {
    hostToDelete.value = null;
  }
}

function onCancelDelete() {
  hostToDelete.value = null;
}

async function updateAgentAccess(host: Host, enabled: boolean) {
  agentError.value = null;
  const input: HostInput = {
    name: host.name,
    hostname: host.hostname,
    port: host.port,
    username: host.username,
    authMethod: host.authMethod,
    groupId: host.groupId,
    identityId: host.identityId,
    jumpHostId: host.jumpHostId,
    forwardAgent: host.forwardAgent,
    agentAccess: enabled,
    agentSudoRead: host.agentSudoRead,
    agentNotes: host.agentNotes,
    keepAliveInterval: host.keepAliveInterval,
    color: host.color,
    icon: host.icon,
    description: host.description,
  };
  try {
    await store.updateHost(host.id, input);
  } catch (e) {
    agentError.value = errorText(e);
  }
}

function onAgentAccessChange(host: Host, event: Event) {
  const checkbox = event.target as HTMLInputElement;
  if (checkbox.checked) {
    checkbox.checked = false;
    agentHost.value = host;
    return;
  }
  void updateAgentAccess(host, false).finally(() => {
    checkbox.checked = store.getHost(host.id)?.agentAccess ?? host.agentAccess;
  });
}

async function confirmAgentAccess() {
  const host = agentHost.value;
  agentHost.value = null;
  if (host) await updateAgentAccess(host, true);
}

/**
 * Двойной клик и Enter на строке подключают к хосту (как кнопка «войти»).
 * Одиночный клик ничего не подключает: случайное подключение хуже лишнего клика.
 * Флажок AGENT, кнопки и меню «…» работают сами — их нажатия сюда не засчитываются.
 */
function isInteractive(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest("input, button, a, select, [role=menu], [role=menuitem]") !== null;
}
function onRowDblClick(id: number, e: MouseEvent) {
  if (isInteractive(e.target)) return;
  connectHost(id);
}
function onRowEnter(id: number, e: KeyboardEvent) {
  // Enter на вложенной кнопке — это её собственное нажатие, не строки.
  if (e.target !== e.currentTarget) return;
  connectHost(id);
}
/** Пробел на сфокусированной строке открывает правку (карандаш). */
function onRowSpace(id: number, e: KeyboardEvent) {
  // Пробел на вложенной кнопке или флажке — их собственное нажатие.
  if (e.target !== e.currentTarget) return;
  // Без этого пробел ещё и прокручивает страницу.
  e.preventDefault();
  editHost(id);
}
</script>

<template>
  <div class="p-6">
    <!-- Header -->
    <header class="flex flex-wrap items-start justify-between gap-4 mb-5">
      <div>
        <h2 class="text-title text-on-surface">{{ t("nav.hosts") }}</h2>
        <p class="mt-1 text-body text-on-surface-muted">{{ t("hosts.listDescription") }}</p>
      </div>
      <div class="flex flex-wrap items-center justify-end gap-2">
        <button
          type="button"
          class="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-r8 bg-surface-alt hover:bg-surface-card text-on-surface"
          @click="groupManageOpen = true"
        >
          <Folder :size="16" />
          {{ t("groups.manage") }}
        </button>
        <button
          type="button"
          class="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-r8 bg-surface-alt hover:bg-surface-card text-on-surface"
          @click="importModalOpen = true"
        >
          <Download :size="16" />
          {{ t("hosts.import") }}
        </button>
        <button
          type="button"
          class="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-r8 bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent"
          @click="addHost"
        >
          <Plus :size="16" />
          {{ t("hosts.add") }}
        </button>
      </div>
    </header>

    <!-- Filters -->
    <div class="flex flex-wrap items-center gap-3 mb-5">
      <div class="relative flex-1 min-w-[240px] max-w-xl">
        <Search :size="16" class="absolute left-3 top-1/2 icon-center-y text-on-surface-muted" />
        <input
          v-model="searchQuery"
          type="text"
          :placeholder="t('hosts.search')"
          class="w-full bg-surface-card border border-edge rounded-r8 pl-9 pr-3 py-2 text-sm focus:outline-none focus:border-accent"
        />
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <button
          type="button"
          class="h-7 px-3 rounded-full border text-xs"
          :class="
            selectedGroupId === 'all'
              ? 'border-accent bg-surface-alt text-on-surface'
              : 'border-edge text-on-surface-muted hover:bg-surface-alt'
          "
          @click="selectedGroupId = 'all'"
        >
          {{ t("hosts.filterAllHosts") }}
        </button>
        <button
          v-for="group in store.groups"
          :key="group.id"
          type="button"
          class="h-7 px-3 rounded-full border text-xs"
          :class="
            selectedGroupId === group.id
              ? 'border-accent bg-surface-alt text-on-surface'
              : 'border-edge text-on-surface-muted hover:bg-surface-alt'
          "
          @click="selectedGroupId = group.id"
        >
          {{ group.name }}
        </button>
      </div>

      <span class="ml-auto text-xs text-on-surface-muted">
        {{ t("hosts.count", { count: filteredHosts.length }) }}
      </span>
    </div>

    <!-- Connect error (non-blocking, shows above list) -->
    <div
      v-if="connectError"
      class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2 mb-4"
    >
      {{ connectError }}
    </div>

    <!-- Loading -->
    <div
      v-if="store.loading"
      class="flex items-center justify-center gap-2 py-16 text-on-surface-muted"
    >
      <Loader2 :size="18" class="animate-spin" />
      <span class="text-sm">{{ t("common.loading") }}</span>
    </div>

    <!-- Error -->
    <div
      v-else-if="store.error"
      class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded px-3 py-2 mb-4"
    >
      {{ store.error }}
    </div>

    <!-- Empty state -->
    <div
      v-else-if="filteredHosts.length === 0"
      data-testid="hosts-empty"
      class="flex flex-col items-center justify-center py-16 text-on-surface-muted"
    >
      <Server :size="40" class="mb-3 opacity-40" />
      <p class="text-sm">{{ t("hosts.empty") }}</p>
    </div>

    <div v-else>
      <div
        v-if="agentError"
        class="bg-danger-soft border border-danger/40 text-danger-text text-sm rounded-r8 px-3 py-2 mb-4"
      >
        {{ agentError }}
      </div>

      <div class="overflow-x-auto">
        <table class="w-full min-w-[900px] border-collapse" data-testid="hosts-table">
          <thead>
            <tr class="border-b border-edge-subtle text-left">
              <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">
                {{ t("hosts.table.host") }}
              </th>
              <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">
                {{ t("hosts.table.group") }}
              </th>
              <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">
                {{ t("hosts.table.identity") }}
              </th>
              <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">AGENT</th>
              <th class="px-3 py-2 text-overline uppercase text-on-surface-muted">
                {{ t("hosts.table.status") }}
              </th>
              <th class="px-3 py-2 text-overline uppercase text-on-surface-muted text-right">
                {{ t("palette.section.actions") }}
              </th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="h in filteredHosts"
              :key="h.id"
              data-testid="host-row"
              tabindex="0"
              :aria-description="t('hosts.rowHint')"
              :title="t('hosts.rowHint')"
              class="h-14 border-b border-edge-subtle hover:bg-surface-alt focus:bg-surface-alt focus:outline-none focus-visible:ring-1 focus-visible:ring-accent cursor-pointer"
              @dblclick="onRowDblClick(h.id, $event)"
              @keydown.enter="onRowEnter(h.id, $event)"
              @keydown.space="onRowSpace(h.id, $event)"
            >
              <td class="px-3 py-3">
                <div class="flex items-center gap-3 min-w-0">
                  <span
                    class="w-9 h-9 rounded-r8 bg-surface-alt flex items-center justify-center shrink-0 text-on-surface-muted"
                  >
                    <Server :size="17" />
                  </span>
                  <div class="min-w-0">
                    <div class="text-body font-semibold text-on-surface break-words">{{ h.name }}</div>
                    <div class="font-mono text-mono text-on-surface-muted break-all">
                      {{ h.username }}@{{ h.hostname }}
                    </div>
                  </div>
                </div>
              </td>
              <td class="px-3 py-3">
                <span
                  v-if="store.groupName(h.groupId)"
                  class="inline-flex px-2.5 py-0.5 rounded-full border border-warning/50 text-xs text-warning-text"
                >
                  {{ store.groupName(h.groupId) }}
                </span>
                <span v-else class="text-on-surface-muted">—</span>
              </td>
              <td class="px-3 py-3 font-mono text-mono text-on-surface-muted break-words">
                {{ identityName(h.identityId) }}
              </td>
              <td class="px-3 py-3">
                <input
                  type="checkbox"
                  :checked="h.agentAccess"
                  :aria-label="t('hosts.form.agentAccess')"
                  data-testid="host-agent-access-row"
                  class="w-4 h-4 accent-accent cursor-pointer"
                  @change="onAgentAccessChange(h, $event)"
                />
              </td>
              <td class="px-3 py-3">
                <div class="flex items-center gap-2 text-xs text-on-surface-muted">
                  <span class="w-2 h-2 rounded-full shrink-0" :class="statusDotClass(h.id)" />
                  <span>{{ t(`hosts.status.${probeState(h.id)}`) }}</span>
                </div>
              </td>
              <td class="px-3 py-3">
                <div class="flex items-center justify-end gap-1">
                  <button
                    type="button"
                    data-testid="host-connect"
                    class="inline-flex items-center justify-center w-8 h-8 rounded-r8 text-accent-text hover:bg-surface-card"
                    :title="t('hosts.connect')"
                    @click="connectHost(h.id)"
                  >
                    <LogIn :size="17" />
                  </button>
                  <button
                    type="button"
                    class="inline-flex items-center justify-center w-8 h-8 rounded-r8 text-on-surface-muted hover:bg-surface-card hover:text-on-surface"
                    :title="t('hosts.edit')"
                    @click="editHost(h.id)"
                  >
                    <Pencil :size="16" />
                  </button>
                  <details class="relative">
                    <summary
                      class="list-none inline-flex items-center justify-center w-8 h-8 rounded-r8 text-on-surface-muted hover:bg-surface-card hover:text-on-surface cursor-pointer"
                      :title="t('palette.section.actions')"
                      :aria-label="t('palette.section.actions')"
                      data-testid="host-actions-menu"
                    >
                      <MoreHorizontal :size="17" />
                    </summary>
                    <div
                      class="absolute right-0 z-10 mt-1 w-52 rounded-r8 border border-edge bg-surface-card shadow-e2 p-1"
                    >
                      <button
                        type="button"
                        class="w-full flex items-center gap-2 px-3 py-2 rounded-r6 text-left text-xs text-on-surface-muted hover:bg-surface-alt hover:text-on-surface"
                        @click="openLogs(h)"
                      >
                        <Clock :size="14" />
                        {{ t("connectionLogs.title") }}
                      </button>
                      <button
                        type="button"
                        class="w-full flex items-center gap-2 px-3 py-2 rounded-r6 text-left text-xs text-danger-text hover:bg-danger-soft"
                        :title="t('hosts.delete')"
                        @click="deleteHost(h.id)"
                      >
                        <Trash2 :size="14" />
                        {{ t("hosts.delete") }}
                      </button>
                    </div>
                  </details>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <p class="mt-5 text-xs text-on-surface-muted">{{ t("hosts.agentListHint") }}</p>
    </div>

    <HostFormPanel v-model:open="panelOpen" :host-id="panelHostId" @saved="store.fetchAll()" />

    <ConnectionLogsPanel
      v-model:open="logsPanelOpen"
      :host-id="logsHostId ?? 0"
      :host-name="logsHostName"
    />

    <SshImportModal v-model:open="importModalOpen" @imported="store.fetchAll()" />

    <GroupManageModal v-model:open="groupManageOpen" />

    <ConfirmDialog
      :open="agentHost !== null"
      :title="t('hosts.agentConfirmTitle', { name: agentHost?.name ?? '' })"
      :description="t('hosts.form.agentAccessWarning')"
      :confirm-label="t('hosts.agentEnable')"
      :cancel-label="t('form.cancel')"
      @update:open="
        (v) => {
          if (!v) agentHost = null;
        }
      "
      @confirm="confirmAgentAccess"
      @cancel="agentHost = null"
    />

    <ConfirmDialog
      :open="hostToDelete !== null"
      :title="t('dialog.deleteTitle')"
      :description="
        hostToDelete
          ? `${hostToDelete.name} — ${t('dialog.deleteDescription')}`
          : t('dialog.deleteDescription')
      "
      :confirm-label="t('form.delete')"
      :cancel-label="t('form.cancel')"
      variant="danger"
      @update:open="
        (v) => {
          if (!v) hostToDelete = null;
        }
      "
      @confirm="onConfirmDelete"
      @cancel="onCancelDelete"
    />
  </div>
</template>
