import { invoke } from "@tauri-apps/api/core";

export interface TextChunkInput {
  chunkIndex: number;
  text: string;
}

export interface ChunkEmbeddingInput {
  chunkIndex: number;
  embedding: number[];
}

export interface IndexStats {
  documentCount: number;
  chunkCount: number;
  embeddedChunkCount: number;
}

export interface DocumentMtime {
  path: string;
  modifiedMs: number;
}

/** Stores chunk text immediately - keyword (BM25) searchable right away, before embedding. */
export function upsertDocumentChunksText(
  path: string,
  modifiedMs: number,
  chunks: TextChunkInput[],
): Promise<void> {
  return invoke("upsert_document_chunks_text", { path, modifiedMs, chunks });
}

/** Attaches embeddings to chunks already stored by upsertDocumentChunksText. */
export function updateChunkEmbeddings(
  path: string,
  embeddings: ChunkEmbeddingInput[],
): Promise<void> {
  return invoke("update_chunk_embeddings", { path, embeddings });
}

export function removeDocument(path: string): Promise<void> {
  return invoke("remove_document", { path });
}

export function getIndexStats(): Promise<IndexStats> {
  return invoke("get_index_stats_cmd");
}

export function getIndexedMtimes(): Promise<DocumentMtime[]> {
  return invoke("get_indexed_mtimes_cmd");
}
