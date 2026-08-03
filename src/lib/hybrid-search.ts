import { embedText } from "@/lib/embeddings";
import { hybridSearchChunks, type ChunkSearchResult } from "@/lib/search";
import { buildSnippet } from "@/lib/snippet";

export interface FileResult {
  path: string;
  fileName: string;
  extension: string;
  /** Short window around the best literal match, for display in the results list. */
  snippet: string;
  /** The full matched chunk text (up to ~256 tokens), for feeding into LLM answer synthesis. */
  fullText: string;
  score: number;
}

const CHUNKS_TO_FETCH = 40;

function fileNameOf(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

function extensionOf(path: string): string {
  const idx = path.lastIndexOf(".");
  return idx === -1 ? "" : path.slice(idx + 1).toLowerCase();
}

/** Keeps only the highest-scoring chunk per file, so one heavily-matched file doesn't crowd out others. */
function bestChunkPerFile(results: ChunkSearchResult[]): ChunkSearchResult[] {
  const bestByPath = new Map<string, ChunkSearchResult>();
  for (const result of results) {
    const existing = bestByPath.get(result.path);
    if (!existing || result.score > existing.score) {
      bestByPath.set(result.path, result);
    }
  }
  return Array.from(bestByPath.values()).sort((a, b) => b.score - a.score);
}

export async function search(query: string, limit = 8): Promise<FileResult[]> {
  const trimmed = query.trim();
  if (!trimmed) return [];

  const queryEmbedding = await embedText(trimmed);
  const chunkResults = await hybridSearchChunks(trimmed, queryEmbedding, CHUNKS_TO_FETCH);

  return bestChunkPerFile(chunkResults)
    .slice(0, limit)
    .map((result) => ({
      path: result.path,
      fileName: fileNameOf(result.path),
      extension: extensionOf(result.path),
      snippet: buildSnippet(result.text, trimmed),
      fullText: result.text,
      score: result.score,
    }));
}
