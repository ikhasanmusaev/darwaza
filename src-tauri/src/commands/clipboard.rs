//! Буфер обмена на стороне Rust.
//!
//! Зачем. `navigator.clipboard.readText()` в WebKitGTK на Wayland отдаёт только
//! то, что скопировали внутри самого окна; текст из других приложений не
//! приходит (отказ или пустая строка), и вставка в терминале молчит. GTK — тот
//! же клиент Wayland, что и окно, и работает с системным буфером как положено.
//! Запись и картинки идут тем же путём, чтобы копирование не зависело от WebView.
//!
//! На macOS причина та же, но другая: страница живёт на своей схеме
//! (`tauri://localhost`), и `navigator.clipboard` там либо требует разрешения,
//! либо отсутствует вовсе — а `clipboard.read()` для картинок в WKWebView не
//! работает и при выданном разрешении. Поэтому идём в NSPasteboard напрямую:
//! он не спрашивает ничего и не зависит от того, что решил WebView.
//!
//! На Windows буфер доступен странице, поэтому команды там честно отвечают
//! «не поддерживается», а страница берёт запасной путь.

/// Сколько ждём владельца буфера. Он может не отвечать, а вставку нельзя вешать.
#[cfg(target_os = "linux")]
const OWNER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Выполнить чтение буфера на главном потоке GTK и дождаться ответа.
#[cfg(target_os = "linux")]
async fn on_main_thread<T, F>(app: &tauri::AppHandle, read: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(gtk::Clipboard) -> T + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(read(gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD)));
    })
    .map_err(|e| format!("clipboard: не удалось обратиться к главному потоку: {e}"))?;
    tokio::task::spawn_blocking(move || rx.recv_timeout(OWNER_TIMEOUT))
        .await
        .map_err(|e| format!("clipboard: {e}"))?
        .map_err(|_| "clipboard: владелец буфера не ответил за 3 секунды".to_string())
}

/// NSPasteboard: системный буфер macOS.
///
/// Функции берут буфер параметром, а не зовут общий сами: тест работает на
/// своём именованном буфере и не трогает то, что человек скопировал.
/// NSPasteboard потокобезопасен и главного потока не требует.
#[cfg(target_os = "macos")]
mod mac {
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardTypePNG,
        NSPasteboardTypeString, NSPasteboardTypeTIFF,
    };
    use objc2_foundation::{NSDictionary, NSString};

    /// Текст из буфера. `None` — там не текст.
    pub fn read_text(pb: &NSPasteboard) -> Option<String> {
        unsafe { pb.stringForType(NSPasteboardTypeString) }.map(|s| s.to_string())
    }

    /// Записать текст. `clearContents` обязателен: без него к новому тексту
    /// остаются подклеены прежние представления, и другое приложение возьмёт
    /// старое значение.
    pub fn write_text(pb: &NSPasteboard, text: &str) -> Result<(), String> {
        pb.clearContents();
        let ok = unsafe { pb.setString_forType(&NSString::from_str(text), NSPasteboardTypeString) };
        if ok {
            Ok(())
        } else {
            Err("clipboard: NSPasteboard не принял текст".to_string())
        }
    }

    /// Картинка из буфера в PNG. `None` — картинки нет.
    ///
    /// Снимок экрана macOS кладёт PNG, но часть программ — только TIFF;
    /// его переводим в PNG сами, потому что на сервер уезжает именно PNG.
    pub fn read_image(pb: &NSPasteboard) -> Result<Option<Vec<u8>>, String> {
        // Текст вместе с картинкой считаем текстом — то же правило, что на Linux.
        if read_text(pb).is_some() {
            return Ok(None);
        }
        if let Some(png) = unsafe { pb.dataForType(NSPasteboardTypePNG) } {
            return Ok(Some(png.to_vec()));
        }
        let Some(tiff) = (unsafe { pb.dataForType(NSPasteboardTypeTIFF) }) else {
            return Ok(None);
        };
        let rep = NSBitmapImageRep::imageRepWithData(&tiff)
            .ok_or_else(|| "clipboard: TIFF из буфера не разобрался".to_string())?;
        let png = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }
        .ok_or_else(|| "clipboard: TIFF не перевёлся в PNG".to_string())?;
        Ok(Some(png.to_vec()))
    }

    /// Общий буфер системы.
    pub fn general() -> objc2::rc::Retained<NSPasteboard> {
        NSPasteboard::generalPasteboard()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Свой буфер под каждый тест: общий — это буфер человека за машиной.
        fn private(name: &str) -> objc2::rc::Retained<NSPasteboard> {
            NSPasteboard::pasteboardWithName(&NSString::from_str(name))
        }

        #[test]
        fn text_survives_the_round_trip() {
            let pb = private("dev.darwaza.test.text");
            write_text(&pb, "привет, ёжик").unwrap();
            assert_eq!(read_text(&pb).as_deref(), Some("привет, ёжик"));
        }

        #[test]
        fn writing_replaces_instead_of_adding_up() {
            let pb = private("dev.darwaza.test.replace");
            write_text(&pb, "первый").unwrap();
            write_text(&pb, "второй").unwrap();
            assert_eq!(read_text(&pb).as_deref(), Some("второй"));
        }

        #[test]
        fn text_in_the_buffer_is_not_reported_as_an_image() {
            // Картинка уезжает на сервер, поэтому решать «это картинка» можно
            // только когда текста рядом нет.
            let pb = private("dev.darwaza.test.image");
            write_text(&pb, "это текст").unwrap();
            assert_eq!(read_image(&pb).unwrap(), None);
        }

        #[test]
        fn an_empty_buffer_has_neither_text_nor_image() {
            let pb = private("dev.darwaza.test.empty");
            pb.clearContents();
            assert_eq!(read_text(&pb), None);
            assert_eq!(read_image(&pb).unwrap(), None);
        }
    }
}

