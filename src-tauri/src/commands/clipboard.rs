//! Чтение буфера обмена на стороне Rust.
//!
//! Зачем. `navigator.clipboard.readText()` в WebKitGTK на Wayland отдаёт только
//! то, что скопировали внутри самого окна; текст из других приложений не
//! приходит (отказ или пустая строка), и вставка в терминале молчит. GTK — тот
//! же клиент Wayland, что и окно, и читает системный буфер как положено.
//!
//! На Windows и macOS `readText` работает, поэтому команда там честно
//! отвечает «не поддерживается», а страница берёт запасной путь.

/// Прочитать текст из системного буфера. `Ok(None)` — буфер пуст или в нём
/// не текст.
#[tauri::command]
pub async fn clipboard_read_text(app: tauri::AppHandle) -> Result<Option<String>, String> {
    #[cfg(target_os = "linux")]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            let text = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD)
                .wait_for_text()
                .map(|t| t.to_string());
            let _ = tx.send(text);
        })
        .map_err(|e| format!("clipboard: не удалось обратиться к главному потоку: {e}"))?;
        // Владелец буфера может не отвечать — не вешаем вставку навсегда.
        tokio::task::spawn_blocking(move || rx.recv_timeout(std::time::Duration::from_secs(3)))
            .await
            .map_err(|e| format!("clipboard: {e}"))?
            .map_err(|_| "clipboard: владелец буфера не ответил за 3 секунды".to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        Err("unsupported".to_string())
    }
}
