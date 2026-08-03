import type { FileResult } from "@/lib/hybrid-search";

export interface AnswerTextPart {
  type: "text";
  text: string;
}

export interface AnswerCitationPart {
  type: "citation";
  /** 1-based excerpt numbers as sent to the model, e.g. [1] or [1, 2]. */
  indices: number[];
}

export type AnswerPart = AnswerTextPart | AnswerCitationPart;

const CITATION_PATTERN = /\[(\d+(?:\s*,\s*\d+)*)\]/g;

/** Splits a streamed answer into plain text and `[n]`/`[n, m]` citation markers. */
export function parseAnswerCitations(answer: string): AnswerPart[] {
  const parts: AnswerPart[] = [];
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  CITATION_PATTERN.lastIndex = 0;
  while ((match = CITATION_PATTERN.exec(answer)) !== null) {
    if (match.index > lastIndex) {
      parts.push({ type: "text", text: answer.slice(lastIndex, match.index) });
    }
    const indices = match[1].split(",").map((n) => parseInt(n.trim(), 10));
    parts.push({ type: "citation", indices });
    lastIndex = match.index + match[0].length;
  }

  if (lastIndex < answer.length) {
    parts.push({ type: "text", text: answer.slice(lastIndex) });
  }

  return parts;
}

function parentFolderName(path: string): string {
  const segments = path.split(/[/\\]/).filter(Boolean);
  return segments.length >= 2 ? segments[segments.length - 2] : "";
}

/** Same filename can appear more than once across different folders — disambiguate those. */
export function citationLabel(result: FileResult, allResults: FileResult[]): string {
  const sameName = allResults.filter((r) => r.fileName === result.fileName);
  if (sameName.length <= 1) return result.fileName;
  return `${result.fileName} (${parentFolderName(result.path)})`;
}
