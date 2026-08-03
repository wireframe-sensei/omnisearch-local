import { useEffect, useRef, useState } from "react";
import { Search, Settings, Sparkles, X } from "lucide-react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Button } from "@/components/ui/button";
import { FileIcon } from "@/components/FileIcon";
import { LoadingDots } from "@/components/LoadingDots";
import { useIndexing } from "@/lib/indexing-context";
import { useOllama } from "@/lib/ollama-context";
import { search as hybridSearch, type FileResult } from "@/lib/hybrid-search";
import { highlightParts } from "@/lib/snippet";
import { buildAnswerPrompt } from "@/lib/answer";
import { cancelOllamaAnswer, streamOllamaAnswer } from "@/lib/ollama";
import { parseAnswerCitations, citationLabel } from "@/lib/citations";

interface SearchViewProps {
  onOpenSettings: () => void;
}

const DEBOUNCE_MS = 300;

export function SearchView({ onOpenSettings }: SearchViewProps) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<FileResult[]>([]);
  const [searching, setSearching] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(0);

  const [answer, setAnswer] = useState("");
  const [answering, setAnswering] = useState(false);
  const [answerError, setAnswerError] = useState<string | null>(null);
  // The exact results an in-progress/completed answer was built from, so its citations
  // stay correct even if `results` itself later changes for some other reason.
  const [answerSources, setAnswerSources] = useState<FileResult[]>([]);

  const { directories, loading } = useIndexing();
  const { selectedModel: ollamaModel } = useOllama();
  const inputRef = useRef<HTMLInputElement>(null);
  const resultRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const requestId = useRef(0);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // The global hotkey shows/hides this window without remounting it, so refocus
  // the input whenever the OS hands focus back (e.g. summoned via the hotkey).
  useEffect(() => {
    const unlisten = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) inputRef.current?.focus();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Cancel any in-flight answer generation when this view unmounts (e.g. Settings opens).
  useEffect(() => {
    return () => {
      cancelOllamaAnswer().catch(() => {});
    };
  }, []);

  useEffect(() => {
    const trimmed = query.trim();
    if (!trimmed) {
      setResults([]);
      setSearching(false);
      return;
    }

    const currentRequest = ++requestId.current;
    setSearching(true);
    const timer = setTimeout(() => {
      hybridSearch(trimmed)
        .then((found) => {
          if (requestId.current === currentRequest) {
            setResults(found);
            setSelectedIndex(0);
          }
        })
        .catch((e) => console.error("Search failed", e))
        .finally(() => {
          if (requestId.current === currentRequest) setSearching(false);
        });
    }, DEBOUNCE_MS);

    return () => clearTimeout(timer);
  }, [query]);

  // The AI answer belongs to whatever query it was asked for - once that query
  // changes, the old answer (and any still-streaming generation) is stale.
  useEffect(() => {
    setAnswer("");
    setAnswerError(null);
    setAnswering(false);
    setAnswerSources([]);
    cancelOllamaAnswer().catch(() => {});
  }, [query]);

  useEffect(() => {
    resultRefs.current[selectedIndex]?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  function handleReveal(path: string) {
    revealItemInDir(path).catch((e) => console.error(`Failed to reveal ${path}`, e));
  }

  function handleDismissAnswer() {
    setAnswer("");
    setAnswerError(null);
    if (answering) {
      setAnswering(false);
      cancelOllamaAnswer().catch(() => {});
    }
  }

  async function handleAskAI() {
    const trimmed = query.trim();
    if (!ollamaModel || !trimmed || results.length === 0 || answering) return;

    await cancelOllamaAnswer().catch(() => {});
    setAnswer("");
    setAnswerError(null);
    setAnswering(true);
    setAnswerSources(results);

    const prompt = buildAnswerPrompt(trimmed, results);
    try {
      await streamOllamaAnswer(ollamaModel, prompt, (token) => {
        setAnswer((prev) => prev + token);
      });
    } catch (e) {
      setAnswerError(e instanceof Error ? e.message : "Failed to reach Ollama.");
    } finally {
      setAnswering(false);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      handleAskAI();
      return;
    }
    if (results.length === 0) return;

    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, results.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const selected = results[selectedIndex];
      if (selected) handleReveal(selected.path);
    }
  }

  const hasDirectories = loading ? null : directories.length > 0;
  const trimmedQuery = query.trim();
  const showAiBar = ollamaModel !== null && trimmedQuery.length > 0 && results.length > 0;

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-3 px-4 py-3">
        <Search className="size-5 shrink-0 text-muted-foreground" />
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
          onKeyDown={handleKeyDown}
          placeholder="Search your files…"
          className="w-full bg-transparent text-lg text-foreground placeholder:text-muted-foreground outline-none"
          autoFocus
        />
        <Button
          variant="ghost"
          size="icon"
          onClick={onOpenSettings}
          aria-label="Open settings"
        >
          <Settings />
        </Button>
      </div>

      <div className="flex-1 overflow-y-auto border-t border-border px-2 py-2">
        {showAiBar && (
          <div className="mb-1 rounded-lg border border-border bg-card">
            {answering || answer || answerError ? (
              <div className="px-3 py-2">
                <div className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                  <Sparkles className="size-3.5" />
                  <span>{answering && !answer ? "Thinking…" : "AI Answer"}</span>
                  {ollamaModel && <span className="font-normal opacity-70">· {ollamaModel}</span>}
                  <button
                    type="button"
                    onClick={handleDismissAnswer}
                    aria-label="Dismiss answer"
                    className="ml-auto rounded p-0.5 hover:bg-accent"
                  >
                    <X className="size-3.5" />
                  </button>
                </div>
                {answerError ? (
                  <p className="text-xs text-destructive">{answerError}</p>
                ) : (
                  <p className="whitespace-pre-wrap text-sm text-foreground">
                    {parseAnswerCitations(answer).map((part, i) =>
                      part.type === "text" ? (
                        <span key={i}>{part.text}</span>
                      ) : (
                        <CitationChips
                          key={i}
                          indices={part.indices}
                          sources={answerSources}
                          onReveal={handleReveal}
                        />
                      ),
                    )}
                    {answering && <LoadingDots />}
                  </p>
                )}
              </div>
            ) : (
              <button
                type="button"
                onClick={handleAskAI}
                className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-xs text-muted-foreground hover:bg-accent"
              >
                <Sparkles className="size-3.5 shrink-0" />
                <span>Ask AI about these results</span>
                <kbd className="ml-auto shrink-0 rounded border border-border px-1 text-[10px]">
                  ⌘↵
                </kbd>
              </button>
            )}
          </div>
        )}

        {hasDirectories === false ? (
          <EmptyState
            title="No folders indexed yet."
            subtitle="Click the gear icon to choose folders to search."
          />
        ) : trimmedQuery.length === 0 ? (
          <EmptyState title="Start typing to search…" />
        ) : searching ? (
          <EmptyState title="Searching…" />
        ) : results.length === 0 ? (
          <EmptyState title="No matches found." />
        ) : (
          <ul className="flex flex-col gap-1">
            {results.map((result, i) => (
              <li key={result.path}>
                <button
                  ref={(el) => {
                    resultRefs.current[i] = el;
                  }}
                  type="button"
                  onClick={() => handleReveal(result.path)}
                  onMouseEnter={() => setSelectedIndex(i)}
                  className={`flex w-full items-start gap-3 rounded-lg px-3 py-2 text-left ${
                    i === selectedIndex ? "bg-accent" : ""
                  }`}
                >
                  <FileIcon extension={result.extension} className="mt-0.5 size-5 shrink-0" />
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium text-foreground">
                      {result.fileName}
                    </p>
                    <p className="truncate text-xs text-muted-foreground" title={result.path}>
                      {result.path}
                    </p>
                    <p className="mt-1 line-clamp-2 text-xs text-muted-foreground">
                      {highlightParts(result.snippet, trimmedQuery).map((part, j) =>
                        part.match ? (
                          <mark key={j} className="rounded-sm bg-primary/20 text-foreground">
                            {part.text}
                          </mark>
                        ) : (
                          <span key={j}>{part.text}</span>
                        ),
                      )}
                    </p>
                  </div>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function EmptyState({ title, subtitle }: { title: string; subtitle?: string }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-1 px-6 text-center text-muted-foreground">
      <p className="text-sm">{title}</p>
      {subtitle && <p className="text-xs">{subtitle}</p>}
    </div>
  );
}

/** Renders a `[1]`/`[1, 2]` citation marker as clickable filename chips instead of raw numbers. */
function CitationChips({
  indices,
  sources,
  onReveal,
}: {
  indices: number[];
  sources: FileResult[];
  onReveal: (path: string) => void;
}) {
  const chips = indices
    .map((i) => sources[i - 1])
    .filter((r): r is FileResult => r !== undefined);

  if (chips.length === 0) {
    return <span>[{indices.join(", ")}]</span>;
  }

  return (
    <span className="mx-0.5 inline-flex flex-wrap items-center gap-1 align-middle">
      {chips.map((result) => (
        <button
          key={result.path}
          type="button"
          onClick={() => onReveal(result.path)}
          title={result.path}
          className="inline-flex items-center gap-1 rounded-md border border-border bg-accent px-1.5 py-0.5 align-middle text-xs font-medium text-accent-foreground hover:bg-primary/20"
        >
          <FileIcon extension={result.extension} className="size-3" />
          {citationLabel(result, sources)}
        </button>
      ))}
    </span>
  );
}
