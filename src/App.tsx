import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { SearchView } from "@/components/SearchView";
import { SettingsView } from "@/components/SettingsView";
import { IndexingProvider } from "@/lib/indexing-context";
import { OllamaProvider } from "@/lib/ollama-context";
import { applyGlobalShortcut } from "@/lib/hotkey";
import { DEFAULT_HOTKEY, getGlobalHotkeyPreference } from "@/lib/settings-store";
import { promptForFilePermissions } from "@/lib/permissions";

function App() {
  const [windowLabel, setWindowLabel] = useState<string>("");

  useEffect(() => {
    setWindowLabel(getCurrentWindow().label);
  }, []);

  // The backend registers the hardcoded default at startup (see SUMMON_SHORTCUT in
  // src-tauri/src/lib.rs) before this JS has even loaded, so the app is summonable
  // immediately. If the user configured a different one, swap it in now - there's a
  // brief window where only the default works, same tradeoff as every other setting
  // here (indexed directories, Ollama model) that's applied on mount rather than read
  // synchronously by the backend at launch.
  useEffect(() => {
    getGlobalHotkeyPreference().then((hotkey) => {
      if (hotkey === DEFAULT_HOTKEY) return;
      applyGlobalShortcut(hotkey).catch(() => {
        // Stored shortcut no longer registers (e.g. now held by another app) - fall
        // back silently to whatever the backend already has bound (the default).
      });
    });

    // Only prompt for permissions on the main window on first load
    if (getCurrentWindow().label === "main") {
      promptForFilePermissions().catch(() => {});
    }
  }, []);

  // Escape handling differs by window: Search dismisses it, Settings just closes
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key !== "Escape") return;
      if (windowLabel === "settings") {
        invoke("close_settings_window").catch((err) => {
          console.error("Failed to close settings:", err);
        });
      } else if (windowLabel === "main") {
        getCurrentWindow().hide().catch((err) => {
          console.error("Failed to hide window:", err);
        });
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [windowLabel]);

  const openSettings = () => {
    invoke("open_settings_window").catch((e) => {
      console.error("Failed to open settings window:", e);
    });
  };

  if (windowLabel === "settings") {
    return (
      <IndexingProvider>
        <OllamaProvider>
          <main className="h-screen w-screen overflow-hidden bg-background">
            <SettingsView onBack={() => invoke("close_settings_window")} />
          </main>
        </OllamaProvider>
      </IndexingProvider>
    );
  }

  return (
    <IndexingProvider>
      <OllamaProvider>
        <main className="h-screen w-screen overflow-hidden rounded-xl border border-border bg-background shadow-2xl backdrop-blur-xl">
          <SearchView onOpenSettings={openSettings} />
        </main>
      </OllamaProvider>
    </IndexingProvider>
  );
}

export default App;
