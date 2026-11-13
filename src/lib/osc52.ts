/** Больше этого в буфер из терминала не принимаем: текст столько не весит. */
const MAX_BASE64_LENGTH = 8 * 1024 * 1024;

/**
 * Разобрать OSC 52 (`Pc;Pd`): программа на сервере просит записать текст в буфер.
 *
 * Так копируют Claude Code в полноэкранном режиме, tmux и vim: мышь у них, и
 * терминал сам ничего не выделяет. Возвращает текст или `null`, если записывать
 * нечего. Запрос чтения (`?`) не исполняется никогда: иначе любой сервер мог бы
 * забрать содержимое буфера, а там бывают пароли. Пустой текст буфер не стирает.
 */
export function parseOsc52(data: string): string | null {
  const separator = data.indexOf(";");
  if (separator === -1) return null;

  const target = data.slice(0, separator);
  const payload = data.slice(separator + 1);
  const isClipboard = target === "" || target.includes("c") || target.includes("s");
  if (!isClipboard || !payload || payload === "?" || payload.length > MAX_BASE64_LENGTH) {
    return null;
  }

  try {
    const bytes = Uint8Array.from(atob(payload), (c) => c.charCodeAt(0));
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}