/// Прочитать текст из системного буфера. `Ok(None)` — буфер пуст или в нём
/// не текст.
#[tauri::command]
pub async fn clipboard_read_text(app: tauri::AppHandle) -> Result<Option<String>, String> {
    #[cfg(target_os = "linux")]
    {
        on_main_thread(&app, |clipboard| {
            clipboard.wait_for_text().map(|t| t.to_string())
        })
        .await
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        Ok(mac::read_text(&mac::general()))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = app;
        Err("unsupported".to_string())
    }
}

/// Записать текст в системный буфер.
#[tauri::command]
pub async fn clipboard_write_text(app: tauri::AppHandle, text: String) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        on_main_thread(&app, move |clipboard| {
            clipboard.set_text(&text);
            // Текст должен пережить закрытие окна, если менеджер буфера это умеет.
            clipboard.store();
        })
        .await
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        mac::write_text(&mac::general(), &text)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (app, text);
        Err("unsupported".to_string())
    }
}

/// Картинка из системного буфера: PNG в base64. `Ok(None)` — картинки нет.
///
/// Если вместе с картинкой лежит текст (ячейка таблицы, например), считаем, что
/// это текст: картинка уезжает на сервер, и делать это можно, только когда в
/// буфере точно она.
#[tauri::command]
pub async fn clipboard_read_image(app: tauri::AppHandle) -> Result<Option<String>, String> {
    #[cfg(target_os = "linux")]
    {
        use base64::Engine;

        on_main_thread(&app, |clipboard| {
            if clipboard.wait_is_text_available() || !clipboard.wait_is_image_available() {
                return Ok(None);
            }
            let Some(image) = clipboard.wait_for_image() else {
                return Ok(None);
            };
            let png = image
                .save_to_bufferv("png", &[])
                .map_err(|e| format!("clipboard: не удалось получить PNG: {e}"))?;
            Ok(Some(base64::engine::general_purpose::STANDARD.encode(png)))
        })
        .await?
    }
    #[cfg(target_os = "macos")]
    {
        use base64::Engine;

        let _ = app;
        Ok(mac::read_image(&mac::general())?
            .map(|png| base64::engine::general_purpose::STANDARD.encode(png)))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = app;
        Err("unsupported".to_string())
    }
}
