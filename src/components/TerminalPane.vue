<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, watch } from "vue";
import { useI18n } from "vue-i18n";
import { Terminal } from "xterm";
import { FitAddon } from "xterm-addon-fit";
import { WebLinksAddon } from "xterm-addon-web-links";
import { SearchAddon } from "xterm-addon-search";
import {
  Loader2,
  AlertTriangle,
  RotateCw,
  Search,
  ChevronDown,
  ChevronUp,
  X,
  Bookmark as BookmarkIcon,
  Trash2,
} from "lucide-vue-next";
import { errorText } from "../lib/errors";
import { readClipboardImage, readClipboardText, writeClipboardText } from "../api/clipboard";
import { sftpApi } from "../api/sftp";
import { parseOsc52 } from "../lib/osc52";
import { useSessionsStore } from "../stores/sessions";
import { useSettingsStore } from "../stores/settings";
import { useBookmarksStore } from "../stores/bookmarks";
import { getTheme } from "../lib/terminal-themes";
import { isAppMod, isMacOS } from "../lib/platform";
import "xterm/css/xterm.css";

const props = defineProps<{ sessionId: string }>();

const { t } = useI18n();
const store = useSessionsStore();
const settings = useSettingsStore();
const bookmarks = useBookmarksStore();

const container = ref<HTMLDivElement | null>(null);
const reconnecting = ref(false);
const showSearch = ref(false);
const searchQuery = ref("");
/** Что происходит с картинкой из буфера: идёт передача или она не удалась. */
const imagePaste = ref<"uploading" | "failed" | null>(null);

const session = computed(() => store.sessions.find((s) => s.sessionId === props.sessionId) ?? null);

const status = computed(() => session.value?.status ?? null);
/**
 * Причина, по которой сессия не работает, — на языке человека (#178).
 *
 * Backend отдаёт код (`errors.sshRefused`), а не текст: раньше сюда попадала
 * английская строка из движка SSH, и человек с «connect to … failed: Connection
 * refused» не мог понять даже, чинить ему сеть или пароль.
 */
const error = computed(() => {
  const raw = session.value?.error;
  return raw ? errorText(raw) : null;
});
const isInteractive = computed(() => status.value === "CONNECTED");
const showOverlay = computed(() => {
  return (
    status.value === "CONNECTING" ||
    status.value === "AUTHENTICATING" ||
    status.value === "PTY_OPENING" ||
    status.value === "DISCONNECTING" ||
    status.value === "FAILED" ||
    status.value === "CLOSED"
  );
});

/* ---- Закладки в выводе (#143) ---- */

/**
 * Поставить закладку на текущую строку.
 *
 * Метка по умолчанию — сам текст строки: он почти всегда информативнее
 * номера («$ systemctl status nginx» против «#1423»).
 */
function addBookmark() {
  if (!term) return;
  const buf = term.buffer.active;
  const cursorLine = buf.baseY + buf.cursorY;

  // Сразу после вывода курсор стоит на пустой строке под ним, и закладка
  // получила бы пустую метку. Отступаем вверх до ближайшей непустой строки —
  // пользователь имеет в виду именно её.
  const MAX_LOOKBACK = 50;
  let line = cursorLine;
  let text = "";
  for (let i = 0; i < MAX_LOOKBACK && line >= 0; i++) {
    const candidate = buf.getLine(line)?.translateToString(true).trim() ?? "";
    if (candidate) {
      text = candidate;
      break;
    }
    line -= 1;
  }
  // Если выше ничего нет (пустой буфер) — ставим на текущую строку.
  if (!text) line = cursorLine;

  bookmarks.add(props.sessionId, line, text);
  bookmarks.panelOpen = true;
}

function scrollToLine(line: number) {
  // scrollToLine принимает абсолютный номер строки в буфере — именно его
  // мы и храним, поэтому пересчёт не нужен.
  term?.scrollToLine(line);
  term?.focus();
}

let unregisterScroller: (() => void) | null = null;
let unregisterAdder: (() => void) | null = null;

let term: Terminal | null = null;
let fitAddon: FitAddon | null = null;
let searchAddon: SearchAddon | null = null;
let unregister: (() => void) | null = null;
let resizeObserver: ResizeObserver | null = null;
let appThemeObserver: MutationObserver | null = null;
const encoder = new TextEncoder();

