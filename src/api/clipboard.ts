import { invoke } from "@tauri-apps/api/core";

/**
 * Текст из системного буфера обмена.
 *
 * Сначала Rust: `navigator.clipboard.readText()` в WebKitGTK на Wayland не
 * отдаёт текст, скопированный в других приложениях, и вставка молчит. Если
 * Rust ответить не может (Windows, macOS — там `readText` работает), берём
 * запасной путь. Ошибки не глотаем: они уходят в консоль.
 */
export async function readClipboardText(): Promise<string> {
  try {
    return (await invoke<string | null>("clipboard_read_text")) ?? "";
  } catch (e) {
    if (String(e) !== "unsupported") {
      console.warn("чтение буфера через Rust не удалось, пробуем WebView:", e);
    }
  }
  try {
    return await navigator.clipboard.readText();
  } catch (e) {
    console.warn("чтение буфера обмена не удалось:", e);
    return "";
  }
}
