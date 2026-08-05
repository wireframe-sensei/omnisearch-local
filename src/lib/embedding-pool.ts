import type { EmbedRequest, EmbedResponse } from "@/workers/embedding-worker";

/**
 * A small pool of embedding Web Workers, so bulk indexing embeds multiple chunks
 * concurrently (real parallelism across CPU cores) instead of one at a time on the
 * main thread. Capped at 4: each worker loads its own copy of the ~30MB model, and
 * embedding is CPU-bound, so more workers than cores buys nothing but memory.
 *
 * Deliberately leaves one core free rather than using every available core - this pool
 * runs continuously in the background for as long as there's an embedding backlog
 * (potentially hours on a large index), and using every core with no headroom made the
 * whole system feel unresponsive, not just this app slow.
 */
const POOL_SIZE = Math.max(1, Math.min((navigator.hardwareConcurrency || 4) - 1, 4));

interface WorkerSlot {
  worker: Worker;
  busy: boolean;
  /** id of the task currently in flight on this worker, so a crash can reject the right one. */
  currentTaskId: number | null;
}

interface PendingTask {
  resolve: (embedding: number[]) => void;
  reject: (err: Error) => void;
}

let slots: WorkerSlot[] | null = null;
/** Set once pool initialization fails, so we stop retrying and use the fallback for good. */
let poolBroken = false;
let nextTaskId = 0;
const pending = new Map<number, PendingTask>();
const queue: { id: number; text: string }[] = [];

function failSlotTask(slot: WorkerSlot, error: Error) {
  const taskId = slot.currentTaskId;
  slot.currentTaskId = null;
  slot.busy = false;
  if (taskId === null) return;
  const task = pending.get(taskId);
  pending.delete(taskId);
  task?.reject(error);
}

function createSlot(): WorkerSlot {
  const worker = new Worker(new URL("../workers/embedding-worker.ts", import.meta.url), {
    type: "module",
  });
  const slot: WorkerSlot = { worker, busy: false, currentTaskId: null };

  worker.onmessage = (event: MessageEvent<EmbedResponse>) => {
    const { id, embedding, error } = event.data;
    const task = pending.get(id);
    pending.delete(id);
    slot.busy = false;
    slot.currentTaskId = null;

    if (task) {
      if (error) task.reject(new Error(error));
      else task.resolve(embedding ?? []);
    }
    dispatchNext();
  };

  // Without this, a worker that fails to load or throws during setup leaves its
  // in-flight (and all future) tasks hanging forever with no visible error at all.
  worker.onerror = (event: ErrorEvent) => {
    console.error("Embedding worker error:", event.message || event);
    failSlotTask(slot, new Error(event.message || "Embedding worker crashed"));
    dispatchNext();
  };

  return slot;
}

function ensurePool(): WorkerSlot[] | null {
  if (poolBroken) return null;
  if (slots) return slots;

  try {
    slots = Array.from({ length: POOL_SIZE }, createSlot);
    return slots;
  } catch (e) {
    console.error("Failed to start embedding worker pool, falling back to main thread:", e);
    poolBroken = true;
    slots = null;
    return null;
  }
}

function dispatchNext() {
  if (queue.length === 0 || !slots) return;
  const idle = slots.find((s) => !s.busy);
  if (!idle) return;

  const task = queue.shift()!;
  idle.busy = true;
  idle.currentTaskId = task.id;
  const request: EmbedRequest = { id: task.id, text: task.text };
  idle.worker.postMessage(request);
}

/** Main-thread fallback used only if the worker pool itself couldn't be created. */
async function embedOneOnMainThread(text: string): Promise<number[]> {
  const { pipeline } = await import("@huggingface/transformers");
  const { EMBEDDING_MODEL_ID } = await import("@/lib/embeddings");
  const extractor = await pipeline("feature-extraction", EMBEDDING_MODEL_ID);
  const output = await extractor(text, { pooling: "mean", normalize: true });
  return Array.from(output.data as Float32Array);
}

/** Embeds a single piece of text via the worker pool. */
export function embedOne(text: string): Promise<number[]> {
  const pool = ensurePool();
  if (!pool) return embedOneOnMainThread(text);

  const id = nextTaskId++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    queue.push({ id, text });
    dispatchNext();
  });
}

/**
 * Embeds many texts concurrently across the pool. Order of the returned array
 * matches the input order, regardless of which worker finishes first.
 */
export function embedMany(texts: string[]): Promise<number[][]> {
  return Promise.all(texts.map(embedOne));
}
