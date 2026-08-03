import {
  createContext,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import { getIndexedDirectories, setIndexedDirectories } from "@/lib/settings-store";
import {
  scanDirectories,
  startWatching,
  stopWatching,
  onFileChanged,
  type FileChangeEvent,
} from "@/lib/ingest";
import { indexDirectories, indexPath } from "@/lib/indexer";
import { removeDocument, getIndexStats, type IndexStats } from "@/lib/vector-store";

const MAX_RECENT_CHANGES = 5;

interface IndexProgress {
  done: number;
  total: number;
}

interface IndexingContextValue {
  directories: string[];
  loading: boolean;
  fileCount: number | null;
  scanning: boolean;
  recentChanges: FileChangeEvent[];
  indexProgress: IndexProgress | null;
  indexStats: IndexStats | null;
  addDirectories: (paths: string[]) => Promise<void>;
  removeDirectory: (path: string) => Promise<void>;
}

const IndexingContext = createContext<IndexingContextValue | null>(null);

export function IndexingProvider({ children }: { children: ReactNode }) {
  const [directories, setDirectories] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [fileCount, setFileCount] = useState<number | null>(null);
  const [scanning, setScanning] = useState(false);
  const [recentChanges, setRecentChanges] = useState<FileChangeEvent[]>([]);
  const [indexProgress, setIndexProgress] = useState<IndexProgress | null>(null);
  const [indexStats, setIndexStats] = useState<IndexStats | null>(null);

  useEffect(() => {
    getIndexedDirectories()
      .then(setDirectories)
      .finally(() => setLoading(false));
  }, []);

  function refreshIndexStats() {
    getIndexStats()
      .then(setIndexStats)
      .catch((e) => console.error("Failed to load index stats", e));
  }

  // Re-scan, re-watch, and (re-)index for the app's lifetime whenever the directory list changes.
  useEffect(() => {
    if (loading) return;

    let cancelled = false;
    setScanning(true);
    scanDirectories(directories)
      .then((files) => {
        if (!cancelled) setFileCount(files.length);
      })
      .finally(() => {
        if (!cancelled) setScanning(false);
      });

    if (directories.length > 0) {
      startWatching(directories);
    } else {
      stopWatching();
    }

    setIndexProgress(directories.length > 0 ? { done: 0, total: 0 } : null);
    indexDirectories(directories, (done, total) => {
      if (!cancelled) setIndexProgress({ done, total });
    })
      .catch((e) => console.error("Indexing failed", e))
      .finally(() => {
        if (!cancelled) {
          setIndexProgress(null);
          refreshIndexStats();
        }
      });

    return () => {
      cancelled = true;
    };
  }, [directories, loading]);

  useEffect(() => {
    const unlisten = onFileChanged((event) => {
      setRecentChanges((prev) => [event, ...prev].slice(0, MAX_RECENT_CHANGES));

      const apply =
        event.kind === "removed" ? removeDocument(event.path) : indexPath(event.path);
      apply
        .then(refreshIndexStats)
        .catch((e) => console.error(`Failed to apply ${event.kind} for ${event.path}`, e));
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  async function addDirectories(paths: string[]) {
    const merged = Array.from(new Set([...directories, ...paths]));
    setDirectories(merged);
    await setIndexedDirectories(merged);
  }

  async function removeDirectory(path: string) {
    const updated = directories.filter((d) => d !== path);
    setDirectories(updated);
    await setIndexedDirectories(updated);
  }

  return (
    <IndexingContext.Provider
      value={{
        directories,
        loading,
        fileCount,
        scanning,
        recentChanges,
        indexProgress,
        indexStats,
        addDirectories,
        removeDirectory,
      }}
    >
      {children}
    </IndexingContext.Provider>
  );
}

export function useIndexing(): IndexingContextValue {
  const ctx = useContext(IndexingContext);
  if (!ctx) {
    throw new Error("useIndexing must be used within an IndexingProvider");
  }
  return ctx;
}
