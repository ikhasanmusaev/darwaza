import { defineStore } from "pinia";
import { ref } from "vue";

/** Ответ на вопрос «такое имя уже есть». */
export interface ConflictChoice {
  action: "replace" | "skip" | "rename";
  /** Тот же ответ для всех следующих совпадений этой передачи. */
  forAll: boolean;
}

export interface ConflictQuestion {
  name: string;
  isDir: boolean;
  /** Сколько ещё совпадений после этого — показываем «для всех», если есть. */
  more: number;
}

/**
 * Окно «такое имя уже есть» — одно на приложение.
 *
 * Спрашивает файловая панель перед передачей; окно монтируется в Layout и
 * отвечает через `answer`. Закрыли окно без ответа (Esc) — это «пропустить»:
 * ничего не делать безопаснее, чем что-то затереть.
 */
export const useConflictsStore = defineStore("conflicts", () => {
  const question = ref<ConflictQuestion | null>(null);
  let resolve: ((c: ConflictChoice) => void) | null = null;

  function ask(q: ConflictQuestion): Promise<ConflictChoice> {
    // Предыдущий вопрос без ответа (вторая передача поверх первой) — пропуск.
    resolve?.({ action: "skip", forAll: false });
    question.value = q;
    return new Promise((r) => {
      resolve = r;
    });
  }

  function answer(choice: ConflictChoice): void {
    const r = resolve;
    resolve = null;
    question.value = null;
    r?.(choice);
  }

  return { question, ask, answer };
});
