/**
 * Выбор нескольких записей в списке файлов, как в любом файловом менеджере:
 * щелчок — одна запись, Ctrl (⌘ на Mac) — добавить или убрать, Shift —
 * диапазон от прошлого щелчка.
 */
export interface Pickable {
  path: string;
}

export interface SelectionState<T extends Pickable> {
  picked: T[];
  /** От неё считается диапазон Shift; она же «текущая» для переименования. */
  anchor: T | null;
}

export function nextSelection<T extends Pickable>(
  list: T[],
  state: SelectionState<T>,
  clicked: T,
  mods: { toggle: boolean; range: boolean },
): SelectionState<T> {
  if (mods.range && state.anchor) {
    const a = list.findIndex((e) => e.path === state.anchor!.path);
    const b = list.findIndex((e) => e.path === clicked.path);
    if (a >= 0 && b >= 0) {
      const [from, to] = a <= b ? [a, b] : [b, a];
      return { picked: list.slice(from, to + 1), anchor: state.anchor };
    }
  }
  if (mods.toggle) {
    const has = state.picked.some((e) => e.path === clicked.path);
    const picked = has
      ? state.picked.filter((e) => e.path !== clicked.path)
      : [...state.picked, clicked];
    return { picked, anchor: clicked };
  }
  return { picked: [clicked], anchor: clicked };
}

export function isPicked<T extends Pickable>(picked: T[], entry: T): boolean {
  return picked.some((e) => e.path === entry.path);
}
