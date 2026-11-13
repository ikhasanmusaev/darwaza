<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import {
  ContextMenuRoot,
  ContextMenuTrigger,
  ContextMenuPortal,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
} from "radix-vue";
import { useSessionsStore } from "../stores/sessions";
import { useLayoutStore } from "../stores/layout";
import { errorText } from "../lib/errors";
import { shortcutLabel } from "../lib/platform";

/**
 * Контекстное меню вкладки (правая кнопка, Shift+F10 или клавиша меню).
 *
 * `sessionId = null` — вкладка «Kutubxona»: она не закрывается, поэтому в ней
 * только «Закрыть другие». Закрытие идёт через `store.close`, так что всё, что
 * сейчас спрашивает подтверждение при закрытии, спрашивает и здесь.
 */
const props = defineProps<{ sessionId: string | null }>();
const emit = defineEmits<{ rename: [sessionId: string]; error: [message: string] }>();

const { t } = useI18n();
const store = useSessionsStore();
const layout = useLayoutStore();

const tab = computed(() => store.sessions.find((s) => s.sessionId === props.sessionId) ?? null);
const isDead = computed(() => tab.value?.status === "FAILED" || tab.value?.status === "CLOSED");
const hasRight = computed(() => {
  const i = store.sessions.findIndex((s) => s.sessionId === props.sessionId);
  return i >= 0 && i < store.sessions.length - 1;
});
const hasOthers = computed(() => store.sessions.some((s) => s.sessionId !== props.sessionId));
/** Делить можно, пока панелей меньше четырёх. */
const splitBlocked = computed(() => layout.isSplit && !layout.canSplit);

async function run(action: () => Promise<unknown>) {
  try {
    await action();
  } catch (e) {
    emit("error", errorText(e));
  }
}

async function split(dir: "row" | "column") {
  if (!props.sessionId) return;
  store.setActive(props.sessionId);
  await run(() => store.splitActive(dir));
}
</script>

<template>
  <ContextMenuRoot>
    <ContextMenuTrigger as-child>
      <slot />
    </ContextMenuTrigger>
    <ContextMenuPortal>
      <ContextMenuContent
        data-testid="tab-menu"
        class="z-50 min-w-[220px] rounded-r8 border border-edge bg-surface-card p-1 text-xs shadow-none"
      >
        <template v-if="sessionId && tab">
          <ContextMenuItem
            data-testid="tab-menu-duplicate"
            class="menu-item"
            @select="run(() => store.duplicate(sessionId!))"
          >
            {{ t("tabMenu.duplicate") }}
          </ContextMenuItem>
          <ContextMenuItem
            data-testid="tab-menu-rename"
            class="menu-item"
            @select="emit('rename', sessionId!)"
          >
            {{ t("tabMenu.rename") }}
          </ContextMenuItem>
          <ContextMenuSeparator class="my-1 h-px bg-edge-subtle" />
          <ContextMenuItem
            data-testid="tab-menu-split-row"
            class="menu-item"
            :disabled="splitBlocked"
            :title="splitBlocked ? t('layout.maxPanes', { count: layout.MAX_PANES }) : undefined"
            @select="split('row')"
          >
            {{ t("tabMenu.splitRight") }}
            <span class="menu-hint">{{ shortcutLabel("Shift", "D") }}</span>
          </ContextMenuItem>
          <ContextMenuItem
            data-testid="tab-menu-split-column"
            class="menu-item"
            :disabled="splitBlocked"
            :title="splitBlocked ? t('layout.maxPanes', { count: layout.MAX_PANES }) : undefined"
            @select="split('column')"
          >
            {{ t("tabMenu.splitDown") }}
            <span class="menu-hint">{{ shortcutLabel("Shift", "E") }}</span>
          </ContextMenuItem>
          <ContextMenuItem
            v-if="isDead"
            data-testid="tab-menu-reconnect"
            class="menu-item"
            @select="run(() => store.reconnect(sessionId!))"
          >
            {{ t("tabMenu.reconnect") }}
          </ContextMenuItem>
          <ContextMenuSeparator class="my-1 h-px bg-edge-subtle" />
          <ContextMenuItem
            data-testid="tab-menu-close"
            class="menu-item"
            @select="run(() => store.close(sessionId!))"
          >
            {{ t("tabMenu.close") }}
            <span class="menu-hint">{{ shortcutLabel("W") }}</span>
          </ContextMenuItem>
          <ContextMenuItem
            data-testid="tab-menu-close-others"
            class="menu-item"
            :disabled="!hasOthers"
            @select="run(() => store.closeOthers(sessionId))"
          >
            {{ t("tabMenu.closeOthers") }}
          </ContextMenuItem>
          <ContextMenuItem
            data-testid="tab-menu-close-right"
            class="menu-item"
            :disabled="!hasRight"
            @select="run(() => store.closeToRight(sessionId!))"
          >
            {{ t("tabMenu.closeRight") }}
          </ContextMenuItem>
        </template>
        <template v-else>
          <ContextMenuItem
            data-testid="tab-menu-close-others"
            class="menu-item"
            :disabled="store.sessions.length === 0"
            @select="run(() => store.closeOthers(null))"
          >
            {{ t("tabMenu.closeOthers") }}
          </ContextMenuItem>
        </template>
      </ContextMenuContent>
    </ContextMenuPortal>
  </ContextMenuRoot>
</template>

<style scoped>
.menu-item {
  @apply flex items-center gap-6 px-2.5 py-1.5 rounded-r6 cursor-pointer text-on-surface outline-none;
}
.menu-item[data-highlighted] {
  @apply bg-surface-alt;
}
.menu-item[data-disabled] {
  @apply opacity-45 cursor-not-allowed;
}
.menu-hint {
  @apply ml-auto font-mono text-[10px] text-on-surface-muted;
}
</style>
