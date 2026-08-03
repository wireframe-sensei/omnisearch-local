import type { FileResult } from "@/lib/hybrid-search";

const MAX_CONTEXT_RESULTS = 5;

/** Builds a RAG-style prompt that constrains the model to only the retrieved excerpts. */
export function buildAnswerPrompt(query: string, results: FileResult[]): string {
  const excerpts = results
    .slice(0, MAX_CONTEXT_RESULTS)
    .map((r, i) => `[${i + 1}] (${r.fileName}):\n${r.fullText}`)
    .join("\n\n");

  return `You are answering a question using ONLY the excerpts below from the user's local files. If the excerpts don't contain enough information to answer, say so plainly instead of guessing or using outside knowledge.

Excerpts:
${excerpts}

Question: ${query}

Answer concisely in a few sentences, and mention which excerpt number(s) you used, e.g. "[1]".`;
}
