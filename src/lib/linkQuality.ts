/**
 * Качество связи с сервером для подписи у вкладок.
 *
 * Замер — открытие пустого канала SSH раз в несколько секунд. Ответил быстро —
 * зелёный «Подключено · 12 ms». Медленно или один замер пропал — оранжевый.
 * Совсем медленно или пропало несколько подряд — красный: значит, сеть легла,
 * хотя соединение формально ещё открыто.
 */
export type LinkQuality = "good" | "slow" | "bad";

/** Что показать: цвет и какая подпись. */
export interface LinkView {
  quality: LinkQuality;
  /** Ключ перевода подписи. */
  label: "session.status.CONNECTED" | "session.link.slow" | "session.link.unstable" | "session.link.lost";
  /** Миллисекунды, если последний замер удался. */
  ms: number | null;
}

/** С этой задержки печатать уже заметно неприятно. */
export const SLOW_MS = 300;
/** С этой — работать почти невозможно. */
export const BAD_MS = 1500;
/** Столько пропавших замеров подряд — сервер считаем недоступным. */
export const LOST_AFTER = 2;

/**
 * @param ms задержка последнего удачного замера, `null` — замеров ещё не было
 * @param failures сколько замеров подряд не удалось
 */
export function linkView(ms: number | null, failures: number): LinkView {
  if (failures >= LOST_AFTER) return { quality: "bad", label: "session.link.lost", ms: null };
  if (failures > 0) return { quality: "slow", label: "session.link.unstable", ms: null };
  if (ms === null) return { quality: "good", label: "session.status.CONNECTED", ms: null };
  if (ms >= BAD_MS) return { quality: "bad", label: "session.link.slow", ms };
  if (ms >= SLOW_MS) return { quality: "slow", label: "session.link.slow", ms };
  return { quality: "good", label: "session.status.CONNECTED", ms };
}
