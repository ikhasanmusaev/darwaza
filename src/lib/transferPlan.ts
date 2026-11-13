import type { ConflictChoice, ConflictQuestion } from "../stores/conflicts";

export type OnConflict = "rename" | "replace";

export interface PlannedTransfer<T> {
  item: T;
  onConflict: OnConflict;
}

/**
 * Решить, что делать с каждым элементом передачи, если в папке назначения
 * уже есть такое имя.
 *
 * Свободные имена идут без вопросов. На каждое совпадение спрашиваем; ответ
 * «для всех» действует на оставшиеся совпадения. «Пропустить» убирает элемент
 * из передачи. Имена в папке назначения берём из уже открытого списка:
 * он может устареть, но тогда backend всё равно не затрёт — положит рядом.
 */
export async function planTransfers<T extends { name: string; kind: string }>(
  items: T[],
  existing: Iterable<string>,
  ask: (q: ConflictQuestion) => Promise<ConflictChoice>,
): Promise<PlannedTransfer<T>[]> {
  const taken = new Set(existing);
  const clashing = items.filter((i) => taken.has(i.name));
  let remembered: ConflictChoice["action"] | null = null;
  const out: PlannedTransfer<T>[] = [];
  let asked = 0;
  for (const item of items) {
    if (!taken.has(item.name)) {
      out.push({ item, onConflict: "rename" });
      continue;
    }
    let action: ConflictChoice["action"] | null = remembered;
    if (!action) {
      const choice = await ask({
        name: item.name,
        isDir: item.kind === "dir",
        more: clashing.length - asked - 1,
      });
      action = choice.action;
      if (choice.forAll) remembered = action;
    }
    asked++;
    if (action === "skip") continue;
    out.push({ item, onConflict: action });
  }
  return out;
}
