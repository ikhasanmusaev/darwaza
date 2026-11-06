import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useSessionsStore } from "../stores/sessions";
import { useHostsStore } from "../stores/hosts";

/**
 * Агент открывает человеку вкладку с подготовленной командой (#216).
 *
 * Команда **набирается** в приглашении и не выполняется: Enter нажимает
 * человек. Так агент подаёт то, что ему самому запрещено навсегда — разметку
 * диска, смену ключа сервера, — вместо «сделайте сами».
 */
const TERMINAL_EVENT = "agent://terminal";

interface TerminalRequest {
  hostId: number;
  host: string;
  command: string | null;
}

/** Сколько ждать подключения, прежде чем сдаться. */
const CONNECT_TIMEOUT_MS = 30_000;

/**
 * Дождаться, пока сессия подключится.
 *
 * Без ожидания набранная команда уходит в ещё не открытый канал и теряется —
 * человек видит пустое приглашение и не понимает, чего от него хотели.
 */
async function waitConnected(sessionId: string): Promise<boolean> {
  const sessions = useSessionsStore();
  const deadline = Date.now() + CONNECT_TIMEOUT_MS;

  while (Date.now() < deadline) {
    const session = sessions.sessions.find((s) => s.sessionId === sessionId);
    if (!session) return false;
    if (session.status === "CONNECTED") return true;
    if (session.status === "FAILED" || session.status === "CLOSED") return false;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  return false;
}

export function subscribeAgentTerminal(): Promise<UnlistenFn> {
  return listen<TerminalRequest>(TERMINAL_EVENT, async (event) => {
    const { hostId, host, command } = event.payload;
    const sessions = useSessionsStore();

    // Имя берём из списка хостов, а не из события: хост могли переименовать,
    // и подпись вкладки должна совпадать с тем, что человек видит у себя.
    const hosts = useHostsStore();
    await hosts.ensureLoaded();
    const label = hosts.hosts.find((h) => h.id === hostId)?.name ?? host;

    let sessionId: string;
    try {
      sessionId = await sessions.open(hostId, label);
    } catch {
      // Не подключились — человек увидит это во вкладке обычным путём.
      return;
    }

    if (!command) return;
    if (!(await waitConnected(sessionId))) return;

    // Без перевода строки — в этом весь смысл: команда набрана, решение за
    // человеком. Backend отдельно отвергает команды с переводом строки, так
    // что сюда он дойти не может, но полагаться на это нельзя: между ними
    // событие, а не вызов функции.
    const typed = command.replace(/[\r\n]/g, " ");
    await sessions.send(sessionId, new TextEncoder().encode(typed)).catch(() => undefined);
  });
}
