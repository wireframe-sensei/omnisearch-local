import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export function checkOllamaAvailable(): Promise<boolean> {
  return invoke("check_ollama_available");
}

export function listOllamaModels(): Promise<string[]> {
  return invoke("list_ollama_models");
}

export function cancelOllamaAnswer(): Promise<void> {
  return invoke("cancel_ollama_answer");
}

/**
 * Streams a local LLM answer via Ollama. Resolves when the full response has arrived
 * (or rejects on failure) - `onToken` fires incrementally as each piece streams in,
 * via the `ollama-token` event emitted from the Rust side.
 */
export async function streamOllamaAnswer(
  model: string,
  prompt: string,
  onToken: (token: string) => void,
): Promise<void> {
  const unlisten = await listen<string>("ollama-token", (event) => onToken(event.payload));
  try {
    await invoke("stream_ollama_answer", { model, prompt });
  } finally {
    unlisten();
  }
}
