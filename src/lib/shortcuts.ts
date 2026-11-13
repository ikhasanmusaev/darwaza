import { onBeforeUnmount, onMounted } from "vue";
import { useSessionsStore } from "../stores/sessions";
import { useCommandPaletteStore } from "../stores/command-palette";
import { useLayoutStore } from "../stores/layout";
import { nextTick } from "vue";
import { isAppMod, isMacOS } from "./platform";

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
 *
 * На macOS вместо Ctrl везде Cmd (⌘T, ⌘W, ⌘K, ⌘1..9, ⇧⌘D/E), а Ctrl остаётся
 * оболочке. Вкладки там ещё ⇧⌘[ и ⇧⌘], как в браузерах, а Ctrl+Tab — как
 * везде. Стрелки — ⌥⌘: голый Option со стрелкой в системе двигает курсор по
 * словам, и в терминале, и в полях ввода, отнимать его нельзя.
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
    const arrowMods = isMacOS()
      ? e.altKey && e.metaKey && !e.ctrlKey && !e.shiftKey
      : e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey;
    if (arrowMods && altArrows(e)) {
      e.preventDefault();
      e.stopPropagation();
      return;
    }

    // Ctrl+Tab / Ctrl+Shift+Tab — следующая/предыдущая. На всех системах:
    // на Mac ⌘Tab занят системой, а Ctrl+Tab оболочке не нужен.
    if (e.ctrlKey && !e.altKey && !e.metaKey && e.code === "Tab") {
      e.preventDefault();
      e.stopPropagation();
      move(tabOrder(), e.shiftKey ? -1 : 1);
      return;
    }

    // ⇧⌘[ и ⇧⌘] — привычные на Mac «предыдущая/следующая вкладка».
    if (
      isMacOS() &&
      e.metaKey &&
      e.shiftKey &&
      !e.ctrlKey &&
      !e.altKey &&
      (e.code === "BracketLeft" || e.code === "BracketRight")
    ) {
      e.preventDefault();
      e.stopPropagation();
      move(tabOrder(), e.code === "BracketLeft" ? -1 : 1);
      return;
    }

    if (!isAppMod(e) || e.altKey) return;

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
