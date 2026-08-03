import { AutoTokenizer, type PreTrainedTokenizer } from "@huggingface/transformers";
import { embedOne, embedMany } from "@/lib/embedding-pool";

/**
 * Runs entirely in the webview via WASM (onnxruntime-web) — no network calls after the
 * one-time model download, which transformers.js caches locally for subsequent offline
 * runs. The actual model inference happens off the main thread; see embedding-pool.ts.
 */
export const EMBEDDING_MODEL_ID = "Xenova/all-MiniLM-L6-v2";

/** all-MiniLM-L6-v2's trained max sequence length; longer input is truncated by the model itself. */
export const MODEL_MAX_TOKENS = 256;

let tokenizerPromise: Promise<PreTrainedTokenizer> | null = null;

/** Tokenization is cheap (not the bottleneck) so it stays on the main thread. */
export function getTokenizer(): Promise<PreTrainedTokenizer> {
  if (!tokenizerPromise) {
    tokenizerPromise = AutoTokenizer.from_pretrained(EMBEDDING_MODEL_ID);
  }
  return tokenizerPromise;
}

export function embedText(text: string): Promise<number[]> {
  return embedOne(text);
}

export function embedTexts(texts: string[]): Promise<number[][]> {
  return embedMany(texts);
}
