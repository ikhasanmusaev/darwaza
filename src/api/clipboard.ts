import { invoke } from "@tauri-apps/api/core";

/**
 * Системный буфер обмена.
 *
 * Сначала Rust: `navigator.clipboard` в WebKitGTK на Wayland не видит то, что
 * скопировали в других приложениях, и вставка молчит. Если Rust ответить не
 * может (Windows, macOS — там буфер доступен странице), берём запасной путь.
 * Ошибки не глотаем: они уходят в консоль.
 */
function warnUnlessUnsupported(action: string, e: unknown) {
  if (String(e) !== "unsupported") {
    console.warn(`${action} через Rust не удалось, пробуем WebView:`, e);
  }
}

export async function readClipboardText(): Promise<string> {
  try {
    return (await invoke<string | null>("clipboard_read_text")) ?? "";
  } catch (e) {
    warnUnlessUnsupported("чтение буфера", e);
  }
  try {
    return await navigator.clipboard.readText();
  } catch (e) {
    console.warn("чтение буфера обмена не удалось:", e);
    return "";
  }
}

export async function writeClipboardText(text: string): Promise<void> {
  try {
    await invoke<void>("clipboard_write_text", { text });
    return;
  } catch (e) {
    warnUnlessUnsupported("запись в буфер", e);
  }
  try {
    await navigator.clipboard.writeText(text);
  } catch (e) {
    console.warn("запись в буфер обмена не удалась:", e);
  }
}

function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(",")[1] ?? "");
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

/**
 * Картинка из буфера: PNG в base64 или `null`, если её нет. Картинка вместе с
 * текстом считается текстом: на сервер уезжает только то, что точно картинка.
 */
export async function readClipboardImage(): Promise<string | null> {
  try {
    return await invoke<string | null>("clipboard_read_image");
  } catch (e) {
    warnUnlessUnsupported("чтение картинки из буфера", e);
  }
  try {
    for (const item of await navigator.clipboard.read()) {
      if (item.types.includes("image/png") && !item.types.includes("text/plain")) {
        return await blobToBase64(await item.getType("image/png"));
      }
    }
  } catch (e) {
    console.warn("чтение картинки из буфера не удалось:", e);
  }
  return null;
}
