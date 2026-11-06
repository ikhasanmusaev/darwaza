import { defineStore } from "pinia";
import { ref, computed, shallowRef } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { sshApi, type SessionStatus } from "../api/ssh";
import { listenSessionEvents } from "../lib/eventBus";
import { useKnownHostsStore } from "./known_hosts";
import { useSftpStore } from "./sftp";
import { useRecordingsStore } from "./recordings";
import { useSettingsStore } from "./settings";
import { useHostsStore } from "./hosts";
import { useBookmarksStore } from "./bookmarks";
import { useLayoutStore } from "./layout";
import { useAuthPromptStore } from "./auth_prompt";

export interface SessionTab {
  /** UUID от backend. */
  sessionId: string;
  hostId: number;
  /** Снимок имени хоста на момент открытия — чтобы вкладка не "терялась" при переименовании. */
  hostLabel: string;
  /** Имя, которое человек дал вкладке (только вкладке: хост не меняется). */
  title?: string;
  status: SessionStatus;
  /** Текст ошибки если FAILED. */
  error: string | null;
  /** Открыто (ms epoch) для сортировки/восстановления. */
  openedAt: number;
}

/**
 * Подписчик на `session:data` для конкретной сессии.
 * Терминал-компонент (`TerminalPane.vue`) регистрирует свой колбэк
 * на свой sessionId, чтобы получать чанки и писать их в xterm.
 */
type DataListener = (chunk: Uint8Array) => void;

function errMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return String(e);
}

