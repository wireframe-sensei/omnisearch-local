use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{AppHandle, Emitter, State};

const OLLAMA_BASE_URL: &str = "http://localhost:11434";
const AVAILABILITY_TIMEOUT_MS: u64 = 800;

/// Holds a cancellation flag for the in-flight answer stream, if any. Starting a new
/// stream replaces it, so an old request's remaining tokens are silently dropped.
#[derive(Default)]
pub struct OllamaState(pub Mutex<Option<Arc<AtomicBool>>>);

#[derive(Debug, Deserialize)]
struct TagsResponse {
    models: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GenerateChunk {
    response: Option<String>,
    #[serde(default)]
    done: bool,
}

#[tauri::command]
pub async fn check_ollama_available() -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(AVAILABILITY_TIMEOUT_MS))
        .build()
    else {
        return false;
    };

    client
        .get(format!("{OLLAMA_BASE_URL}/api/tags"))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

#[tauri::command]
pub async fn list_ollama_models() -> Result<Vec<String>, String> {
    let response = reqwest::get(format!("{OLLAMA_BASE_URL}/api/tags"))
        .await
        .map_err(|e| e.to_string())?;
    let parsed: TagsResponse = response.json().await.map_err(|e| e.to_string())?;
    Ok(parsed.models.into_iter().map(|m| m.name).collect())
}

#[tauri::command]
pub async fn stream_ollama_answer(
    app: AppHandle,
    state: State<'_, OllamaState>,
    model: String,
    prompt: String,
) -> Result<(), String> {
    let cancel_flag = Arc::new(AtomicBool::new(false));
    {
        let mut guard = state.0.lock().map_err(|e| e.to_string())?;
        *guard = Some(cancel_flag.clone());
    }

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": true,
    });

    let response = client
        .post(format!("{OLLAMA_BASE_URL}/api/generate"))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(format!("Ollama returned status {}", response.status()));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = stream.next().await {
        if cancel_flag.load(Ordering::Relaxed) {
            return Ok(());
        }

        let bytes = chunk.map_err(|e| e.to_string())?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer.drain(..=newline_pos);
            if line.is_empty() {
                continue;
            }

            let Ok(parsed) = serde_json::from_str::<GenerateChunk>(&line) else {
                continue;
            };

            if let Some(token) = parsed.response {
                if !token.is_empty() {
                    let _ = app.emit("ollama-token", token);
                }
            }
            if parsed.done {
                return Ok(());
            }
        }
    }

    Ok(())
}

#[tauri::command]
pub fn cancel_ollama_answer(state: State<OllamaState>) -> Result<(), String> {
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(flag) = guard.as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
    Ok(())
}
