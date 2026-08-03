import { useEffect, useState } from "react";
import { SearchView } from "@/components/SearchView";
import { SettingsView } from "@/components/SettingsView";
import { IndexingProvider } from "@/lib/indexing-context";
import { OllamaProvider } from "@/lib/ollama-context";

type View = "search" | "settings";

function App() {
  const [view, setView] = useState<View>("search");

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape" && view === "settings") {
        setView("search");
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [view]);

  return (
    <IndexingProvider>
      <OllamaProvider>
        <main className="h-screen w-screen overflow-hidden rounded-xl border border-border bg-background shadow-2xl backdrop-blur-xl">
          {view === "search" ? (
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
