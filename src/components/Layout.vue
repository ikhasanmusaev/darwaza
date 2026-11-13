<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { computed, ref, watch, onMounted, onBeforeUnmount } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import Sidebar from "./Sidebar.vue";
import TabsBar from "./TabsBar.vue";
import TerminalPane from "./TerminalPane.vue";
import HostKeyPromptDialog from "./HostKeyPromptDialog.vue";
import CommandPalette from "./CommandPalette.vue";
import ConflictDialog from "./ConflictDialog.vue";
import SftpPanel from "./SftpPanel.vue";
import MasterUnlockGate from "./MasterUnlockGate.vue";
import StartupFailureGate from "./StartupFailureGate.vue";
import OnboardingGate from "./OnboardingGate.vue";
import AgentPlanDialog from "./AgentPlanDialog.vue";
import PlanResultDialog from "./PlanResultDialog.vue";
import AuthPromptDialog from "./AuthPromptDialog.vue";
import TransfersBar from "./TransfersBar.vue";
import { useSessionsStore } from "../stores/sessions";
import { useSftpStore } from "../stores/sftp";
import { useLayoutStore, type Divider } from "../stores/layout";
import { useTransfersStore } from "../stores/transfers";
import { useMasterStore } from "../stores/master";
import { useGlobalShortcuts } from "../lib/shortcuts";
import { subscribeAgentTerminal } from "../lib/agentTerminal";

const { t } = useI18n();
const store = useSessionsStore();
const sftp = useSftpStore();
const layout = useLayoutStore();
const transfers = useTransfersStore();
transfers.init();

// Глобальные хоткеи (Ctrl+T/W/Tab/1..9) — #054.
useGlobalShortcuts();

/**
 * Агент открывает вкладку с подготовленной командой (#216).
 *
 * Подписка живёт столько же, сколько окно: вкладку могут попросить открыть в
 * любой момент, а не только когда открыт диалог плана.
 */
let unlistenAgentTerminal: UnlistenFn | null = null;
onMounted(async () => {
  unlistenAgentTerminal = await subscribeAgentTerminal();
});
onBeforeUnmount(() => {
  unlistenAgentTerminal?.();
});

/**
 * Вернуть вкладки, которые были при прошлом запуске (#145).
 *
 * Ждём, пока мастер-пароль разблокируют: до этого секреты недоступны, и
 * каждое восстановление упёрлось бы в «пароль не сохранён». Если пароль
 * выключен, `needsUnlock()` ложен сразу и восстановление идёт при монтировании.
 */
const master = useMasterStore();
watch(
  () => master.needsUnlock(),
  (locked: boolean) => {
    if (!locked) void store.restoreOpenTabs();
  },
  { immediate: true },
);

const activeSession = computed(() =>
  store.sessions.find((s) => s.sessionId === store.activeSessionId) ?? null,
);

/**
 * Панели рисуются теми же элементами, что и обычные вкладки: меняется только
 * видимость и flex-раскладка. Так xterm не пересоздаётся при разделении —
 * компонент остаётся на своём месте в DOM.
 */
function paneVisible(sessionId: string): boolean {
  return layout.isVisible(sessionId, store.activeSessionId);
}

const stage = ref<HTMLElement | null>(null);

/** Положение панели: в обычном режиме — на весь экран, при делении — по прямоугольнику раскладки. */
function paneStyle(sessionId: string): Record<string, string> {
  const r = layout.isSplit ? layout.rects[sessionId] : undefined;
  if (!r) return { left: "0", top: "0", width: "100%", height: "100%" };
  return {
    left: `${r.x * 100}%`,
    top: `${r.y * 100}%`,
    width: `${r.w * 100}%`,
    height: `${r.h * 100}%`,
  };
}

/* ---- Бросок вкладки на панель: новая панель слева/справа/сверху/снизу ---- */

interface Zone {
  target: string;
  dir: "row" | "column";
  before: boolean;
  style: Record<string, string>;
}
const dropZone = ref<Zone | null>(null);
const dropBlocked = ref(false);
/** Край панели, в котором бросок что-то делает; центр — ничего. */
const EDGE = 0.3;