function fitAndPushResize() {
  if (!fitAddon || !term) return;
  fitAddon.fit();
  // term.cols/rows обновляются синхронно после fit()
  store.resize(props.sessionId, term.cols, term.rows).catch(() => {
    /* во время CONNECTING сессия ещё не готова — игнорируем */
  });
}

const termFontFamily = computed(
  () =>
    settings.get("terminal.font.family") ??
    "JetBrains Mono, Fira Code, ui-monospace, SFMono-Regular, Menlo, Monaco, monospace",
);
const termFontSize = computed(() => parseInt(settings.get("terminal.font.size") ?? "13", 10));
const termThemeName = computed(() => settings.get("terminal.theme") ?? "dark-default");
const appAccent = ref("");
const termTheme = computed(() => getTheme(termThemeName.value, appAccent.value || undefined));

function readAppAccent() {
  appAccent.value = getComputedStyle(document.documentElement)
    .getPropertyValue("--c-accent")
    .trim();
}

function buildTerminal(): Terminal {
  const t = new Terminal({
    fontFamily: termFontFamily.value,
    fontSize: termFontSize.value,
    cursorBlink: true,
    allowProposedApi: true,
    scrollback: 5000,
    theme: termTheme.value,
    // На macOS Option обязан работать как Meta. Без этого Option+b и Option+f
    // («слово назад», «слово вперёд» в оболочке) отдают в терминал спецсимвол
    // вместо ESC-последовательности, и перемещение по словам не работает.
    // Платой за это становится набор спецсимволов через Option (например,
    // Option+e для «é») — в терминале он перестаёт работать.
    macOptionIsMeta: isMacOS(),
  });
  fitAddon = new FitAddon();
  searchAddon = new SearchAddon();
  t.loadAddon(fitAddon);
  t.loadAddon(new WebLinksAddon());
  t.loadAddon(searchAddon);
  return t;
}

