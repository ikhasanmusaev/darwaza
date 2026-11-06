import { defineStore } from "pinia";
import { ref, computed } from "vue";

/**
 * Раскладка панелей терминала (#140).
 *
 * Раскладка — двоичное дерево: лист — сессия, узел — деление на две части вдоль
 * `row` (рядом) или `column` (друг под другом) с долей первой части. Так
 * четыре панели 2×2 получаются делением любой панели с любой стороны, а
 * разделитель между двумя частями двигается и меняет долю.
 *
 * Сами терминалы **не вложены** в дерево: каждый рисуется отдельным элементом
 * в одном списке и ставится по прямоугольнику из `rects`. Поэтому при
 * разделении и перетаскивании xterm не пересоздаётся — меняется только CSS.
 */
export type Direction = "row" | "column";

export type LayoutNode =
  | { leaf: string }
  | { dir: Direction; ratio: number; a: LayoutNode; b: LayoutNode };

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Разделитель между двумя частями узла. `path` — путь от корня («a»/«b»). */
export interface Divider {
  path: string;
  dir: Direction;
  /** Прямоугольник узла целиком (доли от окна): по нему считается новая доля. */
  box: Rect;
  ratio: number;
}

/** Не уже этой доли ни одна часть не бывает: тоньше терминал нечитаем. */
const MIN_RATIO = 0.15;

function leaves(n: LayoutNode): string[] {
  return "leaf" in n ? [n.leaf] : [...leaves(n.a), ...leaves(n.b)];
}

function layout(
  n: LayoutNode,
  box: Rect,
  path: string,
  rects: Record<string, Rect>,
  dividers: Divider[],
) {
  if ("leaf" in n) {
    rects[n.leaf] = box;
    return;
  }
  dividers.push({ path, dir: n.dir, box, ratio: n.ratio });
  if (n.dir === "row") {
    const w1 = box.w * n.ratio;
    layout(n.a, { ...box, w: w1 }, path + "a", rects, dividers);
    layout(n.b, { ...box, x: box.x + w1, w: box.w - w1 }, path + "b", rects, dividers);
  } else {
    const h1 = box.h * n.ratio;
    layout(n.a, { ...box, h: h1 }, path + "a", rects, dividers);
    layout(n.b, { ...box, y: box.y + h1, h: box.h - h1 }, path + "b", rects, dividers);
  }
}

/** Заменить лист `id` поддеревом `by`. */
function replaceLeaf(n: LayoutNode, id: string, by: LayoutNode): LayoutNode {
  if ("leaf" in n) return n.leaf === id ? by : n;
  return { ...n, a: replaceLeaf(n.a, id, by), b: replaceLeaf(n.b, id, by) };
}

/** Убрать лист `id`: его место целиком занимает сосед. */
function removeLeaf(n: LayoutNode, id: string): LayoutNode | null {
  if ("leaf" in n) return n.leaf === id ? null : n;
  const a = removeLeaf(n.a, id);
  const b = removeLeaf(n.b, id);
  if (a && b) return { ...n, a, b };
  return a ?? b;
}

function setRatioAt(n: LayoutNode, path: string, ratio: number): LayoutNode {
  if ("leaf" in n) return n;
  if (path === "") return { ...n, ratio };
  const rest = path.slice(1);
  return path[0] === "a"
    ? { ...n, a: setRatioAt(n.a, rest, ratio) }
    : { ...n, b: setRatioAt(n.b, rest, ratio) };
}

function flip(n: LayoutNode): LayoutNode {
  if ("leaf" in n) return n;
  return { ...n, dir: n.dir === "row" ? "column" : "row", a: flip(n.a), b: flip(n.b) };
}