function isTabDrag(e: DragEvent): boolean {
  return Array.from(e.dataTransfer?.types ?? []).includes("sessionid");
}

function zoneAt(e: DragEvent): Zone | null {
  const el = stage.value;
  const active = store.activeSessionId;
  if (!el || !active) return null;
  const b = el.getBoundingClientRect();
  const fx = (e.clientX - b.left) / b.width;
  const fy = (e.clientY - b.top) / b.height;
  const rects = layout.isSplit ? layout.rects : { [active]: { x: 0, y: 0, w: 1, h: 1 } };
  const hit = Object.entries(rects).find(
    ([, r]) => fx >= r.x && fx <= r.x + r.w && fy >= r.y && fy <= r.y + r.h,
  );
  if (!hit) return null;
  const [target, r] = hit;
  const lx = (fx - r.x) / r.w;
  const ly = (fy - r.y) / r.h;
  const dx = Math.min(lx, 1 - lx);
  const dy = Math.min(ly, 1 - ly);
  if (Math.min(dx, dy) > EDGE) return null; // центр
  const horizontal = dx <= dy;
  const before = horizontal ? lx < 0.5 : ly < 0.5;
  const half = (v: number, len: number, first: boolean) => (first ? v : v + len / 2);
  const box = horizontal
    ? { x: half(r.x, r.w, before), y: r.y, w: r.w / 2, h: r.h }
    : { x: r.x, y: half(r.y, r.h, before), w: r.w, h: r.h / 2 };
  return {
    target,
    dir: horizontal ? "row" : "column",
    before,
    style: {
      left: `${box.x * 100}%`,
      top: `${box.y * 100}%`,
      width: `${box.w * 100}%`,
      height: `${box.h * 100}%`,
    },
  };
}

function onDragOver(e: DragEvent) {
  if (!isTabDrag(e)) return;
  e.preventDefault();
  const zone = zoneAt(e);
  const blocked = layout.isSplit ? !layout.canSplit : false;
  dropBlocked.value = zone !== null && blocked;
  dropZone.value = zone && !blocked ? zone : null;
}

function clearDrop() {
  dropZone.value = null;
  dropBlocked.value = false;
}

function onDrop(e: DragEvent) {
  const zone = dropZone.value;
  const id = e.dataTransfer?.getData("sessionId");
  clearDrop();
  if (!zone || !id || id === zone.target) return;
  e.preventDefault();
  if (layout.panes.includes(id)) layout.removePane(id);
  if (layout.splitPane(zone.target, id, zone.dir, zone.before)) store.setActive(id);
}

const GRIP = 6; // px: толщина зоны захвата разделителя

function dividerStyle(d: Divider): Record<string, string> {
  if (d.dir === "row") {
    return {
      left: `calc(${(d.box.x + d.box.w * d.ratio) * 100}% - ${GRIP / 2}px)`,
      top: `${d.box.y * 100}%`,
      width: `${GRIP}px`,
      height: `${d.box.h * 100}%`,
    };
  }
  return {
    left: `${d.box.x * 100}%`,
    top: `calc(${(d.box.y + d.box.h * d.ratio) * 100}% - ${GRIP / 2}px)`,
    width: `${d.box.w * 100}%`,
    height: `${GRIP}px`,
  };
}

function startDrag(d: Divider, down: PointerEvent) {
  const el = stage.value;
  if (!el) return;
  const bounds = el.getBoundingClientRect();
  const target = down.currentTarget as HTMLElement;
  target.setPointerCapture(down.pointerId);
  const move = (e: PointerEvent) => {
    const ratio =
      d.dir === "row"
        ? ((e.clientX - bounds.left) / bounds.width - d.box.x) / d.box.w
        : ((e.clientY - bounds.top) / bounds.height - d.box.y) / d.box.h;
    layout.setRatio(d.path, ratio);
  };
  const up = () => {
    target.removeEventListener("pointermove", move);
    target.removeEventListener("pointerup", up);
  };
  target.addEventListener("pointermove", move);
  target.addEventListener("pointerup", up);
}

