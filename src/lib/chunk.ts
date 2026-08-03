import { getTokenizer, MODEL_MAX_TOKENS } from "@/lib/embeddings";

export interface TextChunk {
  index: number;
  text: string;
}

const CHUNK_SIZE_TOKENS = MODEL_MAX_TOKENS;
const CHUNK_OVERLAP_TOKENS = 32;
const STRIDE_TOKENS = CHUNK_SIZE_TOKENS - CHUNK_OVERLAP_TOKENS;

/**
 * Splits `text` into overlapping windows sized to the embedding model's own token budget
 * (see MODEL_MAX_TOKENS), so no chunk gets silently truncated when it's later embedded.
 */
export async function chunkText(text: string): Promise<TextChunk[]> {
  const trimmed = text.trim();
  if (!trimmed) return [];

  const tokenizer = await getTokenizer();
  const ids = tokenizer.encode(trimmed, { add_special_tokens: false });
  if (ids.length === 0) return [];

  const chunks: TextChunk[] = [];
  let start = 0;
  let index = 0;

  while (start < ids.length) {
    const end = Math.min(start + CHUNK_SIZE_TOKENS, ids.length);
    const windowIds = ids.slice(start, end);
    const chunkText = tokenizer.decode(windowIds, { skip_special_tokens: true }).trim();
    if (chunkText.length > 0) {
      chunks.push({ index, text: chunkText });
      index += 1;
    }
    if (end === ids.length) break;
    start += STRIDE_TOKENS;
  }

  return chunks;
}
