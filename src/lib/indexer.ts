import { scanDirectories, extractDocumentText, type IndexableFile } from "@/lib/ingest";
import { chunkText, type TextChunk } from "@/lib/chunk";
import { embedOne } from "@/lib/embedding-pool";
import {
  upsertDocumentChunksText,
  updateChunkEmbeddings,
  removeDocument,
  getIndexedMtimes,
  getIndexStats,
  getChunksPendingEmbedding,
  type ChunkEmbeddingInput,
} from "@/lib/vector-store";

/** Bounds IPC payload size and memory per round-trip when draining a large backlog of
 * chunks still missing an embedding - see `resumePendingEmbeddings`. Kept small (rather
 * than a larger batch processed in one uninterrupted burst) so there's a checkpoint to
 * pause at every few seconds instead of every few minutes. */
const PENDING_EMBED_BATCH_SIZE = 100;
/** Paused between batches so the CPU cores the worker pool was using actually go idle
 * for a moment - the resume loop can run for hours on a large backlog, and a tight
 * back-to-back loop left no window for the OS scheduler or other apps to get in. */
const PENDING_EMBED_BATCH_PAUSE_MS = 500;

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export interface IndexFailure {
  path: string;
  fileName: string;
  message: string;
}

export interface IndexRunResult {
  /** Every path this run attempted to (re-)index, whether it succeeded or not - lets
   * the caller clear stale failure entries for files that succeeded this time. */
  attemptedPaths: string[];
  failures: IndexFailure[];
}

function extensionOf(path: string): string {
  const idx = path.lastIndexOf(".");
  return idx === -1 ? "" : path.slice(idx + 1).toLowerCase();
}

function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Extracts, chunks, and stores a file's text - no embedding yet, so it's immediately
 * keyword (BM25) searchable. Returns the chunks so the caller can embed them next. */
async function storeFileText(file: IndexableFile): Promise<TextChunk[]> {
  const doc = await extractDocumentText(file.path);
  const chunks = await chunkText(doc.text);

  if (chunks.length === 0) {
    await removeDocument(file.path);
    return [];
  }

  await upsertDocumentChunksText(
    file.path,
    file.modifiedMs,
    chunks.map((c) => ({ chunkIndex: c.index, text: c.text })),
  );
  return chunks;
}

/** Embeds a file's already-stored chunks (in parallel across the worker pool) and
 * attaches the results. */
async function embedFileChunks(path: string, chunks: TextChunk[]): Promise<void> {
  const embeddings = await Promise.all(chunks.map((c) => embedOne(c.text)));
  const inputs: ChunkEmbeddingInput[] = chunks.map((c, i) => ({
    chunkIndex: c.index,
    embedding: embeddings[i],
  }));
  await updateChunkEmbeddings(path, inputs);
}

interface FlatChunk {
  path: string;
  chunkIndex: number;
  text: string;
}

/**
 * Embeds a flat list of chunks - possibly spanning many files - concurrently across the
 * whole worker pool, attaching each file's embeddings via `updateChunkEmbeddings` as soon
 * as every one of that file's chunks in this batch has been attempted. Flattened (rather
 * than processed file-by-file) so a file with few chunks never leaves workers idle while
 * another file with many chunks is still going. A chunk that fails to embed is logged and
 * skipped rather than failing the whole batch - it simply stays unembedded and gets picked
 * up again by whatever finds pending chunks next (see `resumePendingEmbeddings`).
 *
 * Shared by `indexDirectories`'s Phase 2 (chunks just freshly extracted) and
 * `resumePendingEmbeddings` (chunks left over from an earlier, interrupted run) so the
 * two can't drift apart.
 */
async function embedAndAttachChunks(
  chunks: FlatChunk[],
  callbacks?: { onChunkDone?: () => void; onPathDone?: (path: string) => void },
): Promise<void> {
  const remainingByPath = new Map<string, number>();
  const resultsByPath = new Map<string, ChunkEmbeddingInput[]>();
  for (const chunk of chunks) {
    remainingByPath.set(chunk.path, (remainingByPath.get(chunk.path) ?? 0) + 1);
    if (!resultsByPath.has(chunk.path)) resultsByPath.set(chunk.path, []);
  }

  await Promise.all(
    chunks.map(async (chunk) => {
      try {
        const embedding = await embedOne(chunk.text);
        resultsByPath.get(chunk.path)!.push({ chunkIndex: chunk.chunkIndex, embedding });
      } catch (e) {
        console.error(`Failed to embed a chunk of ${chunk.path}`, e);
      }
      callbacks?.onChunkDone?.();

      const remaining = remainingByPath.get(chunk.path)! - 1;
      remainingByPath.set(chunk.path, remaining);
      if (remaining === 0) {
        try {
          await updateChunkEmbeddings(chunk.path, resultsByPath.get(chunk.path)!);
        } catch (e) {
          console.error(`Failed to save embeddings for ${chunk.path}`, e);
        }
        callbacks?.onPathDone?.(chunk.path);
      }
    }),
  );
}

export async function indexFile(file: IndexableFile): Promise<void> {
  const chunks = await storeFileText(file);
  if (chunks.length === 0) return;
  await embedFileChunks(file.path, chunks);
}

