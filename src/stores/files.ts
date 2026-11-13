import { defineStore } from "pinia";
import { ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { sftpApi } from "../api/sftp";
import { localFsApi } from "../api/localfs";
import { listenSessionEvents } from "../lib/eventBus";
import { errorText } from "../lib/errors";
import { useSessionsStore } from "./sessions";
import { useTransfersStore } from "./transfers";
import type { TreeEndpoint } from "../api/transfers";
import { useConflictsStore } from "./conflicts";
import { planTransfers } from "../lib/transferPlan";
import { nextSelection } from "../lib/selection";

/**
 * Экран SFTP: две панели, в каждой — компьютер или любой сервер (#sftp-screen).
 *
 * Сервер открывается через уже живую вкладку терминала этого хоста, если она
 * есть, иначе отдельным соединением «только для файлов» (без оболочки).
 */
export type Side = "left" | "right";

export interface FileEntry {
  name: string;
  path: string;
  kind: "dir" | "file" | "symlink" | "other";
  size: number;
  modified: number | null;
  /** Unix-режим; у локальных записей его нет. */
  permissions?: number | null;
}

export type Source = { kind: "local" } | { kind: "host"; hostId: number };

interface Pane {
  source: Source | null;
  /** Сессия, через которую идёт SFTP; у локальной стороны её нет. */
  sessionId: string | null;
  path: string;
  entries: FileEntry[];
  loading: boolean;
  connecting: boolean;
  error: string | null;
  /**
   * Ошибка правки (папка, имя, удаление). Отдельно от `error`: та заменяет
   * список, а после неудачной правки список обновляется и стёр бы её.
   */
  opError: string | null;
  /** Последняя щёлкнутая запись: от неё диапазон Shift, её переименовываем. */
  selected: FileEntry | null;
  /** Все выбранные (Ctrl/⌘ и Shift): их передаём и удаляем. */
  picked: FileEntry[];
}

function emptyPane(): Pane {
  return {
    source: null,
    sessionId: null,
    path: "",
    entries: [],
    loading: false,
    connecting: false,
    error: null,
    opError: null,
    selected: null,
    picked: [],
  };
}

const other = (side: Side): Side => (side === "left" ? "right" : "left");

/** Объединить путь на сервере с именем. */
function joinRemote(dir: string, name: string): string {
  return dir === "/" ? `/${name}` : `${dir.replace(/\/+$/, "")}/${name}`;
}

export const useFilesStore = defineStore("files", () => {
  const left = ref<Pane>(emptyPane());
  const right = ref<Pane>(emptyPane());
  /** Что тащим между панелями. */
  const dragging = ref<{ side: Side; entries: FileEntry[] } | null>(null);

  /** Свои соединения «только для файлов»: hostId → sessionId. */
  const owned = new Map<number, string>();
  /** Ожидающие готовности: sessionId → колбэки. */
  const waiting = new Map<string, { ok: () => void; fail: (e: string) => void }>();
  let unlisten: UnlistenFn | null = null;

  const pane = (side: Side) => (side === "left" ? left.value : right.value);

  async function init() {
    if (unlisten) return;
    unlisten = await listenSessionEvents({
      onStatus: ({ sessionId, status, error }) => {
        const w = waiting.get(sessionId);
        if (!w) return;
        if (status === "CONNECTED") {
          waiting.delete(sessionId);
          w.ok();
        } else if (status === "FAILED" || status === "CLOSED") {
          waiting.delete(sessionId);
          w.fail(error ?? "");
        }
      },
    });
    // Левая панель по умолчанию — свой компьютер.
    if (!left.value.source) await setSource("left", { kind: "local" });
  }

  /** Соединение для хоста: живая вкладка или отдельное «для файлов». */
  async function sessionFor(hostId: number): Promise<string> {
    const tab = useSessionsStore().sessions.find(
      (s) => s.hostId === hostId && s.status === "CONNECTED",
    );
    if (tab) return tab.sessionId;
    const known = owned.get(hostId);
    if (known) return known;

    const { sessionId } = await sftpApi.connect(hostId);
    owned.set(hostId, sessionId);
    try {
      await new Promise<void>((ok, fail) => {
        waiting.set(sessionId, { ok, fail: (e) => fail(new Error(e)) });
      });
    } catch (e) {
      owned.delete(hostId);
      throw e;
    }
    return sessionId;
  }

  /** Закрыть своё соединение, если им больше никто не пользуется. */
  function releaseUnused() {
    const used = new Set([left.value.sessionId, right.value.sessionId]);
    for (const [hostId, sessionId] of owned) {
      if (!used.has(sessionId)) {
        owned.delete(hostId);
        void sftpApi.disconnect(sessionId).catch(() => undefined);
      }
    }
  }

  async function setSource(side: Side, source: Source | null) {
    const p = pane(side);
    Object.assign(p, emptyPane(), { source });
    if (!source) {
      releaseUnused();
      return;
    }
    if (source.kind === "local") {
      releaseUnused();
      await open(side);
      return;
    }
    p.connecting = true;
    try {
      p.sessionId = await sessionFor(source.hostId);
    } catch (e) {
      p.error = errorText(e);
      p.connecting = false;
      releaseUnused();
      return;
    }
    p.connecting = false;
    releaseUnused();
    await open(side);
  }

  async function list(p: Pane, path?: string): Promise<{ path: string; entries: FileEntry[] }> {
    if (p.source?.kind === "local") {
      const target = path ?? (await localFsApi.home());
      return { path: target, entries: await localFsApi.list(target) };
    }
    const id = p.sessionId!;
    const target = path ?? (await sftpApi.home(id));
    return { path: target, entries: await sftpApi.list(id, target) };
  }

  async function open(side: Side, path?: string) {
    const p = pane(side);
    if (!p.source) return;
    p.loading = true;
    p.error = null;
    try {
      const res = await list(p, path);
      p.entries = res.entries;
      p.path = res.path;
      p.selected = null;
      p.picked = [];
    } catch (e) {
      p.error = errorText(e);
    } finally {
      p.loading = false;
    }
  }

  /** Двойной щелчок: папка — открыть, ссылка — открыть, если она на папку. */
  async function enter(side: Side, entry: FileEntry): Promise<boolean> {
    pane(side).opError = null;
    if (entry.kind === "dir") {
      await open(side, entry.path);
      return true;
    }
    if (entry.kind !== "symlink") return false;
    const p = pane(side);
    try {
      const res = await list(p, entry.path);
      p.entries = res.entries;
      p.path = res.path;
      p.selected = null;
      p.picked = [];
      p.error = null;
      return true;
    } catch {
      return false;
    }
  }

  const refresh = (side: Side) => open(side, pane(side).path || undefined);

  async function up(side: Side) {
    const p = pane(side);
    if (!p.path) return;
    p.opError = null;
    const parent =
      p.source?.kind === "local"
        ? await localFsApi.parent(p.path)
        : await sftpApi.parent(p.path);
    if (parent !== p.path) await open(side, parent);
  }

  /** Обёртка правок: показывает ошибку над списком и обновляет его. `false` — не вышло. */
  async function edit(side: Side, action: () => Promise<unknown>): Promise<boolean> {
    const p = pane(side);
    p.error = null;
    p.opError = null;
    let ok = true;
    try {
      await action();
    } catch (e) {
      p.opError = errorText(e);
      ok = false;
    }
    await refresh(side);
    return ok;
  }

  const mkdir = (side: Side, name: string) =>
    edit(side, () => {
      const p = pane(side);
      return p.source?.kind === "local"
        ? localFsApi.mkdir(p.path, name)
        : sftpApi.mkdir(p.sessionId!, joinRemote(p.path, name));
    });

  const rename = (side: Side, entry: FileEntry, newName: string) =>
    edit(side, () => {
      const p = pane(side);
      if (p.source?.kind === "local") return localFsApi.rename(entry.path, newName);
      const dir = entry.path.slice(0, entry.path.lastIndexOf("/")) || "/";
      return sftpApi.rename(p.sessionId!, entry.path, joinRemote(dir, newName));
    });

  const remove = (side: Side, entry: FileEntry) =>
    edit(side, () => {
      const p = pane(side);
      const isDir = entry.kind === "dir";
      return p.source?.kind === "local"
        ? localFsApi.remove(entry.path, isDir)
        : sftpApi.remove(p.sessionId!, entry.path, isDir);
    });

  const endpoint = (p: Pane): TreeEndpoint =>
    p.source?.kind === "local" ? { kind: "local" } : { kind: "remote", sessionId: p.sessionId! };

  /**
   * Можно ли отправить выбранный файл или папку `from` в папку соседней панели.
   * Годится всё, кроме «компьютер → компьютер»: для него есть файловый менеджер.
   */
  const transferable = (e: FileEntry) => e.kind === "file" || e.kind === "dir";

  /** Что передавать: переданные явно или выбранные в панели. */
  function itemsOf(from: Side, entries?: FileEntry[]): FileEntry[] {
    const a = pane(from);
    const items = entries ?? (a.picked.length ? a.picked : a.selected ? [a.selected] : []);
    return items.filter(transferable);
  }

  function canTransfer(from: Side, entries?: FileEntry[]): boolean {
    const a = pane(from);
    const b = pane(other(from));
    if (itemsOf(from, entries).length === 0) return false;
    if (!a.source || !b.source) return false;
    if (a.source.kind === "local" && b.source.kind === "local") return false;
    return !!b.path && !a.loading && !b.loading;
  }

  /**
   * Передать выбранное в папку соседней панели. Если там уже есть такие
   * имена — сначала спросить: заменить, оставить оба или пропустить.
   */
  async function transfer(from: Side, entries?: FileEntry[]) {
    const a = pane(from);
    const b = pane(other(from));
    if (!canTransfer(from, entries)) return;
    const items = itemsOf(from, entries);
    const plan = await planTransfers(
      items,
      b.entries.map((e) => e.name),
      useConflictsStore().ask,
    );
    b.opError = null;
    for (const { item, onConflict } of plan) {
      try {
        await useTransfersStore().startTree(
          endpoint(a),
          item.path,
          item.kind === "dir",
          endpoint(b),
          b.path,
          onConflict,
        );
      } catch (e) {
        b.opError = errorText(e);
      }
    }
  }

  /** Щелчок по записи: одна, Ctrl/⌘ — добавить, Shift — диапазон по видимому списку. */
  function pick(
    side: Side,
    entry: FileEntry,
    mods: { toggle: boolean; range: boolean },
    visible: FileEntry[],
  ) {
    const p = pane(side);
    const next = nextSelection(visible, { picked: p.picked, anchor: p.selected }, entry, mods);
    p.picked = next.picked;
    p.selected = next.anchor;
  }

  /** Передача закончилась — списки могли измениться. */
  function refreshBoth() {
    void refresh("left");
    void refresh("right");
  }

  return {
    left,
    right,
    dragging,
    pane,
    init,
    setSource,
    open,
    enter,
    refresh,
    refreshBoth,
    up,
    mkdir,
    rename,
    remove,
    canTransfer,
    transfer,
    pick,
  };
});
