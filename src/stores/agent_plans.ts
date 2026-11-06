import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { errorText } from "../lib/errors";

/** Строка в показе разницы (#206). */
export interface DiffLine {
  kind: "context" | "removed" | "added";
  text: string;
}

/** Вид шага. Определяет Rust; контракт — `docs/08-design/desktop/backend-gaps.md`. */
export type StepKind = "edit" | "check" | "service" | "run" | "blocked" | "skipped";

export interface PlanStep {
  kind: StepKind;
  /** Команда или путь — показывать целиком, без усечения. */
  title: string;
  /** Только у `blocked`. */
  blockedReason?: string;
  /** Машинный вид причины запрета: по нему окно подбирает перевод. */
  blockedCode?: "protected_path" | "never" | "device_write" | "delete_root";
  /** Путь (`protected_path`) или имя программы (`never`). */
  blockedArg?: string;
  /** Только у `edit`. */
  diff?: DiffLine[];
}

/** План изменений, ожидающий решения человека (#205, #227). */
export interface PendingPlan {
  id: string;
  host: string;
  hostId: number;
  hostAddress: string;
  hostUser: string;
  /** Как назвался агент — со слов самого агента, проверить нечем. */
  client: string;
  session: string;
  /** Зачем это делается — словами агента. */
  intent: string;
  steps: PlanStep[];
  /** Когда план будет отклонён сам, Unix-миллисекунды. */
  expiresAt: number;
}

/** Исход шага в итоге плана. Вывода команд здесь нет намеренно: он может нести секреты. */
export type StepOutcome = "done" | "failed" | "blocked" | "notRun";

export interface StepReport {
  title: string;
  /** `done` — код 0; `failed` — ненулевой код или сбой связи; `blocked` — запрещён, до сервера не дошёл; `notRun` — не выполнялся. */
  outcome: StepOutcome;
  exitCode: number | null;
}

/** Итог подтверждённого плана (#255); приходит событием после выполнения. */
export interface PlanReport {
  planId: string;
  host: string;
  /** `stopped` — остановлен на ошибке или запрещённом шаге: это не успех. */
  status: "completed" | "stopped";
  steps: StepReport[];
  /** Почему остановились; не длиннее 300 знаков. */
  reason: string | null;
  done: number;
  total: number;
}

/** Почему карточка плана больше не действует (#257). */
export type PlanGoneReason = "expired" | "rejected" | "cancelled";

const PLAN_EVENT = "agent://plan";
const PLAN_GONE_EVENT = "agent://plan_gone";
const PLAN_RESULT_EVENT = "agent://plan_result";

/**
 * Планы агента, ожидающие подтверждения (#205, #206).
 *
 * Пока план здесь, вызов агента стоит и ждёт ответа. Поэтому карточку нельзя
 * просто закрыть — только подтвердить или отклонить: молча закрытое окно
 * оставило бы агента ждать до предела в пять минут, и человек не понял бы,
 * почему тот «завис».
 */
export const useAgentPlansStore = defineStore("agentPlans", () => {
  const queue = ref<PendingPlan[]>([]);
  const busy = ref(false);
  const error = ref<string | null>(null);
  /** Итоги выполненных планов, новые в конце. Окно результата убирает их само. */
  const results = ref<PlanReport[]>([]);
  /**
   * Планы, которые больше нельзя подтвердить: срок вышел, агент ушёл или план
   * отклонён с другой стороны. Карточка остаётся на экране, чтобы человек
   * понял, что произошло, а «Подтвердить» у неё неактивна. Убирается
   * `dismissGone(id)`.
   */
  const gone = ref<Record<string, PlanGoneReason>>({});

  function markGone(id: string, reason: PlanGoneReason) {
    if (queue.value.some((p) => p.id === id)) {
      gone.value = { ...gone.value, [id]: reason };
    }
  }

  function dismissGone(id: string) {
    const { [id]: _removed, ...rest } = gone.value;
    gone.value = rest;
    drop(id);
  }

  function dismissResult(planId: string) {
    results.value = results.value.filter((r) => r.planId !== planId);
  }

  function current(): PendingPlan | null {
    return queue.value[0] ?? null;
  }

  function drop(id: string) {
    queue.value = queue.value.filter((p) => p.id !== id);
  }

  /**
   * Спросить, что ждёт ответа.
   *
   * Нужно при запуске окна: план мог прийти, пока окно было закрыто, и без
   * этого он висел бы невидимым до самого предела ожидания.
   */
  async function refresh(): Promise<void> {
    try {
      queue.value = await invoke<PendingPlan[]>("agent_plans_pending");
      gone.value = {};
    } catch {
      // Молча: доступ агента — дополнение, а не условие работы. Окно, не
      // открывшееся из-за недоступной очереди планов, отрезало бы человека
      // от собственных серверов на ровном месте.
      queue.value = [];
    }
  }

  async function subscribe(): Promise<UnlistenFn> {
    await refresh();
    const unlistenPlan = await listen<PendingPlan>(PLAN_EVENT, (e) => {
      // Повтор возможен, если событие пришло сразу после refresh.
      if (!queue.value.some((p) => p.id === e.payload.id)) {
        queue.value = [...queue.value, e.payload];
      }
    });
    const unlistenGone = await listen<{ id: string; reason: PlanGoneReason }>(
      PLAN_GONE_EVENT,
      (e) => {
        // Свой отказ карточка уже убрала сама; чужой или по сроку — помечаем.
        if (e.payload.reason !== "rejected") markGone(e.payload.id, e.payload.reason);
        else drop(e.payload.id);
      },
    );
    const unlistenResult = await listen<PlanReport>(PLAN_RESULT_EVENT, (e) => {
      results.value = [...results.value.filter((r) => r.planId !== e.payload.planId), e.payload];
    });
    return () => {
      unlistenPlan();
      unlistenResult();
      unlistenGone();
    };
  }

  async function answer(id: string, approve: boolean): Promise<void> {
    if (busy.value) return;
    busy.value = true;
    error.value = null;
    try {
      const answered = await invoke<boolean>(approve ? "agent_plan_approve" : "agent_plan_reject", {
        id,
      });
      // `false` при подтверждении — плана уже нет (агент не дождался): карточку
      // не убираем молча, а помечаем, как после события plan_gone.
      if (approve && !answered) markGone(id, "expired");
      else drop(id);
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = false;
    }
  }

  /** Отклонить всё разом — когда агент делает явно не то. */
  async function rejectAll(): Promise<void> {
    busy.value = true;
    try {
      await invoke<number>("agent_plans_reject_all");
      queue.value = [];
    } catch (e) {
      error.value = errorText(e);
    } finally {
      busy.value = false;
    }
  }

  return { queue, busy, error, results, gone, dismissGone, dismissResult, current, refresh, subscribe, answer, rejectAll };
});
