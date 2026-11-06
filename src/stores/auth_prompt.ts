import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { AuthPromptPayload } from "../lib/eventBus";

/**
 * Запросы интерактивной аутентификации (#153).
 *
 * Сервер с 2FA спрашивает код прямо во время подключения, и пока ответа нет,
 * рукопожатие стоит. Очередь — потому что одновременно могут подключаться
 * несколько сессий, и каждая ждёт своего ответа.
 */
export const useAuthPromptStore = defineStore("authPrompt", () => {
  const queue = ref<AuthPromptPayload[]>([]);

  function enqueue(p: AuthPromptPayload) {
    queue.value = [...queue.value, p];
  }

  function current(): AuthPromptPayload | null {
    return queue.value[0] ?? null;
  }

  function dequeue() {
    queue.value = queue.value.slice(1);
  }

  async function submit(sessionId: string, responses: string[]): Promise<void> {
    await invoke("ssh_submit_auth_prompt", { sessionId, responses });
    dequeue();
  }

  /** Отмена: сессия уйдёт в FAILED — подключиться без кода всё равно нельзя. */
  async function cancel(sessionId: string): Promise<void> {
    await invoke("ssh_cancel_auth_prompt", { sessionId });
    dequeue();
  }

  return { queue, enqueue, current, dequeue, submit, cancel };
});
