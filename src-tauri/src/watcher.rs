use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::path::Path;
use std::sync::mpsc;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

use crate::ingest::{is_supported, path_is_within_skipped_dir};

#[derive(Debug, Clone, Serialize)]
pub struct FileChangeEvent {
    pub path: String,
    pub kind: String,
}

#[derive(Default)]
pub struct WatcherState(pub Mutex<Option<RecommendedWatcher>>);

fn classify(kind: &EventKind) -> Option<&'static str> {
    match kind {
        EventKind::Create(_) => Some("created"),
        EventKind::Modify(_) => Some("modified"),
        EventKind::Remove(_) => Some("removed"),
        _ => None,
    }
}

#[tauri::command]
pub fn start_watching(
    app: AppHandle,
    state: State<WatcherState>,
    directories: Vec<String>,
) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    *guard = None; // drop any existing watcher before starting a new one

    if directories.is_empty() {
        return Ok(());
    }

    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })
        .map_err(|e| e.to_string())?;

    for dir in &directories {
        watcher
            .watch(Path::new(dir), RecursiveMode::Recursive)
            .map_err(|e| format!("failed to watch {dir}: {e}"))?;
    }

    let emitter = app.clone();
    std::thread::spawn(move || {
        for res in rx {
            let Ok(event) = res else { continue };
            let Some(kind) = classify(&event.kind) else {
                continue;
            };
            for path in event.paths {
                // `notify` watches the whole tree recursively at the OS level with no
                // per-subdirectory opt-out, so events from inside e.g. Library or
                // node_modules still arrive here even though the initial scan never
                // touches them - filter them out the same way scan_directory would.
                if !is_supported(&path) || path_is_within_skipped_dir(&path) {
                    continue;
                }
                let _ = emitter.emit(
                    "file-changed",
                    FileChangeEvent {
                        path: path.to_string_lossy().to_string(),
                        kind: kind.to_string(),
                    },
                );
            }
        }
    });

    *guard = Some(watcher);
    Ok(())
}

#[tauri::command]
pub fn stop_watching(state: State<WatcherState>) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    *guard = None;
    Ok(())
}
