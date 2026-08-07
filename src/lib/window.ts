import { invoke } from "@tauri-apps/api/core";

export async function setWindowSize(width: number, height: number): Promise<void> {
  return invoke("set_window_size", { width, height });
}
