/** Builds a prompt asking the local LLM to translate a raw indexing error into plain
 * English, for the "Explain" action on a failed-to-index file. */
export function buildErrorExplanationPrompt(fileName: string, message: string): string {
  return `A local desktop search app failed to index a file. Explain the likely cause in 1-2 short, plain-English sentences for a non-technical user - avoid jargon like stack traces, object types, or code identifiers. If there's something the user could reasonably do about it (e.g. re-saving the file in a different format), mention it briefly; otherwise just say the file will be skipped.

File name: ${fileName}
Technical error message: ${message}

Plain-English explanation:`;
}
