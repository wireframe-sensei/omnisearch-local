import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface IndexableFile {
  path: string;
  fileName: string;
  extension: string;
  sizeBytes: number;
  modifiedMs: number;
}

export interface ExtractedDocument {
  path: string;
  text: string;
}

export interface FileChangeEvent {
  path: string;
  kind: "created" | "modified" | "removed";
}

export function scanDirectories(directories: string[]): Promise<IndexableFile[]> {
  return invoke("scan_directories_cmd", { directories });
}

export function extractDocumentText(path: string): Promise<ExtractedDocument> {
  return invoke("extract_document_text_cmd", { path });
}

export function startWatching(directories: string[]): Promise<void> {
  return invoke("start_watching", { directories });
}

export function stopWatching(): Promise<void> {
  return invoke("stop_watching");
}

export function onFileChanged(
  callback: (event: FileChangeEvent) => void,
): Promise<UnlistenFn> {
  return listen<FileChangeEvent>("file-changed", (event) => callback(event.payload));
}
