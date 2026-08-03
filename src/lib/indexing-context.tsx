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
import {
  indexDirectories,
  indexPath,
  resumePendingEmbeddings,
  type IndexFailure,
} from "@/lib/indexer";
import { removeDocument, getIndexStats, type IndexStats } from "@/lib/vector-store";

const MAX_RECENT_CHANGES = 5;
/** Caps memory/UI size if a huge scan (e.g. a whole home folder) has many broken files. */
const MAX_FAILURES = 50;

interface IndexProgress {
  done: number;
  total: number;
  /** "files": extracting/embedding newly changed files. "embeddings": draining a backlog
   * of chunks left over from an earlier, interrupted embedding pass - see
   * `resumePendingEmbeddings`. */
  phase: "files" | "embeddings";
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
  /** Re-runs indexing for the current directories on demand, rather than waiting for
   * the directory list to change or the app to restart. `force` reprocesses every file
   * regardless of mtime - see `indexDirectories`'s doc comment for why that's sometimes
   * necessary. */
  refreshIndex: (force?: boolean) => Promise<void>;
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

  // Shared by the automatic pass below and the manual `refreshIndex` trigger, so
  // "re-scan on directory change" and "user clicked refresh" can't drift apart.
  async function runIndexPass(dirs: string[], force: boolean, signal: { cancelled: boolean }) {
    setScanning(true);
    try {
      const files = await scanDirectories(dirs);
      if (!signal.cancelled) setFileCount(files.length);
    } finally {
      if (!signal.cancelled) setScanning(false);
    }

    setIndexProgress(dirs.length > 0 ? { done: 0, total: 0, phase: "files" } : null);
    try {
      const { attemptedPaths, failures: newFailures } = await indexDirectories(
        dirs,
        (done, total) => {
          if (!signal.cancelled) setIndexProgress({ done, total, phase: "files" });
        },
        { force },
      );
      if (!signal.cancelled) {
        // Clear stale entries for anything reprocessed this run, then re-add only
        // what's still actually failing - a file that got fixed just drops off.
        const attempted = new Set(attemptedPaths);
        setFailures((prev) => {
          const kept = prev.filter((f) => !attempted.has(f.path));
          return [...newFailures, ...kept].slice(0, MAX_FAILURES);
        });
      }

      // Resume any chunks still missing an embedding, regardless of mtime - this is
      // what lets a pass interrupted mid-embedding (app closed, process restarted)
      // pick back up automatically instead of being silently skipped forever.
      if (!signal.cancelled && dirs.length > 0) {
        await resumePendingEmbeddings((done, total) => {
          if (!signal.cancelled) setIndexProgress({ done, total, phase: "embeddings" });
        });
      }
    } catch (e) {
      console.error("Indexing failed", e);
    } finally {
      if (!signal.cancelled) {
        setIndexProgress(null);
        refreshIndexStats();
      }
    }
  }

  // Re-scan, re-watch, and (re-)index for the app's lifetime whenever the directory list changes.
  useEffect(() => {
    if (loading) return;

    const signal = { cancelled: false };

    if (directories.length > 0) {
      startWatching(directories);
    } else {
      stopWatching();
    }

    runIndexPass(directories, false, signal);

    return () => {
      signal.cancelled = true;
    };
  }, [directories, loading]);

  async function refreshIndex(force = false) {
    await runIndexPass(directories, force, { cancelled: false });
  }

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
        refreshIndex,
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
