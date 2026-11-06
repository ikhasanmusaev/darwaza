import { defineStore } from "pinia";
import { ref, shallowRef } from "vue";

/**
 * Закладка в выводе терминала (#143).
 *
 * `line` — абсолютный номер строки в буфере xterm, а не в видимой области:
 * при прокрутке видимые координаты меняются, абсолютные — нет.
 */
export interface Bookmark {
  id: number;
  line: number;
  label: string;
  createdAt: number;
}

/**
 * Закладки живут **только в памяти** и умирают вместе с сессией.
 *
 * Сохранять их в БД бессмысленно: номер строки имеет смысл лишь внутри
 * конкретного буфера прокрутки, а он существует, пока открыта вкладка.
 * После переподключения тот же номер указывал бы в произвольное место.
 */
export const useBookmarksStore = defineStore("bookmarks", () => {
  const items = ref<Record<string, Bookmark[]>>({});
  const panelOpen = ref(false);

  let nextId = 1;

  /**
   * Как прокрутить терминал к строке. Регистрирует `TerminalPane`: только он
   * держит экземпляр xterm.
   */
  const scrollers = shallowRef(new Map<string, (line: number) => void>());
  /** Как поставить закладку на текущей строке — тоже знает только `TerminalPane`. */
  const adders = shallowRef(new Map<string, () => void>());

  function registerScroller(
    sessionId: string,
    fn: (line: number) => void,
  ): () => void {
    scrollers.value.set(sessionId, fn);
    return () => {
      scrollers.value.delete(sessionId);
    };
  }

  function registerAdder(sessionId: string, fn: () => void): () => void {
    adders.value.set(sessionId, fn);
    return () => {
      adders.value.delete(sessionId);
    };
  }

  /** Поставить закладку на текущей строке активной сессии. */
  function addHere(sessionId: string): void {
    adders.value.get(sessionId)?.();
  }

  function list(sessionId: string): Bookmark[] {
    return items.value[sessionId] ?? [];
  }

  function add(sessionId: string, line: number, label: string): Bookmark {
    const bookmark: Bookmark = {
      id: nextId++,
      line,
      label: label.trim() || `#${line}`,
      createdAt: Date.now(),
    };
    const current = items.value[sessionId] ?? [];
    // Держим по возрастанию строки: так список читается как сама сессия,
    // сверху вниз, а не в порядке добавления.
    const next = [...current, bookmark].sort((a, b) => a.line - b.line);
    items.value = { ...items.value, [sessionId]: next };
    return bookmark;
  }

  function remove(sessionId: string, id: number): void {
    const current = items.value[sessionId];
    if (!current) return;
    items.value = {
      ...items.value,
      [sessionId]: current.filter((b) => b.id !== id),
    };
  }

  function jump(sessionId: string, line: number): void {
    scrollers.value.get(sessionId)?.(line);
  }

  function togglePanel(): void {
    panelOpen.value = !panelOpen.value;
  }

  /** Забыть закладки закрытой сессии — её буфера больше нет. */
  function forget(sessionId: string): void {
    if (!items.value[sessionId]) return;
    const next = { ...items.value };
    delete next[sessionId];
    items.value = next;
  }

  return {
    items,
    panelOpen,
    registerScroller,
    registerAdder,
    addHere,
    list,
    add,
    remove,
    jump,
    togglePanel,
    forget,
  };
});
