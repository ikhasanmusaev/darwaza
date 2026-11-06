import { defineStore } from "pinia";
import { ref } from "vue";
import { sftpApi, type SftpEntry } from "../api/sftp";

function errMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}

/**
 * Состояние файлового браузера одной сессии.
 *
 * Состояние держим по sessionId: у пользователя может быть открыто несколько
 * вкладок, и переключение между ними не должно сбрасывать текущую директорию.
 */
interface PaneState {
  path: string;
  entries: SftpEntry[];
  loading: boolean;
  error: string | null;
}

function emptyPane(): PaneState {
  return { path: "", entries: [], loading: false, error: null };
}

export const useSftpStore = defineStore("sftp", () => {
  const panes = ref<Record<string, PaneState>>({});
  /** Открыта ли панель файлов. Одна на приложение: показывается для активной сессии. */
  const panelOpen = ref(false);

  function togglePanel(): void {
    panelOpen.value = !panelOpen.value;
  }

  function pane(sessionId: string): PaneState {
    if (!panes.value[sessionId]) {
      panes.value[sessionId] = emptyPane();
    }
    return panes.value[sessionId];
  }

  /** Открыть директорию. Пустой путь = домашняя директория пользователя. */
  async function open(sessionId: string, path?: string): Promise<void> {
    const p = pane(sessionId);
    p.loading = true;
    p.error = null;
    try {
      const target = path ?? (await sftpApi.home(sessionId));
      p.entries = await sftpApi.list(sessionId, target);
      p.path = target;
    } catch (e) {
      p.error = errMessage(e);
    } finally {
      p.loading = false;
    }
  }

  async function refresh(sessionId: string): Promise<void> {
    const p = pane(sessionId);
    if (!p.path) return open(sessionId);
    return open(sessionId, p.path);
  }

  async function goUp(sessionId: string): Promise<void> {
    const p = pane(sessionId);
    if (!p.path) return;
    const parent = await sftpApi.parent(p.path);
    if (parent !== p.path) await open(sessionId, parent);
  }

  async function mkdir(sessionId: string, path: string): Promise<void> {
    await sftpApi.mkdir(sessionId, path);
    await refresh(sessionId);
  }

  async function remove(
    sessionId: string,
    path: string,
    isDir: boolean,
  ): Promise<void> {
    await sftpApi.remove(sessionId, path, isDir);
    await refresh(sessionId);
  }

  async function rename(
    sessionId: string,
    from: string,
    to: string,
  ): Promise<void> {
    await sftpApi.rename(sessionId, from, to);
    await refresh(sessionId);
  }

  async function download(
    sessionId: string,
    remotePath: string,
    localPath: string,
  ): Promise<number> {
    return sftpApi.download(sessionId, remotePath, localPath);
  }

  async function upload(
    sessionId: string,
    localPath: string,
    remotePath: string,
  ): Promise<number> {
    const bytes = await sftpApi.upload(sessionId, localPath, remotePath);
    await refresh(sessionId);
    return bytes;
  }

  /** Забыть состояние вкладки — вызывается при закрытии сессии. */
  function forget(sessionId: string): void {
    delete panes.value[sessionId];
  }

  return {
    panes,
    panelOpen,
    togglePanel,
    pane,
    open,
    refresh,
    goUp,
    mkdir,
    remove,
    rename,
    download,
    upload,
    forget,
  };
});
