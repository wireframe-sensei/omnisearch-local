import {
  createContext,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import { checkOllamaAvailable, listOllamaModels } from "@/lib/ollama";
import { getOllamaModelPreference, setOllamaModelPreference } from "@/lib/settings-store";

interface OllamaContextValue {
  /** null while the initial availability check is in flight. */
  available: boolean | null;
  models: string[];
  /** null if Ollama is unavailable or has no models installed. */
  selectedModel: string | null;
  setSelectedModel: (model: string) => void;
  /** Re-checks availability and re-lists installed models (e.g. after `ollama pull`). */
  refresh: () => Promise<void>;
  refreshing: boolean;
}

const OllamaContext = createContext<OllamaContextValue | null>(null);

export function OllamaProvider({ children }: { children: ReactNode }) {
  const [available, setAvailable] = useState<boolean | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [selectedModel, setSelectedModelState] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  async function refresh() {
    setRefreshing(true);
    try {
      const ok = await checkOllamaAvailable();
      setAvailable(ok);
      if (!ok) {
        setModels([]);
        setSelectedModelState(null);
        return;
      }

      const list = await listOllamaModels();
      setModels(list);
      if (list.length === 0) {
        setSelectedModelState(null);
        return;
      }

      const preferred = await getOllamaModelPreference();
      setSelectedModelState(preferred && list.includes(preferred) ? preferred : list[0]);
    } finally {
      setRefreshing(false);
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  function setSelectedModel(model: string) {
    setSelectedModelState(model);
    setOllamaModelPreference(model).catch((e) =>
      console.error("Failed to save model preference", e),
    );
  }

  return (
    <OllamaContext.Provider
      value={{ available, models, selectedModel, setSelectedModel, refresh, refreshing }}
    >
      {children}
    </OllamaContext.Provider>
  );
}

export function useOllama(): OllamaContextValue {
  const ctx = useContext(OllamaContext);
  if (!ctx) {
    throw new Error("useOllama must be used within an OllamaProvider");
  }
  return ctx;
}
