import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { SearchView } from "@/components/SearchView";
import { SettingsView } from "@/components/SettingsView";
import { IndexingProvider } from "@/lib/indexing-context";
import { OllamaProvider } from "@/lib/ollama-context";
import { applyGlobalShortcut } from "@/lib/hotkey";
import { DEFAULT_HOTKEY, getGlobalHotkeyPreference } from "@/lib/settings-store";
import { promptForFilePermissions } from "@/lib/permissions";
import { ensureStoreInitialized } from "@/lib/settings-store";

type View = "search" | "settings";

function App() {
  const [view, setView] = useState<View>("search");

  // The backend registers the hardcoded default at startup (see SUMMON_SHORTCUT in
  // src-tauri/src/lib.rs) before this JS has even loaded, so the app is summonable
  // immediately. If the user configured a different one, swap it in now - there's a
  // brief window where only the default works, same tradeoff as every other setting
  // here (indexed directories, Ollama model) that's applied on mount rather than read
  // synchronously by the backend at launch.
  useEffect(() => {
    (async () => {
      // Initialize settings store first
      await ensureStoreInitialized();

      // Then apply hotkey preference
      const hotkey = await getGlobalHotkeyPreference();
      if (hotkey !== DEFAULT_HOTKEY) {
        applyGlobalShortcut(hotkey).catch(() => {
          // Stored shortcut no longer registers (e.g. now held by another app) - fall
          // back silently to whatever the backend already has bound (the default).
        });
      }

      // Prompt for permissions on first load
      promptForFilePermissions().catch(() => {});
    })();
  }, []);

  // Escape: on Search, dismiss window; from Settings, step back to Search
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key !== "Escape") return;
      if (view === "settings") {
        setView("search");
      } else {
        getCurrentWindow().hide();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [view]);

  const isSearchView = view === "search";
  const containerClass = isSearchView
    ? "h-screen w-screen overflow-hidden rounded-xl border border-border bg-background shadow-2xl backdrop-blur-xl"
    : "h-screen w-screen overflow-hidden bg-background";

  return (
    <IndexingProvider>
      <OllamaProvider>
        <main className={containerClass}>
          {isSearchView ? (
            <SearchView onOpenSettings={() => setView("settings")} />
          ) : (
            <SettingsView onBack={() => setView("search")} />
          )}
        </main>
      </OllamaProvider>
    </IndexingProvider>
  );
}

export default App;
