export interface SnippetPart {
  text: string;
  match: boolean;
}

const MIN_WORD_LEN = 3;

function queryWords(query: string): string[] {
  return Array.from(
    new Set(
      query
        .toLowerCase()
        .split(/\s+/)
        .map((w) => w.replace(/[^\p{L}\p{N}]/gu, ""))
        .filter((w) => w.length >= MIN_WORD_LEN),
    ),
  );
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Extracts a short window from `text` centered on the first literal occurrence of a
 * query word, falling back to the start of the text when nothing matches literally
 * (semantic matches don't always share vocabulary with the query).
 */
export function buildSnippet(text: string, query: string, maxLen = 220): string {
  const words = queryWords(query);
  const lower = text.toLowerCase();

  let matchIndex = -1;
  for (const word of words) {
    const idx = lower.indexOf(word);
    if (idx !== -1 && (matchIndex === -1 || idx < matchIndex)) {
      matchIndex = idx;
    }
  }

  if (matchIndex === -1) {
    const truncated = text.slice(0, maxLen).trim();
    return text.length > maxLen ? `${truncated}…` : truncated;
  }

  const start = Math.max(0, matchIndex - Math.floor(maxLen / 3));
  const end = Math.min(text.length, start + maxLen);
  const prefix = start > 0 ? "…" : "";
  const suffix = end < text.length ? "…" : "";
  return prefix + text.slice(start, end).trim() + suffix;
}

/** Splits `snippet` into parts, flagging the ones that literally match a query word. */
export function highlightParts(snippet: string, query: string): SnippetPart[] {
  const words = queryWords(query);
  if (words.length === 0) return [{ text: snippet, match: false }];

  const pattern = new RegExp(`(${words.map(escapeRegExp).join("|")})`, "gi");
  const parts = snippet.split(pattern);
  const wordSet = new Set(words);

  return parts
    .filter((part) => part.length > 0)
    .map((part) => ({ text: part, match: wordSet.has(part.toLowerCase()) }));
}