export const useLayoutStore = defineStore("layout", () => {
  /** `null` — обычный режим одной вкладки. */
  const tree = ref<LayoutNode | null>(null);
  /** Направление, которым делит Ctrl+клик по вкладке и кнопка переключения. */
  const direction = ref<Direction>("row");

  /** Больше четырёх панелей (2×2) на экране — нечитаемые полоски. */
  const MAX_PANES = 4;

  /** Сессии, показываемые одновременно, в порядке обхода дерева. */
  const panes = computed<string[]>(() => (tree.value ? leaves(tree.value) : []));
  const isSplit = computed(() => panes.value.length > 1);

  const geometry = computed(() => {
    const rects: Record<string, Rect> = {};
    const dividers: Divider[] = [];
    if (tree.value) layout(tree.value, { x: 0, y: 0, w: 1, h: 1 }, "", rects, dividers);
    return { rects, dividers };
  });
  const rects = computed(() => geometry.value.rects);
  const dividers = computed(() => geometry.value.dividers);

  function isVisible(sessionId: string, activeSessionId: string | null): boolean {
    if (panes.value.length === 0) return sessionId === activeSessionId;
    return panes.value.includes(sessionId);
  }

  /** Показать только одну сессию — обычный режим. */
  function single(sessionId: string): void {
    tree.value = { leaf: sessionId };
  }

  function reset(): void {
    tree.value = null;
  }

  /** Можно ли добавить ещё одну панель. */
  const canSplit = computed(() => panes.value.length < MAX_PANES);

  /**
   * Разделить панель `target` и положить рядом `sessionId`.
   * `before` — новая панель слева/сверху от `target`. `false`, если предел
   * достигнут или `sessionId` уже в раскладке.
   */
  function splitPane(
    target: string,
    sessionId: string,
    dir: Direction = direction.value,
    before = false,
  ): boolean {
    const current = tree.value ?? { leaf: target };
    const ids = leaves(current);
    if (ids.includes(sessionId)) return true;
    if (ids.length >= MAX_PANES || !ids.includes(target)) return false;
    tree.value = replaceLeaf(current, target, {
      dir,
      ratio: 0.5,
      a: before ? { leaf: sessionId } : { leaf: target },
      b: before ? { leaf: target } : { leaf: sessionId },
    });
    return true;
  }

  /** Добавить сессию в раскладку (Ctrl+клик по вкладке): делит последнюю панель. */
  function addPane(sessionId: string, activeSessionId: string | null): boolean {
    const ids = panes.value;
    const target = ids.length > 0 ? ids[ids.length - 1] : activeSessionId;
    if (!target) return false;
    if (target === sessionId) return true;
    return splitPane(target, sessionId);
  }

  function removePane(sessionId: string): void {
    if (!tree.value) return;
    const next = removeLeaf(tree.value, sessionId);
    // Одна оставшаяся панель — это уже обычный режим, дерево можно сбросить.
    tree.value = next && leaves(next).length > 1 ? next : null;
  }

  /** Ctrl+клик по вкладке: добавить в раскладку или убрать из неё. */
  function togglePane(sessionId: string, activeSessionId: string | null): boolean {
    if (panes.value.includes(sessionId)) {
      removePane(sessionId);
      return true;
    }
    return addPane(sessionId, activeSessionId);
  }

  function toggleDirection(): void {
    direction.value = direction.value === "row" ? "column" : "row";
    if (tree.value) tree.value = flip(tree.value);
  }

  /** Передвинуть разделитель; доля зажимается, чтобы панель не исчезла. */
  function setRatio(path: string, ratio: number): void {
    if (!tree.value) return;
    const r = Math.min(1 - MIN_RATIO, Math.max(MIN_RATIO, ratio));
    tree.value = setRatioAt(tree.value, path, r);
  }

  /** Сессия закрылась — убрать её из раскладки. */
  function forget(sessionId: string): void {
    if (!panes.value.includes(sessionId)) return;
    removePane(sessionId);
  }

  return {
    tree,
    panes,
    direction,
    isSplit,
    canSplit,
    rects,
    dividers,
    MAX_PANES,
    isVisible,
    single,
    reset,
    splitPane,
    addPane,
    removePane,
    togglePane,
    toggleDirection,
    setRatio,
    forget,
  };
});