onMounted(() => {
  if (!container.value) return;
  readAppAccent();
  appThemeObserver = new MutationObserver(readAppAccent);
  appThemeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["class"],
  });
  term = buildTerminal();
  term.open(container.value);
  fitAndPushResize();

  unregisterScroller = bookmarks.registerScroller(props.sessionId, scrollToLine);
  unregisterAdder = bookmarks.registerAdder(props.sessionId, addBookmark);

  // Stream from backend
  unregister = store.registerDataListener(props.sessionId, (chunk) => {
    term?.write(chunk);
  });

  // Stdin
  term.onData((data) => {
    if (!isInteractive.value) return;
    store.send(props.sessionId, encoder.encode(data)).catch(() => {
      /* при FAILED/CLOSED сессия мертва — глушим */
    });
  });

  /** Скопировать выделенное. `false`, если копировать нечего. */
  function copySelection(): boolean {
    const sel = term?.getSelection();
    if (!sel) return false;
    void writeClipboardText(sel);
    // Выделение снимаем: скопированное остаётся подсвеченным и мешает читать
    // вывод, а человек уже считает копирование законченным.
    term?.clearSelection();
    return true;
  }

  /**
   * Вставка выполняется в один шаг и одним путём: читаем буфер сами и отдаём
   * `term.paste`, который сам оборачивает текст в скобочную вставку. Браузерное
   * событие paste в это время гасим: в WebKitGTK оно то не приходит на
   * Ctrl+Shift+V вовсе, то приходит ещё раз — и вставка либо пропадает, либо
   * двоится. Окно подавления короткое, чтобы не задеть настоящую вставку
   * (правая кнопка мыши, средняя кнопка) сразу после.
   */
  let suppressNativePasteUntil = 0;
  term.textarea?.addEventListener(
    "paste",
    (ev) => {
      if (performance.now() < suppressNativePasteUntil) {
        ev.preventDefault();
        ev.stopImmediatePropagation();
      }
    },
    true,
  );

  function pasteFromClipboard() {
    if (!isInteractive.value) return;
    suppressNativePasteUntil = performance.now() + 400;
    void readClipboardText().then((txt) => {
      if (txt) term?.paste(txt);
    });
  }

  /**
   * OSC 52: программа на сервере просит записать текст в буфер. Без этого
   * копирование в Claude Code на весь экран, tmux и vim с мышью молча пропадало.
   * Пишем, только пока этот терминал в фокусе: фоновая вкладка или свёрнутое окно
   * не должны подменять то, что человек скопировал сам.
   */
  term.parser.registerOscHandler(52, (data) => {
    const text = parseOsc52(data);
    if (text !== null && document.hasFocus() && term?.textarea === document.activeElement) {
      void writeClipboardText(text);
    }
    return true;
  });

  /**
   * Голый Ctrl+V. В буфере картинка: кладём её на сервер и вставляем путь,
   * потому что программа на сервере видит только буфер сервера. Иначе уходит ^V,
   * как и раньше: текст этим сочетанием не вставляем (решение от 2026-11-02).
   */
  async function handleCtrlV() {
    const image = await readClipboardImage();
    if (!isInteractive.value) return;
    if (!image) {
      store.send(props.sessionId, encoder.encode("\x16")).catch(() => undefined);
      return;
    }
    imagePaste.value = "uploading";
    try {
      term?.paste(await sftpApi.putClipboardImage(props.sessionId, image));
      imagePaste.value = null;
    } catch (e) {
      console.warn("картинка из буфера не передана:", e);
      imagePaste.value = "failed";
      window.setTimeout(() => {
        if (imagePaste.value === "failed") imagePaste.value = null;
      }, 6000);
    }
  }

  // Copy/paste keyboard (xterm.js v5 пропускает их через customHandlers)
  term.attachCustomKeyEventHandler((e) => {
    if (e.type !== "keydown") return true;

    // Сочетания привязаны к физической клавише (`e.code`), а не к букве
    // (`e.key`): при кириллической раскладке `key` — «ц», «м», «а», и привязка
    // к латинице молча не срабатывала. Обычный ввод не затронут: везде нужен Ctrl.

    // macOS: Cmd+C копирует выделенное, как во всей системе.
    //
    // Само по себе копирование здесь не случилось бы: xterm держит выделение
    // в своей модели, а не в выделении документа, поэтому системное
    // «Копировать» в терминале копировать нечего и нажатие доходит до нас.
    //
    // При пустом выделении нажатие НЕ перехватываем: оно должно уйти дальше,
    // иначе системное копирование в полях ввода сломается.
    //
    // Cmd+V не перехватываем сознательно: вставку делает сам WebView, а xterm
    // подхватывает её событием paste — чистит скрытую textarea и шлёт текст
    // один раз, в скобочной вставке. Своего кода для этого не нужно, и лишний
    // перехват сломал бы вставку правой кнопкой и из меню приложения.
    if (isMacOS() && e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && e.code === "KeyC") {
      if (!copySelection()) return true;
      e.preventDefault();
      e.stopPropagation();
      return false;
    }

    // Голый Ctrl+C не перехватывается — решение владельца (2026-08-31): он
    // обязан прерывать команду, и подменять его копией значит отнимать у
    // человека единственный способ остановить зависший процесс. Копия и
    // вставка живут на Ctrl+Shift+C и Ctrl+Shift+V — так принято в терминалах.
    //
    // Голый Ctrl+V (решение владельца, 2026-11-02) текст не вставляет: в
    // оболочку уходит ^V (0x16, «quoted-insert»); исключение — картинка в буфере,
    // см. handleCtrlV. WebKitGTK при этом запускал ещё и собственную вставку,
    // поэтому её гасим — как и у Ctrl+Shift+V.
    if (e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey && e.code === "KeyV") {
      e.preventDefault();
      e.stopPropagation();
      suppressNativePasteUntil = performance.now() + 400;
      if (isInteractive.value) void handleCtrlV();
      return false;
    }

    // Ctrl+W (⌘W на Mac) закрывает вкладку и до оболочки доходить не должен:
    // там это «стереть слово», и человек терял бы слово в набранной команде.
    // На Mac голый Ctrl+W, наоборот, уходит в оболочку — вкладку закрывает ⌘W.
    if (isAppMod(e) && !e.shiftKey && !e.altKey && e.code === "KeyW") {
      return false;
    }

    // Ctrl+Insert — вторая стандартная пара копирования. Без этого xterm
    // отправил бы в оболочку последовательность Insert, а выделение не
    // скопировалось бы. Shift+Insert (вставка) — ниже, вместе с Ctrl+Shift+V.
    if (e.ctrlKey && !e.shiftKey && !e.altKey && e.code === "Insert") {
      e.preventDefault();
      copySelection();
      return false;
    }

    // Ctrl+Shift+C — копия выделенного
    if (e.ctrlKey && e.shiftKey && e.code === "KeyC") {
      copySelection();
      return false;
    }
    // Ctrl+Shift+V и Shift+Insert — вставка, одним путём (см. pasteFromClipboard).
    // preventDefault гасит ещё и собственную вставку браузера.
    if (
      (e.ctrlKey && e.shiftKey && !e.altKey && e.code === "KeyV") ||
      (e.shiftKey && !e.ctrlKey && !e.altKey && e.code === "Insert")
    ) {
      e.preventDefault();
      e.stopPropagation();
      pasteFromClipboard();
      return false;
    }
    // Ctrl+Shift+B (⇧⌘B) — поставить закладку на текущей строке
    if (isAppMod(e) && e.shiftKey && e.code === "KeyB") {
      addBookmark();
      return false;
    }

    // Ctrl+F (⌘F на Mac, где Ctrl+F — «вперёд на символ» в оболочке) — поиск
    if (isAppMod(e) && e.code === "KeyF") {
      openSearch();
      return false;
    }
    return true;
  });

  // Resize по контейнеру
  resizeObserver = new ResizeObserver(() => fitAndPushResize());
  resizeObserver.observe(container.value);

  // Дать терминалу фокус
  term.focus();
});

onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  appThemeObserver?.disconnect();
  unregister?.();
  unregisterScroller?.();
  unregisterAdder?.();
  term?.dispose();
  term = null;
  fitAddon = null;
  searchAddon = null;
  unregisterScroller = null;
  unregisterAdder = null;
});

// При смене активной сессии (если компонент переиспользуется) — на всякий случай ремонтируем.
// Сейчас мы используем :key="sessionId" в родителе, так что компонент пересоздаётся.
watch(
  () => props.sessionId,
  () => {
    /* родитель должен передать новый sessionId через :key */
  },
);

function openSearch() {
  showSearch.value = true;
  // Фокус на input в следующем тике, после рендера
  requestAnimationFrame(() => {
    document.getElementById("terminal-search-input")?.focus();
  });
}

function closeSearch() {
  showSearch.value = false;
  searchQuery.value = "";
  searchAddon?.clearDecorations();
  term?.focus();
}

function findNext() {
  if (!searchAddon || !searchQuery.value) return;
  searchAddon.findNext(searchQuery.value);
}

function findPrevious() {
  if (!searchAddon || !searchQuery.value) return;
  searchAddon.findPrevious(searchQuery.value);
}

// Live update terminal options when settings change
watch(termFontFamily, (v) => {
  if (term) term.options.fontFamily = v;
});
watch(termFontSize, (v) => {
  if (term) {
    term.options.fontSize = v;
    fitAndPushResize();
  }
});
watch(termTheme, (v) => {
  if (term) term.options.theme = v;
});

async function reconnect() {
  if (!session.value) return;
  reconnecting.value = true;
  try {
    // Открываем новую сессию к тому же хосту, закрываем старую вкладку.
    const oldId = props.sessionId;
    const hostId = session.value.hostId;
    const label = session.value.hostLabel;
    await store.open(hostId, label);
    await store.close(oldId);
  } finally {
    reconnecting.value = false;
  }
}
</script>