/** Re-indexes a single path reported by the file watcher (create/modify events). */
export async function indexPath(path: string): Promise<void> {
  await indexFile({
    path,
    fileName: path.split(/[/\\]/).pop() ?? path,
    extension: extensionOf(path),
    sizeBytes: 0,
    modifiedMs: Date.now(),
  });
}

/**
 * Scans all `directories`, embedding any file that's new or whose modified time has
 * changed since it was last indexed, and removes documents whose files are no longer
 * reachable from the current directory selection.
 *
 * Runs in two phases: first every changed file's text is extracted and stored (fast -
 * no ML involved - so the whole scan becomes keyword-searchable almost immediately),
 * then embeddings for everything just stored are computed across a worker pool with
 * real cross-file parallelism, attaching to (and making semantically searchable) each
 * file as soon as its own chunks finish, rather than waiting for the whole scan.
 *
 * Files that fail extraction entirely (a file this whole app couldn't get any text
 * out of) are reported in `failures` rather than just logged, so the UI can show the
 * user what didn't make it in. A file that never successfully indexes has no recorded
 * mtime, so it's retried - and re-reported here if it's still broken - on every run.
 *
 * `force` skips the mtime comparison and reprocesses every scanned file regardless of
 * whether it changed. Normal runs are mtime-gated so cost scales with what actually
 * changed, but that also means a file whose *extraction logic* changed (a bug fix, not
 * the file itself) keeps its old, possibly-wrong stored text forever - `force` is the
 * only way to pick that up.
 */
export async function indexDirectories(
  directories: string[],
  onProgress?: (done: number, total: number) => void,
  options?: { force?: boolean },
): Promise<IndexRunResult> {
  const [files, existingMtimes] = await Promise.all([
    scanDirectories(directories),
    getIndexedMtimes(),
  ]);

  const mtimeMap = new Map(existingMtimes.map((d) => [d.path, d.modifiedMs]));
  const scannedPaths = new Set(files.map((f) => f.path));

  for (const path of mtimeMap.keys()) {
    if (!scannedPaths.has(path)) {
      await removeDocument(path).catch(() => {});
    }
  }

  const filesToProcess = options?.force
    ? files
    : files.filter((f) => mtimeMap.get(f.path) !== f.modifiedMs);
  const attemptedPaths = filesToProcess.map((f) => f.path);
  const failures: IndexFailure[] = [];

  let done = files.length - filesToProcess.length;
  onProgress?.(done, files.length);

  // Phase 1: text only. Sequential is fine here - it's I/O + parsing, not ML, so it's
  // already fast, and each file becomes keyword-searchable the moment it's stored.
  const pending: { path: string; chunks: TextChunk[] }[] = [];
  for (const file of filesToProcess) {
    try {
      const chunks = await storeFileText(file);
      if (chunks.length > 0) {
        pending.push({ path: file.path, chunks });
      }
    } catch (e) {
      console.error(`Failed to extract ${file.path}`, e);
      failures.push({ path: file.path, fileName: file.fileName, message: errorMessage(e) });
    }
  }

  // Phase 2: embed every pending file's chunks concurrently across the whole pool -
  // flattened across files (not per-file) so a file with few chunks never leaves
  // workers idle while another file with many chunks is still going.
  const allChunkTasks: FlatChunk[] = pending.flatMap((p) =>
    p.chunks.map((chunk) => ({ path: p.path, chunkIndex: chunk.index, text: chunk.text })),
  );

  await embedAndAttachChunks(allChunkTasks, {
    onPathDone: () => {
      done += 1;
      onProgress?.(done, files.length);
    },
  });

  return { attemptedPaths, failures };
}

/**
 * Embeds every chunk still missing an embedding, regardless of when its text was stored -
 * independent of the mtime comparison `indexDirectories` uses to decide which *files* to
 * reprocess. This is what lets an embedding pass interrupted mid-run (app closed, process
 * restarted) resume where it left off on the next call, instead of those files being
 * permanently marked "up to date" by their already-recorded mtime and never revisited.
 *
 * Streams through the backlog in bounded batches rather than fetching it all at once -
 * necessary since a large, freshly-adopted home folder can leave hundreds of thousands of
 * chunks pending. Safely interruptible and idempotent by construction: a chunk that fails
 * (or that the app never gets to before closing) simply stays `embedding IS NULL`, so the
 * same query naturally picks it up again next time - no separate checkpoint state needed.
 */
export async function resumePendingEmbeddings(
  onProgress?: (done: number, total: number) => void,
): Promise<void> {
  const stats = await getIndexStats();
  const total = stats.chunkCount - stats.embeddedChunkCount;
  if (total <= 0) return;

  let done = 0;
  onProgress?.(done, total);

  while (true) {
    const batch = await getChunksPendingEmbedding(PENDING_EMBED_BATCH_SIZE);
    if (batch.length === 0) break;

    await embedAndAttachChunks(batch, {
      onChunkDone: () => {
        done += 1;
        onProgress?.(Math.min(done, total), total);
      },
    });

    if (batch.length < PENDING_EMBED_BATCH_SIZE) break;
    await sleep(PENDING_EMBED_BATCH_PAUSE_MS);
  }
}
