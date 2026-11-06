import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  SessionStatusPayload,
  SessionDataPayload,
  SessionClosedPayload,
} from "../api/ssh";

/**
 * Подписки на события от Rust backend.
 * Имена событий должны точно совпадать с константами в `src-tauri/src/commands/ssh.rs`.
 */

const EVT_STATUS = "session:status";
const EVT_DATA = "session:data";
const EVT_CLOSED = "session:closed";
const EVT_HOST_KEY_PROMPT = "session:host_key_prompt"; // зарезервировано под #046
const EVT_AUTH_PROMPT = "session:auth_prompt"; // интерактивная аутентификация (#153)

export interface HostKeyPromptPayload {
  sessionId: string;
  hostname: string;
  keyType: string;
  fingerprint: string;
  kind: "new" | "changed";
  previousFingerprint?: string;
}

export interface SessionBusHandlers {
  onStatus?: (p: SessionStatusPayload) => void;
  onData?: (p: SessionDataPayload) => void;
  onClosed?: (p: SessionClosedPayload) => void;
  onHostKeyPrompt?: (p: HostKeyPromptPayload) => void;
  onAuthPrompt?: (p: AuthPromptPayload) => void;
}

/**
 * Подписаться на весь набор session-событий разом.
 * Возвращает функцию-отписку, которую нужно вызвать при уничтожении подписчика
 * (например в `onBeforeUnmount` компонента или при выгрузке стора).
 */
/** Запрос сервера при интерактивной аутентификации (#153). */
export interface AuthPromptField {
  prompt: string;
  /** false — ввод скрывать: это код или пароль, а не логин. */
  echo: boolean;
}

export interface AuthPromptPayload {
  sessionId: string;
  hostname: string;
  name: string;
  instructions: string;
  prompts: AuthPromptField[];
}

export async function listenSessionEvents(
  handlers: SessionBusHandlers,
): Promise<UnlistenFn> {
  const unsubs: UnlistenFn[] = [];

  if (handlers.onStatus) {
    unsubs.push(
      await listen<SessionStatusPayload>(EVT_STATUS, (e) =>
        handlers.onStatus!(e.payload),
      ),
    );
  }
  if (handlers.onData) {
    unsubs.push(
      await listen<SessionDataPayload>(EVT_DATA, (e) =>
        handlers.onData!(e.payload),
      ),
    );
  }
  if (handlers.onClosed) {
    unsubs.push(
      await listen<SessionClosedPayload>(EVT_CLOSED, (e) =>
        handlers.onClosed!(e.payload),
      ),
    );
  }
  if (handlers.onAuthPrompt) {
    unsubs.push(
      await listen<AuthPromptPayload>(EVT_AUTH_PROMPT, (e) =>
        handlers.onAuthPrompt!(e.payload),
      ),
    );
  }

  if (handlers.onHostKeyPrompt) {
    unsubs.push(
      await listen<HostKeyPromptPayload>(EVT_HOST_KEY_PROMPT, (e) =>
        handlers.onHostKeyPrompt!(e.payload),
      ),
    );
  }

  return async () => {
    for (const u of unsubs) {
      try {
        u();
      } catch {
        /* noop */
      }
    }
  };
}
