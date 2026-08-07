import { LazyStore } from "@tauri-apps/plugin-store";

export const settingsStore = new LazyStore("settings.json");

const DIRECTORIES_KEY = "indexedDirectories";

export async function getIndexedDirectories(): Promise<string[]> {
  return (await settingsStore.get<string[]>(DIRECTORIES_KEY)) ?? [];
}

export async function setIndexedDirectories(dirs: string[]): Promise<void> {
  await settingsStore.set(DIRECTORIES_KEY, dirs);
  await settingsStore.save();
}

const OLLAMA_MODEL_KEY = "ollamaModel";

export async function getOllamaModelPreference(): Promise<string | null> {
  return (await settingsStore.get<string>(OLLAMA_MODEL_KEY)) ?? null;
}

export async function setOllamaModelPreference(model: string): Promise<void> {
  await settingsStore.set(OLLAMA_MODEL_KEY, model);
  await settingsStore.save();
}

const HOTKEY_KEY = "globalHotkey";

// Must match SUMMON_SHORTCUT in src-tauri/src/lib.rs - that's what's actually
// registered at app startup before this stored value (if different) gets applied.
export const DEFAULT_HOTKEY = "Alt+Shift+Space";

export async function getGlobalHotkeyPreference(): Promise<string> {
  return (await settingsStore.get<string>(HOTKEY_KEY)) ?? DEFAULT_HOTKEY;
}

export async function setGlobalHotkeyPreference(hotkey: string): Promise<void> {
  await settingsStore.set(HOTKEY_KEY, hotkey);
  await settingsStore.save();
}
