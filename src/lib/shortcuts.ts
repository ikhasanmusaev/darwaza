import { onBeforeUnmount, onMounted } from "vue";
import { useSessionsStore } from "../stores/sessions";
import { useCommandPaletteStore } from "../stores/command-palette";
import { useLayoutStore } from "../stores/layout";
import { nextTick } from "vue";

/**
 * Глобальные горячие клавиши вкладок (#054).
 * Вешаются на window в capture-фазе, чтобы перехватить событие
 * раньше xterm (он жадно поглощает Ctrl-комбинации).
 *
 * - Ctrl+T            — вернуться на Library-вкладку (router-view).
 * - Ctrl+W            — закрыть активную SSH-сессию (для Library — no-op).
 * - Ctrl+Tab          — следующая вкладка по кругу (Library → sessions → Library).
 * - Ctrl+Shift+Tab    — предыдущая.
 * - Ctrl+1            — Library.
 * - Ctrl+2..9         — sessions[0..7] (если есть).
 * - Ctrl+Shift+D/E    — разделить активную панель вправо/вниз (до 4 панелей).
 * - Alt+←/→           — предыдущая/следующая вкладка по кругу.
 * - Alt+↑/↓           — при разделении экрана фокус между панелями, иначе как ←/→.
 * Привязка к `e.code`, а не к `e.key`: не зависит от раскладки.
 */
export function useGlobalShortcuts() {
  const sessions = useSessionsStore();
  const palette = useCommandPaletteStore();
  const layout = useLayoutStore();

  function tabOrder(): (string | null)[] {
    return [null, ...sessions.sessions.map((s) => s.sessionId)];
  }

  function setIndex(n: number) {
    const order = tabOrder();
    if (n >= 0 && n < order.length) sessions.setActive(order[n]);
  }

  /** Переставить активную вкладку на `step` по кругу в списке `order`. */
  function move(order: (string | null)[], step: 1 | -1) {
    const i = order.indexOf(sessions.activeSessionId);
    const idx = i < 0 ? 0 : i;
    sessions.setActive(order[(idx + step + order.length) % order.length]);
  }

  /** Отдать фокус терминалу активной панели: иначе клавиатура остаётся на прежней. */
  async function focusActiveTerminal() {
    await nextTick();
    const id = sessions.activeSessionId;
    if (!id) return;
    document
      .querySelector<HTMLElement>(`[data-session-id="${id}"] .xterm-helper-textarea`)
      ?.focus();
  }

  /** Alt+стрелки. `true` — нажатие обработано. */
  function altArrows(e: KeyboardEvent): boolean {
    const step = e.code === "ArrowLeft" || e.code === "ArrowUp" ? -1 : e.code === "ArrowRight" || e.code === "ArrowDown" ? 1 : 0;
    if (step === 0) return false;
    const vertical = e.code === "ArrowUp" || e.code === "ArrowDown";
    if (vertical && layout.isSplit) move(layout.panes, step);
    else move(tabOrder(), step);
    void focusActiveTerminal();
    return true;
  }

  function handler(e: KeyboardEvent) {
    // Alt+стрелки — до xterm, иначе они уйдут в оболочку ESC-последовательностью.
    if (e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey && altArrows(e)) {
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    if (!e.ctrlKey || e.altKey || e.metaKey) return;

    // Ctrl+Shift+D — разделить активную панель вправо, Ctrl+Shift+E — вниз.
    if (e.shiftKey && (e.code === "KeyD" || e.code === "KeyE")) {
      e.preventDefault();
      e.stopPropagation();
      void sessions.splitActive(e.code === "KeyD" ? "row" : "column");
      return;
    }

    // Ctrl+T → Library (фокус на навигацию)
    if (e.code === "KeyT" && !e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();
      sessions.setActive(null);
      return;
    }

    // Ctrl+W → закрыть активную сессию
    if (e.code === "KeyW" && !e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();
      if (sessions.activeSessionId) {
        void sessions.close(sessions.activeSessionId);
      }
      return;
    }

    // Ctrl+Tab / Ctrl+Shift+Tab — следующая/предыдущая
    if (e.code === "Tab") {
      e.preventDefault();
      e.stopPropagation();
      const order = tabOrder();
      const i = order.indexOf(sessions.activeSessionId);
      const idx = i < 0 ? 0 : i;
      const next = e.shiftKey
        ? (idx - 1 + order.length) % order.length
        : (idx + 1) % order.length;
      sessions.setActive(order[next]);
      return;
    }

    // Ctrl+K → открыть Command Palette
    if (e.code === "KeyK" && !e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();
      palette.open();
      return;
    }

    // Ctrl+1..9 → переход по индексу
    const match = e.code.match(/^Digit([1-9])$/);
    if (match) {
      e.preventDefault();
      e.stopPropagation();
      const n = parseInt(match[1], 10) - 1;
      setIndex(n);
    }
  }

  onMounted(() => {
    window.addEventListener("keydown", handler, true);
  });
  onBeforeUnmount(() => {
    window.removeEventListener("keydown", handler, true);
  });
}
