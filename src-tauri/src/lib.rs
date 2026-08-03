mod hybrid_search;
mod ingest;
mod ollama;
mod store;
mod watcher;

use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, RunEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Summons or dismisses the search window, mirroring launchers like Spotlight/Raycast.
/// Avoids Cmd+Space (macOS Spotlight), Cmd+Shift+Space (1Password's default quick-access
/// shortcut), and bare Alt+Space (Windows' window system menu).
const SUMMON_SHORTCUT: &str = "Alt+Shift+Space";

fn hide_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

fn toggle_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(watcher::WatcherState::default())
        .manage(ollama::OllamaState::default())
        .setup(|app| {
            // Run as a menu-bar/tray-only utility (like Spotlight/Raycast/Alfred): no
            // persistent Dock icon while the process is alive, only the tray icon.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let app_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_dir)?;
            let conn = rusqlite::Connection::open(app_dir.join("omnisearch.db"))?;
            store::init_db(&conn)?;
            app.manage(store::Db(Mutex::new(conn)));

            // Non-fatal: another process (e.g. a still-running previous `tauri dev`
            // instance) can hold this exact combo. Failing here shouldn't take down
            // the rest of setup (tray icon, database, etc.) with it.
            if let Err(e) = app
                .global_shortcut()
                .on_shortcut(SUMMON_SHORTCUT, |app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_main_window(app);
                    }
                })
            {
                eprintln!("Failed to register global shortcut {SUMMON_SHORTCUT}: {e}");
            }

            let show_hide_item = MenuItem::with_id(app, "show_hide", "Show/Hide", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show_hide_item, &quit_item])?;

            TrayIconBuilder::new()
                .icon(
                    app.default_window_icon()
                        .expect("app icon missing from tauri.conf.json")
                        .clone(),
                )
                .menu(&tray_menu)
                .tooltip("OmniSearch")
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show_hide" => toggle_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ingest::scan_directories_cmd,
            ingest::extract_document_text_cmd,
            watcher::start_watching,
            watcher::stop_watching,
            store::upsert_document_chunks_text,
            store::update_chunk_embeddings,
            store::remove_document,
            store::get_index_stats_cmd,
            store::get_indexed_mtimes_cmd,
            hybrid_search::hybrid_search_cmd,
            ollama::check_ollama_available,
            ollama::list_ollama_models,
            ollama::stream_ollama_answer,
            ollama::cancel_ollama_answer,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Keep the app (and its tray icon / global hotkey) alive when the user
    // closes the window or hits Cmd+Q — hide instead of quitting. `code` is
    // `None` for user-driven exit requests and `Some(_)` for a programmatic
    // `AppHandle::exit` call (used by the tray's real "Quit" item), so this
    // only intercepts the former.
    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { code, api, .. } = event {
            if code.is_none() {
                api.prevent_exit();
                hide_main_window(app_handle);
            }
        }
    });
}
