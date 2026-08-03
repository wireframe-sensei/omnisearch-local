import { invoke } from "@tauri-apps/api/core";

export interface ChunkSearchResult {
  path: string;
  chunkIndex: number;
  text: string;
  score: number;
}

/**
 * Blends semantic (embedding) similarity with BM25 keyword matching via Reciprocal
 * Rank Fusion, computed entirely in Rust — see hybrid_search.rs.
 */
export function hybridSearchChunks(
  query: string,
  queryEmbedding: number[],
  limit: number,
): Promise<ChunkSearchResult[]> {
  return invoke("hybrid_search_cmd", { query, queryEmbedding, limit });
}
