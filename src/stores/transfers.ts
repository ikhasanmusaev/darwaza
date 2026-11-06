import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { transfersApi, type TransferProgress, type TreeEndpoint } from "../api/transfers";

/**
 * Очередь передач для панели снизу (#124).
 *
 * Состояние ведёт backend, здесь только отражение: события приходят на каждый
 * шаг и заменяют запись целиком.
 */
export const useTransfersStore = defineStore("transfers", () => {
  const items = ref<TransferProgress[]>([]);
  let unsubscribe: UnlistenFn | null = null;

  const active = computed(() => items.value.filter((t) => t.status === "RUNNING"));
  /** Панель показывается, пока есть что показывать. */
  const visible = computed(() => items.value.length > 0);

  async function init() {
    if (unsubscribe) return;
    unsubscribe = await listen<TransferProgress>("transfer:progress", (e) => {
      const incoming = e.payload;
      const i = items.value.findIndex((t) => t.id === incoming.id);
      if (i >= 0) {
        items.value[i] = incoming;
      } else {
        items.value = [...items.value, incoming];
      }
    });
  }

  async function startDownload(
    sessionId: string,
    remotePath: string,
    targetDir?: string,
  ): Promise<string> {
    return transfersApi.download(sessionId, remotePath, targetDir);
  }

  async function startUpload(
    sessionId: string,
    localPath: string,
    remoteDir: string,
  ): Promise<string> {
    return transfersApi.upload(sessionId, localPath, remoteDir);
  }

  async function startTree(
    source: TreeEndpoint,
    sourcePath: string,
    isDir: boolean,
    target: TreeEndpoint,
    targetDir: string,
  ): Promise<string> {
    return transfersApi.tree(source, sourcePath, isDir, target, targetDir);
  }

  async function cancel(id: string): Promise<void> {
    await transfersApi.cancel(id);
  }

  /** Убрать запись из списка. Завершённые копятся, пока их не уберут. */
  function dismiss(id: string): void {
    items.value = items.value.filter((t) => t.id !== id);
  }

  function dismissFinished(): void {
    items.value = items.value.filter((t) => t.status === "RUNNING");
  }

  return {
    items,
    active,
    visible,
    init,
    startDownload,
    startUpload,
    startTree,
    cancel,
    dismiss,
    dismissFinished,
  };
});
