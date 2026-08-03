import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { homeDir } from "@tauri-apps/api/path";
import {
  ArrowLeft,
  FolderPlus,
  FolderX,
  FolderClosed,
  House,
  RefreshCw,
  Sparkles,
  TriangleAlert,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { LoadingDots } from "@/components/LoadingDots";
import { useIndexing } from "@/lib/indexing-context";
import { useOllama } from "@/lib/ollama-context";
import { cancelOllamaAnswer, streamOllamaAnswer } from "@/lib/ollama";
import { buildErrorExplanationPrompt } from "@/lib/error-explainer";
import type { IndexFailure } from "@/lib/indexer";

interface SettingsViewProps {
  onBack: () => void;
}

interface Explanation {
  path: string;
  text: string;
  loading: boolean;
  error: string | null;
}

export function SettingsView({ onBack }: SettingsViewProps) {
  const {
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
  } = useIndexing();
  const {
    available: ollamaAvailable,
    models: ollamaModels,
    selectedModel,
    setSelectedModel,
    refresh: refreshOllama,
    refreshing: ollamaRefreshing,
  } = useOllama();
  const [explanation, setExplanation] = useState<Explanation | null>(null);
  const [confirmingRebuild, setConfirmingRebuild] = useState(false);
  const rebuildConfirmTimeout = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Settings is exactly where a newly-pulled model would need to show up, so
  // re-check every time this view opens rather than relying on stale app-launch state.
  useEffect(() => {
    refreshOllama();
  }, []);

  // Cancel any in-flight explanation if the view unmounts mid-stream.
  useEffect(() => {
    return () => {
      cancelOllamaAnswer().catch(() => {});
    };
  }, []);

  // Clear a pending rebuild confirmation on unmount so the timeout doesn't fire against
  // an unmounted component.
  useEffect(() => {
    return () => {
      if (rebuildConfirmTimeout.current) clearTimeout(rebuildConfirmTimeout.current);
    };
  }, []);

  // Two-click confirm rather than a modal: forcing a full rebuild re-embeds every file
  // regardless of whether it changed, which can take a while on a large index, so it
  // shouldn't be a single accidental click away.
  function handleRebuildClick() {
    if (confirmingRebuild) {
      if (rebuildConfirmTimeout.current) clearTimeout(rebuildConfirmTimeout.current);
      setConfirmingRebuild(false);
      refreshIndex(true);
      return;
    }
    setConfirmingRebuild(true);
    rebuildConfirmTimeout.current = setTimeout(() => setConfirmingRebuild(false), 4000);
  }

  async function handleExplain(failure: IndexFailure) {
    if (!selectedModel) return;

    await cancelOllamaAnswer().catch(() => {});
    setExplanation({ path: failure.path, text: "", loading: true, error: null });

    const prompt = buildErrorExplanationPrompt(failure.fileName, failure.message);
    try {
      await streamOllamaAnswer(selectedModel, prompt, (token) => {
        setExplanation((prev) =>
          prev && prev.path === failure.path ? { ...prev, text: prev.text + token } : prev,
        );
      });
      setExplanation((prev) =>
        prev && prev.path === failure.path ? { ...prev, loading: false } : prev,
      );
    } catch (e) {
      setExplanation({
        path: failure.path,
        text: "",
        loading: false,
        error: e instanceof Error ? e.message : "Failed to reach Ollama.",
      });
    }
  }

  async function handleAddDirectory() {
    const selected = await open({
      directory: true,
      multiple: true,
      title: "Select folders to index",
    });
    if (!selected) return;

    const picked = Array.isArray(selected) ? selected : [selected];
    await addDirectories(picked);
  }

  async function handleAddHomeFolder() {
    // Recursively includes Desktop, Documents, Downloads, etc. - one directory
    // covers all of them without picking each one individually.
    const home = await homeDir();
    await addDirectories([home]);
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-2 border-b border-border px-4 py-3">
        <Button variant="ghost" size="icon" onClick={onBack} aria-label="Back to search">
          <ArrowLeft />
        </Button>
        <h1 className="text-sm font-semibold text-foreground">Indexed Directories</h1>
      </div>

      <div className="flex-1 overflow-y-auto p-4">
        {!loading && directories.length > 0 && (
          <div className="mb-3 space-y-1">
            <div className="flex items-center gap-1.5">
              <p className="text-xs text-muted-foreground">
                {scanning
                  ? "Scanning…"
                  : `${fileCount ?? 0} file${fileCount === 1 ? "" : "s"} found across ${
                      directories.length
                    } folder${directories.length === 1 ? "" : "s"}`}
              </p>
              <button
                type="button"
                onClick={() => refreshIndex()}
                disabled={indexProgress !== null}
                aria-label="Refresh index"
                title="Rescan now for changes the file watcher may have missed"
                className="ml-auto rounded p-0.5 hover:bg-accent disabled:opacity-50"
              >
                <RefreshCw className={`size-3.5 ${indexProgress !== null ? "animate-spin" : ""}`} />
              </button>
            </div>
            {indexProgress && (
              <p className="text-xs text-muted-foreground">
                {indexProgress.phase === "files"
                  ? indexProgress.total > 0
                    ? `Indexing ${indexProgress.done}/${indexProgress.total} files…`
                    : "Preparing local embedding model (first run downloads it once)…"
                  : `Embedding ${indexProgress.done}/${indexProgress.total} pending chunks…`}
              </p>
            )}
            {!indexProgress && indexStats && (
              <p className="text-xs text-muted-foreground">
                {indexStats.embeddedChunkCount === indexStats.chunkCount
                  ? `${indexStats.chunkCount} chunk${indexStats.chunkCount === 1 ? "" : "s"} embedded`
                  : `${indexStats.embeddedChunkCount}/${indexStats.chunkCount} chunks embedded so far`}{" "}
                across {indexStats.documentCount} document
                {indexStats.documentCount === 1 ? "" : "s"}
              </p>
            )}
            {!indexProgress && (
              <button
                type="button"
                onClick={handleRebuildClick}
                title="Reprocess every file from scratch, even ones that haven't changed - use if you fixed an indexing bug or the index seems stale"
                className={`text-xs underline decoration-dotted ${
                  confirmingRebuild
                    ? "text-destructive"
                    : "text-muted-foreground hover:text-foreground"
                }`}
              >
                {confirmingRebuild
                  ? "Click again to reprocess every file (may take a while)"
                  : "Rebuild index from scratch"}
              </button>
            )}
          </div>
        )}
        {loading ? (
          <p className="text-sm text-muted-foreground">Loading…</p>
        ) : directories.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-center text-muted-foreground">
            <FolderClosed className="size-8 opacity-50" />
            <p className="text-sm">No folders selected yet.</p>
            <p className="text-xs">Add a folder to start indexing its files.</p>
          </div>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {directories.map((dir) => (
              <li
                key={dir}
                className="flex items-center justify-between gap-2 rounded-md border border-border bg-card px-3 py-2"
              >
                <span className="truncate text-sm text-foreground" title={dir}>
                  {dir}
                </span>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => removeDirectory(dir)}
                  aria-label={`Remove ${dir}`}
                >
                  <FolderX className="text-destructive" />
                </Button>
              </li>
            ))}
          </ul>
        )}

        {recentChanges.length > 0 && (
          <div className="mt-4">
            <p className="mb-1.5 text-xs font-medium text-muted-foreground">
              Recent activity
            </p>
            <ul className="flex flex-col gap-1">
              {recentChanges.map((change, i) => (
                <li
                  key={`${change.path}-${i}`}
                  className="flex items-center gap-2 truncate text-xs text-muted-foreground"
                  title={change.path}
                >
                  <span className="rounded bg-accent px-1.5 py-0.5 text-[10px] uppercase text-accent-foreground">
                    {change.kind}
                  </span>
                  <span className="truncate">{change.path}</span>
                </li>
              ))}
            </ul>
          </div>
        )}

        {failures.length > 0 && (
          <div className="mt-4 border-t border-border pt-4">
            <p className="mb-1.5 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
              <TriangleAlert className="size-3.5" />
              Couldn't index {failures.length} file{failures.length === 1 ? "" : "s"}
            </p>
            <ul className="flex flex-col gap-1.5">
              {failures.map((failure) => (
                <li
                  key={failure.path}
                  className="rounded-md border border-border bg-card px-3 py-2"
                >
                  <div className="flex items-center justify-between gap-2">
                    <span
                      className="truncate text-sm font-medium text-foreground"
                      title={failure.path}
                    >
                      {failure.fileName}
                    </span>
                    {selectedModel && (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="shrink-0"
                        onClick={() => handleExplain(failure)}
                      >
                        <Sparkles className="size-3" />
                        Explain
                      </Button>
                    )}
                  </div>
                  <p
                    className="mt-0.5 truncate text-xs text-muted-foreground"
                    title={failure.message}
                  >
                    {failure.message}
                  </p>
                  {explanation?.path === failure.path && (
                    <div className="mt-1.5 rounded-md bg-accent px-2 py-1.5 text-xs text-accent-foreground">
                      {explanation.error ? (
                        <span className="text-destructive">{explanation.error}</span>
                      ) : (
                        <>
                          {explanation.text}
                          {explanation.loading && <LoadingDots />}
                        </>
                      )}
                    </div>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )}

        <div className="mt-4 border-t border-border pt-4">
          <div className="mb-1.5 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <Sparkles className="size-3.5" />
            <span>AI Answers</span>
            <button
              type="button"
              onClick={() => refreshOllama()}
              disabled={ollamaRefreshing}
              aria-label="Refresh Ollama models"
              className="ml-auto rounded p-0.5 hover:bg-accent disabled:opacity-50"
            >
              <RefreshCw className={`size-3.5 ${ollamaRefreshing ? "animate-spin" : ""}`} />
            </button>
          </div>
          {ollamaAvailable === false && (
            <p className="text-xs text-muted-foreground">
              Ollama not detected. Install and run it locally to enable AI-generated answers.
            </p>
          )}
          {ollamaAvailable && ollamaModels.length === 0 && (
            <p className="text-xs text-muted-foreground">
              Ollama is running, but no models are installed. Run{" "}
              <code className="rounded bg-accent px-1 py-0.5">ollama pull &lt;model&gt;</code> to
              add one.
            </p>
          )}
          {ollamaAvailable && ollamaModels.length > 0 && (
            <div>
              <label
                htmlFor="ollama-model"
                className="mb-1 block text-xs text-muted-foreground"
              >
                Model used for AI answers
              </label>
              <select
                id="ollama-model"
                value={selectedModel ?? ""}
                onChange={(e) => setSelectedModel(e.target.value)}
                className="w-full rounded-md border border-input bg-transparent px-3 py-1.5 text-sm text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring"
              >
                {ollamaModels.map((model) => (
                  <option key={model} value={model}>
                    {model}
                  </option>
                ))}
              </select>
            </div>
          )}
        </div>
      </div>

      <div className="flex gap-2 border-t border-border p-4">
        <Button onClick={handleAddDirectory} className="flex-1">
          <FolderPlus />
          Add Directory
        </Button>
        <Button onClick={handleAddHomeFolder} variant="outline" className="flex-1" title="Includes Desktop, Documents, Downloads, and every other folder in your home directory">
          <House />
          Add Home Folder
        </Button>
      </div>
    </div>
  );
}
