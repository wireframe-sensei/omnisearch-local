import { LazyStore } from "@tauri-apps/plugin-store";

export const settingsStore = new LazyStore("settings.json");

// The LazyStore automatically loads data on first access.
// This function ensures a slight delay to allow initialization.
export async function ensureStoreInitialized(): Promise<void> {
  // Access the store to trigger initialization
  try {
    await settingsStore.get("_initialized");
  } catch (e) {
    console.error("Failed to initialize settings store:", e);
  }
}

const DIRECTORIES_KEY = "indexedDirectories";

export async function getIndexedDirectories(): Promise<string[]> {
  await ensureStoreInitialized();
  return (await settingsStore.get<string[]>(DIRECTORIES_KEY)) ?? [];
}

export async function setIndexedDirectories(dirs: string[]): Promise<void> {
  await ensureStoreInitialized();
  await settingsStore.set(DIRECTORIES_KEY, dirs);
  await settingsStore.save();
}

const OLLAMA_MODEL_KEY = "ollamaModel";

export async function getOllamaModelPreference(): Promise<string | null> {
  await ensureStoreInitialized();
  return (await settingsStore.get<string>(OLLAMA_MODEL_KEY)) ?? null;
}

export async function setOllamaModelPreference(model: string): Promise<void> {
  await ensureStoreInitialized();
  await settingsStore.set(OLLAMA_MODEL_KEY, model);
  await settingsStore.save();
}

const HOTKEY_KEY = "globalHotkey";

// Must match SUMMON_SHORTCUT in src-tauri/src/lib.rs - that's what's actually
// registered at app startup before this stored value (if different) gets applied.
export const DEFAULT_HOTKEY = "Alt+Shift+Space";

export async function getGlobalHotkeyPreference(): Promise<string> {
  await ensureStoreInitialized();
  return (await settingsStore.get<string>(HOTKEY_KEY)) ?? DEFAULT_HOTKEY;
}

export async function setGlobalHotkeyPreference(hotkey: string): Promise<void> {
  await ensureStoreInitialized();
  await settingsStore.set(HOTKEY_KEY, hotkey);
  await settingsStore.save();
}

const IMAGE_EXTRACTION_KEY = "extractImageText";

export async function getImageExtractionEnabled(): Promise<boolean> {
  await ensureStoreInitialized();
  return (await settingsStore.get<boolean>(IMAGE_EXTRACTION_KEY)) ?? false;
}

export async function setImageExtractionEnabled(enabled: boolean): Promise<void> {
  await ensureStoreInitialized();
  await settingsStore.set(IMAGE_EXTRACTION_KEY, enabled);
  await settingsStore.save();
}
