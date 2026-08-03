import { scanDirectories, extractDocumentText, type IndexableFile } from "@/lib/ingest";
import { chunkText, type TextChunk } from "@/lib/chunk";
import { embedOne } from "@/lib/embedding-pool";
import {
  upsertDocumentChunksText,
  updateChunkEmbeddings,
  removeDocument,
  getIndexedMtimes,
  type ChunkEmbeddingInput,
} from "@/lib/vector-store";

function extensionOf(path: string): string {
  const idx = path.lastIndexOf(".");
  return idx === -1 ? "" : path.slice(idx + 1).toLowerCase();
}

/** Extracts, chunks, and stores a file's text — no embedding yet, so it's immediately
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
 * Runs in two phases: first every changed file's text is extracted and stored (fast —
 * no ML involved — so the whole scan becomes keyword-searchable almost immediately),
 * then embeddings for everything just stored are computed across a worker pool with
 * real cross-file parallelism, attaching to (and making semantically searchable) each
 * file as soon as its own chunks finish, rather than waiting for the whole scan.
 */
export async function indexDirectories(
  directories: string[],
  onProgress?: (done: number, total: number) => void,
): Promise<void> {
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

  const filesToProcess = files.filter((f) => mtimeMap.get(f.path) !== f.modifiedMs);
  let done = files.length - filesToProcess.length;
  onProgress?.(done, files.length);

  // Phase 1: text only. Sequential is fine here — it's I/O + parsing, not ML, so it's
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
    }
  }

  // Phase 2: embed every pending file's chunks concurrently across the whole pool —
  // flattened across files (not per-file) so a file with few chunks never leaves
  // workers idle while another file with many chunks is still going.
  const remainingByPath = new Map(pending.map((p) => [p.path, p.chunks.length]));
  const resultsByPath = new Map<string, ChunkEmbeddingInput[]>(
    pending.map((p) => [p.path, []]),
  );

  const allChunkTasks = pending.flatMap((p) =>
    p.chunks.map((chunk) => ({ path: p.path, chunk })),
  );

  await Promise.all(
    allChunkTasks.map(async ({ path, chunk }) => {
      try {
        const embedding = await embedOne(chunk.text);
        resultsByPath.get(path)!.push({ chunkIndex: chunk.index, embedding });
      } catch (e) {
        console.error(`Failed to embed a chunk of ${path}`, e);
      }

      const remaining = remainingByPath.get(path)! - 1;
      remainingByPath.set(path, remaining);
      if (remaining === 0) {
        try {
          await updateChunkEmbeddings(path, resultsByPath.get(path)!);
        } catch (e) {
          console.error(`Failed to save embeddings for ${path}`, e);
        }
        done += 1;
        onProgress?.(done, files.length);
      }
    }),
  );
}
