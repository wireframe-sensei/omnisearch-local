mod hybrid_search;
mod ingest;
mod ollama;
mod store;
mod watcher;

use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, RunEvent, WindowEvent,
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Summons or dismisses the search window, mirroring launchers like Spotlight/Raycast.
/// Avoids Cmd+Space (macOS Spotlight), Cmd+Shift+Space (1Password's default quick-access
/// shortcut), and bare Alt+Space (Windows' window system menu).
const SUMMON_SHORTCUT: &str = "Alt+Shift+Space";

/// Tracks whichever shortcut is currently bound, so `set_global_shortcut` knows what to
/// unregister and can fall back to it if the requested replacement fails (e.g. already
/// held by another app) rather than leaving the app with no hotkey at all.
struct ActiveShortcut(Mutex<String>);

fn register_summon_shortcut(app: &AppHandle, shortcut: &str) -> Result<(), String> {
    app.global_shortcut()
        .on_shortcut(shortcut, |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                toggle_main_window(app);
            }
        })
        .map_err(|e| e.to_string())
}

/// Unregisters whatever shortcut is currently bound and registers `shortcut` in its
/// place. Falls back to re-registering the previous shortcut on failure (e.g. the
/// requested combo is already held by another app), so the app never ends up with no
/// hotkey at all.
#[tauri::command]
fn set_global_shortcut(
    app: AppHandle,
    state: tauri::State<ActiveShortcut>,
    shortcut: String,
) -> Result<(), String> {
    let mut current = state.0.lock().unwrap();
    let _ = app.global_shortcut().unregister_all();
    match register_summon_shortcut(&app, &shortcut) {
        Ok(()) => {
            *current = shortcut;
            Ok(())
        }
        Err(e) => {
            let _ = register_summon_shortcut(&app, &current);
            Err(e)
        }
    }
}

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
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .manage(watcher::WatcherState::default())
        .manage(ollama::OllamaState::default())
        .manage(ActiveShortcut(Mutex::new(SUMMON_SHORTCUT.to_string())))
        .setup(|app| {
            // Run as a menu-bar/tray-only utility (like Spotlight/Raycast/Alfred): no
            // persistent Dock icon while the process is alive, only the tray icon.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let app_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_dir)?;
            let db_path = app_dir.join("omnisearch.db");

            let conn = rusqlite::Connection::open(&db_path)?;
            store::init_db(&conn)?;
            app.manage(store::Db(Mutex::new(conn)));

            // A dedicated read-only connection for search, so it never queues behind an
            // indexing write transaction on the connection above - see `store::ReadDb`.
            let read_conn = rusqlite::Connection::open_with_flags(
                &db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            app.manage(store::ReadDb(Mutex::new(read_conn)));

            // Auto-hide on focus loss (clicking into another app), mirroring
            // Spotlight/Alfred/Raycast. Without this, `toggle_main_window`'s
            // `is_visible()` check gets out of sync with what the user actually sees:
            // the window stays visible-but-unfocused in the background, so the next
            // hotkey press hides it (since it's still "visible") instead of bringing it
            // forward, requiring a second press. Hiding here keeps `is_visible()`
            // truthful, so a single press always does what's expected.
            if let Some(window) = app.get_webview_window("main") {
                let window_to_hide = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        let _ = window_to_hide.hide();
                    }
                });
            }

            // Non-fatal: another process (e.g. a still-running previous `tauri dev`
            // instance) can hold this exact combo. Failing here shouldn't take down
            // the rest of setup (tray icon, database, etc.) with it. The frontend may
            // immediately replace this with a user-configured shortcut on mount (see
            // `set_global_shortcut`) - this is just what's active until then.
            if let Err(e) = register_summon_shortcut(app.handle(), SUMMON_SHORTCUT) {
                eprintln!("Failed to register global shortcut {SUMMON_SHORTCUT}: {e}");
            }

            let show_hide_item = MenuItem::with_id(app, "show_hide", "Show/Hide", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show_hide_item, &quit_item])?;

            // A dedicated monochrome icon (black artwork, alpha-only shape) rather than
            // the full-color app icon - macOS treats it as a "template" image and
            // recolors it automatically for light/dark menu bars and click-highlight
            // states. `icon_as_template` is a no-op outside macOS.
            let tray_icon = tauri::image::Image::from_bytes(include_bytes!(
                "../icons/tray-icon.png"
            ))?;

            TrayIconBuilder::new()
                .icon(tray_icon)
                .icon_as_template(true)
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
            store::get_chunks_pending_embedding_cmd,
            hybrid_search::hybrid_search_cmd,
            ollama::check_ollama_available,
            ollama::list_ollama_models,
            ollama::stream_ollama_answer,
            ollama::cancel_ollama_answer,
            set_global_shortcut,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Keep the app (and its tray icon / global hotkey) alive when the user
    // closes the window or hits Cmd+Q - hide instead of quitting. `code` is
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
