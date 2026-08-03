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
import { indexDirectories, indexPath, type IndexFailure } from "@/lib/indexer";
import { removeDocument, getIndexStats, type IndexStats } from "@/lib/vector-store";

const MAX_RECENT_CHANGES = 5;
/** Caps memory/UI size if a huge scan (e.g. a whole home folder) has many broken files. */
const MAX_FAILURES = 50;

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
  /** Files that couldn't be indexed at all, most recent first. */
  failures: IndexFailure[];
  addDirectories: (paths: string[]) => Promise<void>;
  removeDirectory: (path: string) => Promise<void>;
}

const IndexingContext = createContext<IndexingContextValue | null>(null);

function fileNameOf(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

export function IndexingProvider({ children }: { children: ReactNode }) {
  const [directories, setDirectories] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [fileCount, setFileCount] = useState<number | null>(null);
  const [scanning, setScanning] = useState(false);
  const [recentChanges, setRecentChanges] = useState<FileChangeEvent[]>([]);
  const [indexProgress, setIndexProgress] = useState<IndexProgress | null>(null);
  const [indexStats, setIndexStats] = useState<IndexStats | null>(null);
  const [failures, setFailures] = useState<IndexFailure[]>([]);

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

  function recordFailure(path: string, fileName: string, message: string) {
    setFailures((prev) => [
      { path, fileName, message },
      ...prev.filter((f) => f.path !== path),
    ].slice(0, MAX_FAILURES));
  }

  function clearFailure(path: string) {
    setFailures((prev) => prev.filter((f) => f.path !== path));
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
      .then(({ attemptedPaths, failures: newFailures }) => {
        if (cancelled) return;
        // Clear stale entries for anything reprocessed this run, then re-add only
        // what's still actually failing - a file that got fixed just drops off.
        const attempted = new Set(attemptedPaths);
        setFailures((prev) => {
          const kept = prev.filter((f) => !attempted.has(f.path));
          return [...newFailures, ...kept].slice(0, MAX_FAILURES);
        });
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
        .then(() => {
          clearFailure(event.path);
          refreshIndexStats();
        })
        .catch((e) => {
          console.error(`Failed to apply ${event.kind} for ${event.path}`, e);
          recordFailure(
            event.path,
            fileNameOf(event.path),
            e instanceof Error ? e.message : String(e),
          );
        });
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
        failures,
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