export const useSessionsStore = defineStore("sessions", () => {
  const sessions = ref<SessionTab[]>([]);
  const activeSessionId = ref<string | null>(null);
  const unread = ref<Record<string, number>>({});

  // Карта sessionId → колбэк терминала. shallowRef — потому что Uint8Array не нужен reactive.
  const dataListeners = shallowRef(new Map<string, DataListener>());

  let unsubscribe: UnlistenFn | null = null;

  const activeSession = computed(() =>
    sessions.value.find((s) => s.sessionId === activeSessionId.value) ?? null,
  );

  function findIndex(sessionId: string): number {
    return sessions.value.findIndex((s) => s.sessionId === sessionId);
  }

  function patch(sessionId: string, patch: Partial<SessionTab>) {
    const i = findIndex(sessionId);
    if (i < 0) return;
    sessions.value[i] = { ...sessions.value[i], ...patch };
  }

  /** Зарегистрировать колбэк терминала на чанки от backend. Возвращает функцию-снятие. */
  function registerDataListener(sessionId: string, cb: DataListener): () => void {
    dataListeners.value.set(sessionId, cb);
    return () => {
      dataListeners.value.delete(sessionId);
    };
  }

  /** Инициализация подписок (вызвать один раз при старте app). */
  async function init() {
    if (unsubscribe) return;
    const knownHosts = useKnownHostsStore();
    const authPrompt = useAuthPromptStore();
    unsubscribe = await listenSessionEvents({
      onStatus: ({ sessionId, status, error }) => {
        patch(sessionId, { status, error: error ?? null });
      },
      onData: ({ sessionId, chunk }) => {
        const cb = dataListeners.value.get(sessionId);
        if (cb) cb(Uint8Array.from(chunk));
        if (sessionId !== activeSessionId.value) {
          unread.value[sessionId] = (unread.value[sessionId] ?? 0) + 1;
        }
      },
      onClosed: ({ sessionId, exitCode }) => {
        // Сбой закрывает сессию, и backend шлёт `closed` следом за `FAILED`.
        // Затирать им причину нельзя: человек увидел бы «Отключено» без
        // единого слова о том, что случилось, — а причина уже была известна.
        // Ровно так пропадала «authentication failed» (2026-08-28).
        const known = sessions.value.find((s) => s.sessionId === sessionId);
        if (known?.status === "FAILED") return;

        patch(sessionId, {
          status: "CLOSED",
          error:
            exitCode !== undefined && exitCode !== 0
              ? `exit ${exitCode}`
              : null,
        });

        // Сессия кончилась — вкладке нечего показывать (#209).
        //
        // `exit` в оболочке, `logout`, обрыв связи: терминала за вкладкой
        // больше нет, и человек всё равно закроет её руками. Так делают все
        // терминалы, и ожидание именно такое.
        //
        // Вкладку со сбоем НЕ трогаем — она выше уже отсеяна возвратом при
        // `FAILED`: там на экране причина, ради которой её и держат. Ровно
        // так пропадала «authentication failed» (2026-08-28), второй раз
        // этой ошибки быть не должно.
        forget(sessionId);
      },
      onHostKeyPrompt: (p) => {
        knownHosts.enqueuePrompt(p);
      },
      onAuthPrompt: (p) => {
        authPrompt.enqueue(p);
      },
    });
  }

  /** Открыть сессию для хоста: вызывает backend, добавляет вкладку, делает её активной. */
  async function open(hostId: number, hostLabel: string): Promise<string> {
    const { sessionId } = await sshApi.connect(hostId);
    const tab: SessionTab = {
      sessionId,
      hostId,
      hostLabel,
      status: "CONNECTING",
      error: null,
      openedAt: Date.now(),
    };
    sessions.value = [...sessions.value, tab];
    activeSessionId.value = sessionId;
    rememberOpenTabs();
    return sessionId;
  }


  /**
   * Ключ, под которым помнятся открытые вкладки (#145).
   *
   * Хранятся именно номера хостов, а не идентификаторы сессий: сессия живёт в
   * памяти backend и после перезапуска её нет ни у кого. Восстанавливать
   * нечего — можно только подключиться заново к тем же машинам.
   */
  const OPEN_TABS_KEY = "session.open_hosts";

  /**
   * Запомнить, какие хосты сейчас открыты.
   *
   * Ошибка записи намеренно проглатывается: не сохранившийся список вкладок —
   * досадно, но мешать работе с серверами он не должен.
   */
  function rememberOpenTabs() {
    const ids = sessions.value.map((s) => s.hostId);
    void useSettingsStore()
      .set(OPEN_TABS_KEY, JSON.stringify(ids))
      .catch(() => undefined);
  }

  /**
   * Убрать вкладку и всё, что к ней привязано, ничего не спрашивая у backend.
   *
   * Одна уборка на два случая: `close()` зовёт её после отключения живой
   * сессии, а событие о конце сессии — сразу. Двух копий здесь быть не
   * должно: разъехавшись, они оставят за собой то пустую панель в раскладке,
   * то закладки на буфер, которого нет, — и заметит это только человек.
   */
  function forget(sessionId: string) {
    if (!sessions.value.some((s) => s.sessionId === sessionId)) return;

    sessions.value = sessions.value.filter((s) => s.sessionId !== sessionId);
    delete unread.value[sessionId];
    // Состояние файлового браузера привязано к сессии: SFTP-канал жил поверх
    // неё и на backend уже выброшен — держать путь и листинг незачем.
    useSftpStore().forget(sessionId);
    // Запись backend закрыл сам при завершении сессии — снимаем отметку,
    // иначе кнопка так и осталась бы в состоянии «идёт запись».
    useRecordingsStore().forget(sessionId);
    // Закладки указывают на строки буфера прокрутки, которого больше нет.
    useBookmarksStore().forget(sessionId);
    // Панель закрытой сессии должна исчезнуть из раскладки, иначе на её месте
    // останется пустой прямоугольник.
    useLayoutStore().forget(sessionId);

    if (activeSessionId.value === sessionId) {
      activeSessionId.value =
        sessions.value[sessions.value.length - 1]?.sessionId ?? null;
    }
    rememberOpenTabs();
  }

  /**
   * Закрыть сессию: дёргает backend (для активных) и удаляет вкладку.
   * Для уже CLOSED/FAILED — просто удаляет вкладку без backend-вызова.
   */
  async function close(sessionId: string): Promise<void> {
    const tab = sessions.value.find((s) => s.sessionId === sessionId);
    if (!tab) return;
    const isLive = tab.status !== "CLOSED" && tab.status !== "FAILED";
    if (isLive) {
      try {
        await sshApi.disconnect(sessionId);
      } catch (e) {
        // Сессия могла уже отвалиться сама — это норма.
        console.warn("disconnect failed:", errMessage(e));
      }
    }
    forget(sessionId);
  }


  /**
   * Дождаться, пока подключение определится: встало, упало или закрылось.
   *
   * `ssh_connect` возвращается сразу, а само подключение — рукопожатие, вопрос
   * про отпечаток, запрос пароля — идёт в фоне. Поэтому «открыл и перешёл к
   * следующему» запускало все подключения разом, вопреки замыслу #145 (#234).
   * Спрашиваем у backend, а не слушаем события: так не зависим от того,
   * успели ли подписаться, и сессию, которой в списке уже нет, считаем
   * определившейся.
   *
   * Предел щедрый — человек может не сразу ответить на вопрос о пароле, — но
   * конечный: не отвеченный вопрос не должен навсегда оставить остальные
   * вкладки закрытыми.
   */
  async function settled(sessionId: string, limitMs = 120_000): Promise<void> {
    const pending = new Set<SessionStatus>(["CONNECTING", "AUTHENTICATING", "PTY_OPENING"]);
    const deadline = Date.now() + limitMs;
    while (Date.now() < deadline) {
      let snapshot;
      try {
        snapshot = await sshApi.activeSessions();
      } catch {
        return;
      }
      const mine = snapshot.find((s) => s.sessionId === sessionId);
      if (!mine || !pending.has(mine.status)) return;
      await new Promise((resolve) => setTimeout(resolve, 300));
    }
  }

  /**
   * Открыть заново вкладки, которые были при прошлом запуске (#145).
   *
   * Подключения устанавливаются **по очереди**, а не разом: десять
   * одновременных рукопожатий с запросами пароля и подтверждением ключа
   * превращают запуск в допрос. Последовательно человек отвечает на них по
   * одному, как если бы открывал вкладки сам.
   *
   * Ошибка одного хоста не мешает остальным: сервера выключают, и не самый
   * важный из них не должен отменять восстановление всех.
   */
  async function restoreOpenTabs(): Promise<void> {
    if (sessions.value.length > 0) return;

    const settings = useSettingsStore();
    await settings.ensureLoaded();
    if (settings.get("session.restore_tabs") === "false") return;

    let ids: unknown;
    try {
      ids = JSON.parse(settings.get(OPEN_TABS_KEY) ?? "[]");
    } catch {
      // Список испорчен — не повод падать при запуске.
      return;
    }
    if (!Array.isArray(ids) || ids.length === 0) return;

    // Имя хоста нужно для подписи вкладки, и брать его надо из списка, а не
    // из запомненного: хост могли переименовать между запусками.
    const hosts = useHostsStore();
    await hosts.ensureLoaded();

    for (const hostId of ids) {
      const host = hosts.hosts.find((h) => h.id === hostId);
      // Хост удалили — восстанавливать нечего, и это не ошибка.
      if (!host) continue;
      try {
        const sessionId = await open(host.id, host.name);
        // Следующий хост — только когда этот определился: вопросы приходят по
        // одному, как при открытии вкладок вручную.
        await settled(sessionId);
      } catch {
        // Сервер мог быть выключен: не самый важный из них не должен
        // отменять восстановление остальных.
      }
    }
  }

  /** Послать stdin в активную сессию. Используется TerminalPane.vue. */
  async function send(sessionId: string, data: Uint8Array): Promise<void> {
    await sshApi.send(sessionId, data);
  }

  /** Сообщить backend о новых cols/rows PTY. */
  async function resize(sessionId: string, cols: number, rows: number): Promise<void> {
    await sshApi.resize(sessionId, cols, rows);
  }

  /**
   * Разделить активную панель: новое подключение к тому же хосту ложится рядом.
   * `false` — делить нечего (нет активной сессии) или панелей уже максимум.
   */
  async function splitActive(dir: "row" | "column"): Promise<boolean> {
    const current = activeSession.value;
    const layout = useLayoutStore();
    if (!current) return false;
    if (!layout.isSplit ? false : !layout.canSplit) return false;
    const id = await open(current.hostId, current.hostLabel);
    return layout.splitPane(current.sessionId, id, dir);
  }

  /** Новое подключение к тому же хосту в новой вкладке. */
  async function duplicate(sessionId: string): Promise<string | null> {
    const tab = sessions.value.find((s) => s.sessionId === sessionId);
    if (!tab) return null;
    const id = await open(tab.hostId, tab.hostLabel);
    const title = tab.title;
    if (title) patch(id, { title });
    return id;
  }

  /** Переименовать вкладку. Пустое имя возвращает имя хоста. */
  function rename(sessionId: string, title: string) {
    const t = title.trim();
    patch(sessionId, { title: t === "" ? undefined : t });
  }

  /** Подключиться заново к хосту оборвавшейся сессии, старую вкладку убрать. */
  async function reconnect(sessionId: string): Promise<string | null> {
    const tab = sessions.value.find((s) => s.sessionId === sessionId);
    if (!tab || (tab.status !== "FAILED" && tab.status !== "CLOSED")) return null;
    const id = await open(tab.hostId, tab.hostLabel);
    if (tab.title) patch(id, { title: tab.title });
    forget(sessionId);
    setActive(id);
    return id;
  }

  /** Закрыть все вкладки, кроме `keepId` (или все, если `null`). */
  async function closeOthers(keepId: string | null) {
    for (const s of [...sessions.value]) {
      if (s.sessionId !== keepId) await close(s.sessionId);
    }
  }

  /** Закрыть вкладки правее `sessionId`. */
  async function closeToRight(sessionId: string) {
    const i = findIndex(sessionId);
    if (i < 0) return;
    for (const s of sessions.value.slice(i + 1)) await close(s.sessionId);
  }

  function setActive(sessionId: string | null) {
    activeSessionId.value = sessionId;
    if (sessionId) {
      delete unread.value[sessionId];
    }
  }

  function reorder(sessionId: string, toIndex: number) {
    const i = sessions.value.findIndex((s) => s.sessionId === sessionId);
    if (i < 0 || toIndex < 0 || toIndex >= sessions.value.length) return;
    const next = [...sessions.value];
    const [removed] = next.splice(i, 1);
    next.splice(toIndex, 0, removed);
    sessions.value = next;
  }

  return {
    restoreOpenTabs,
    sessions,
    activeSessionId,
    activeSession,
    unread,
    init,
    open,
    close,
    send,
    resize,
    setActive,
    splitActive,
    duplicate,
    rename,
    reconnect,
    closeOthers,
    closeToRight,
    reorder,
    registerDataListener,
  };
});
