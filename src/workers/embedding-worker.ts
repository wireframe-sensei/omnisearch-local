import { pipeline } from "@huggingface/transformers";
import { EMBEDDING_MODEL_ID } from "@/lib/embeddings";

/**
 * Runs the embedding model off the main thread so indexing (or a live search query)
 * never blocks UI rendering. A pool of these (see embedding-pool.ts) gives real
 * parallelism across CPU cores for bulk indexing.
 */

type Extractor = Awaited<ReturnType<typeof pipeline<"feature-extraction">>>;
let extractorPromise: Promise<Extractor> | null = null;

function getExtractor(): Promise<Extractor> {
  if (!extractorPromise) {
    extractorPromise = pipeline("feature-extraction", EMBEDDING_MODEL_ID);
  }
  return extractorPromise;
}

export interface EmbedRequest {
  id: number;
  text: string;
}

export interface EmbedResponse {
  id: number;
  embedding?: number[];
  error?: string;
}

self.onmessage = async (event: MessageEvent<EmbedRequest>) => {
  const { id, text } = event.data;
  try {
    const extractor = await getExtractor();
    const output = await extractor(text, { pooling: "mean", normalize: true });
    const embedding = Array.from(output.data as Float32Array);
    const response: EmbedResponse = { id, embedding };
    (self as unknown as Worker).postMessage(response);
  } catch (err) {
    const response: EmbedResponse = {
      id,
      error: err instanceof Error ? err.message : String(err),
    };
    (self as unknown as Worker).postMessage(response);
  }
};