// Файловый браузер имеет смысл только на живой сессии: SFTP-канал поднимается
// поверх неё же.
const sftpVisible = computed(
  () => sftp.panelOpen && activeSession.value?.status === "CONNECTED",
);
</script>

<template>
  <div class="h-full w-full flex">
    <Sidebar />
    <div class="flex-1 flex flex-col min-w-0">
      <TabsBar />
      <main class="flex-1 overflow-hidden bg-canvas flex min-h-0">
        <div
          ref="stage"
          class="flex-1 min-w-0 relative min-h-0"
          data-testid="terminal-stage"
          @dragover="onDragOver"
          @dragleave="clearDrop"
          @drop="onDrop"
        >
        <!--
          Все вкладки-сессии монтируются один раз (v-show), чтобы xterm не пересоздавался
          при переключении между ними и при разделении: меняется только положение.
          router-view используется для "Library" вкладки.
        -->
        <div
          v-for="s in store.sessions"
          v-show="paneVisible(s.sessionId)"
          :key="s.sessionId"
          :data-session-id="s.sessionId"
          class="absolute min-w-0 min-h-0"
          :class="[
            layout.isSplit ? 'border border-edge' : '',
            layout.isSplit && s.sessionId === store.activeSessionId ? 'ring-1 ring-inset ring-accent border-accent' : '',
          ]"
          :style="paneStyle(s.sessionId)"
          @mousedown="store.setActive(s.sessionId)"
        >
          <TerminalPane :session-id="s.sessionId" />
        </div>

        <!-- Разделители панелей: тянутся мышью, доля зажата в [0.15, 0.85]. -->
        <div
          v-for="d in layout.dividers"
          :key="d.path"
          data-testid="pane-divider"
          role="separator"
          :aria-orientation="d.dir === 'row' ? 'vertical' : 'horizontal'"
          class="absolute z-10 bg-transparent hover:bg-accent/40"
          :class="d.dir === 'row' ? 'cursor-col-resize' : 'cursor-row-resize'"
          :style="dividerStyle(d)"
          @pointerdown.prevent="startDrag(d, $event)"
        />

        <!-- Куда встанет брошенная вкладка: подсвечивается половина панели. -->
        <div
          v-if="dropZone"
          data-testid="drop-zone"
          class="absolute z-20 pointer-events-none border-2 border-accent bg-accent/15"
          :style="dropZone.style"
        />
        <p
          v-if="dropBlocked"
          data-testid="drop-blocked"
          class="absolute z-20 left-1/2 top-3 -translate-x-1/2 px-3 py-1 rounded-r8 bg-surface-card border border-edge text-xs text-on-surface pointer-events-none"
        >
          {{ t("layout.maxPanes", { count: layout.MAX_PANES }) }}
        </p>

        <div v-show="!activeSession" class="absolute inset-0 overflow-auto">
          <router-view />
        </div>
        </div>

        <!-- Файловый браузер SFTP (#122) -->
        <div v-if="sftpVisible && activeSession" class="w-[60%] max-w-[900px] min-w-[420px] shrink-0">
          <SftpPanel
            :key="activeSession.sessionId"
            :session-id="activeSession.sessionId"
            @close="sftp.panelOpen = false"
          />
        </div>
      </main>

      <!-- Передачи файлов (#124): снизу, чтобы не перекрывать работу -->
      <TransfersBar />
    </div>

    <!-- Глобальный TOFU диалог — один на app, обрабатывает все pending prompts. -->
    <HostKeyPromptDialog />

    <!-- Интерактивная аутентификация: 2FA-код (#153) -->
    <AuthPromptDialog />
    <AgentPlanDialog />
    <PlanResultDialog />

    <!-- Command Palette — Ctrl+K (#142) -->
    <CommandPalette />
    <!-- «Такое имя уже есть» перед передачей файлов -->
    <ConflictDialog />

    <OnboardingGate />

    <!-- Разблокировка мастер-паролем при старте (#112) -->
    <MasterUnlockGate />
    <StartupFailureGate />
  </div>
</template>