<template>
  <div class="relative h-full w-full bg-surface">
    <div
      v-if="imagePaste"
      data-testid="image-paste-notice"
      role="status"
      class="absolute top-2 left-1/2 -translate-x-1/2 z-20 px-3 py-1.5 text-xs rounded border border-edge bg-surface-card"
      :class="imagePaste === 'failed' ? 'text-danger-text' : 'text-on-surface'"
    >
      {{ imagePaste === "failed" ? t("terminalClipboard.imageFailed") : t("terminalClipboard.imageUploading") }}
    </div>
    <!-- Закладки (#143) -->
    <div
      v-if="bookmarks.panelOpen && bookmarks.list(props.sessionId).length > 0"
      data-testid="bookmarks-panel"
      class="absolute top-2 left-2 z-20 w-72 max-h-64 overflow-auto bg-surface-card border border-edge rounded shadow-lg"
    >
      <div class="flex items-center gap-2 px-2 py-1.5 border-b border-edge">
        <BookmarkIcon :size="13" class="text-warning" />
        <span class="text-xs font-medium text-on-surface">{{ t("bookmarks.title") }}</span>
        <button
          type="button"
          class="ml-auto p-1 rounded hover:bg-surface-alt text-on-surface-muted"
          :title="t('form.cancel')"
          @click="bookmarks.panelOpen = false"
        >
          <X :size="13" />
        </button>
      </div>
      <button
        v-for="b in bookmarks.list(props.sessionId)"
        :key="b.id"
        type="button"
        class="w-full group flex items-center gap-2 px-2 py-1 text-left hover:bg-surface-alt"
        @click="bookmarks.jump(props.sessionId, b.line)"
      >
        <span class="text-[10px] text-on-surface-muted shrink-0 w-10 text-right">
          {{ b.line }}
        </span>
        <span class="text-xs text-on-surface-muted truncate flex-1">{{ b.label }}</span>
        <span
          role="button"
          tabindex="0"
          class="p-0.5 rounded text-on-surface-muted hover:text-danger-text opacity-0 group-hover:opacity-100"
          :title="t('hosts.delete')"
          @click.stop="bookmarks.remove(props.sessionId, b.id)"
          @keydown.enter.stop="bookmarks.remove(props.sessionId, b.id)"
        >
          <Trash2 :size="12" />
        </span>
      </button>
    </div>

    <!-- Search bar -->
    <div
      v-if="showSearch"
      class="absolute top-2 right-2 z-20 flex items-center gap-2 bg-surface-alt border border-edge rounded px-2 py-1.5 shadow-lg"
    >
      <Search :size="14" class="text-on-surface-muted" />
      <input
        id="terminal-search-input"
        v-model="searchQuery"
        type="text"
        class="bg-transparent text-on-surface text-sm focus:outline-none w-40"
        :placeholder="t('terminalSearch.placeholder')"
        @keydown.enter.prevent="findNext"
        @keydown.shift.enter.prevent="findPrevious"
        @keydown.esc.prevent="closeSearch"
      />
      <button
        type="button"
        class="text-on-surface-muted hover:text-on-surface"
        :title="t('terminalSearch.prev')"
        @click="findPrevious"
      >
        <ChevronUp :size="14" />
      </button>
      <button
        type="button"
        class="text-on-surface-muted hover:text-on-surface"
        :title="t('terminalSearch.next')"
        @click="findNext"
      >
        <ChevronDown :size="14" />
      </button>
      <button
        type="button"
        class="text-on-surface-muted hover:text-on-surface"
        :title="t('terminalSearch.close')"
        @click="closeSearch"
      >
        <X :size="14" />
      </button>
    </div>

    <div class="h-full w-full px-6 pt-4">
      <div ref="container" class="h-full w-full" />
    </div>

    <!-- Status overlay -->
    <div
      v-if="showOverlay"
      class="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-scrim"
    >
      <template
        v-if="
          status === 'CONNECTING' ||
          status === 'AUTHENTICATING' ||
          status === 'PTY_OPENING' ||
          status === 'DISCONNECTING'
        "
      >
        <Loader2 :size="32" class="text-on-surface-muted animate-spin" />
        <p class="text-sm text-on-surface-muted">
          {{ t(`session.status.${status}`) }}
        </p>
      </template>

      <template v-else-if="status === 'FAILED'">
        <AlertTriangle :size="32" class="text-danger-text" />
        <p class="text-sm text-danger-text">{{ t("session.status.FAILED") }}</p>
        <p v-if="error" class="text-xs text-on-surface-muted max-w-md text-center px-4">
          {{ error }}
        </p>
        <button
          type="button"
          class="mt-2 inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded bg-accent hover:bg-accent-hover active:bg-accent-active text-on-accent disabled:opacity-60"
          :disabled="reconnecting"
          @click="reconnect"
        >
          <RotateCw :size="14" :class="reconnecting ? 'animate-spin' : ''" />
          {{ t("session.reconnect") }}
        </button>
      </template>

      <template v-else-if="status === 'CLOSED'">
        <p class="text-sm text-on-surface-muted">{{ t("session.status.CLOSED") }}</p>
        <button
          type="button"
          class="mt-2 inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded bg-surface-alt hover:bg-surface-card"
          :disabled="reconnecting"
          @click="reconnect"
        >
          <RotateCw :size="14" :class="reconnecting ? 'animate-spin' : ''" />
          {{ t("session.reconnect") }}
        </button>
      </template>
    </div>
  </div>
</template>

<style scoped>
:deep(.xterm-viewport) {
  overflow-x: hidden;
  scrollbar-width: thin;
  scrollbar-color: rgb(var(--c-border)) rgb(var(--c-bg));
}

:deep(.xterm-viewport::-webkit-scrollbar) {
  width: 8px;
  height: 8px;
}

:deep(.xterm-viewport::-webkit-scrollbar-track) {
  background: rgb(var(--c-bg));
}

:deep(.xterm-viewport::-webkit-scrollbar-thumb) {
  background: rgb(var(--c-border));
  border-radius: 8px;
}
</style>
