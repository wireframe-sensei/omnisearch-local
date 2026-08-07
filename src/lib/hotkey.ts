import { invoke } from "@tauri-apps/api/core";

// Throws if the combo is already held by another app - the backend falls back to
// re-registering the previous shortcut in that case, so the app is never left with
// no hotkey at all.
export async function applyGlobalShortcut(shortcut: string): Promise<void> {
  await invoke("set_global_shortcut", { shortcut });
}

const MODIFIER_KEYS = new Set(["Alt", "Control", "Shift", "Meta"]);

// Builds a Tauri accelerator string (e.g. "Alt+Shift+Space") from a keydown event, or
// null if the event doesn't represent a valid shortcut (no modifier held, or the key
// itself is a bare modifier press with nothing else yet).
export function shortcutFromKeyboardEvent(e: KeyboardEvent): string | null {
  if (MODIFIER_KEYS.has(e.key)) return null;

  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Control");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Super");

  // Require at least one modifier, matching Spotlight/Alfred/Raycast-style
  // launchers - a bare single key as a *global* hotkey would swallow that key
  // everywhere else on the system.
  if (parts.length === 0) return null;
  if (!e.code) return null;

  parts.push(e.code);
  return parts.join("+");
}

// Formats an accelerator string for display, using macOS symbols on macOS and word
// form elsewhere (⌥⇧Space vs Alt+Shift+Space).
export function formatShortcut(shortcut: string): string {
  const isMac = navigator.platform.toUpperCase().includes("MAC");
  if (!isMac) return shortcut.replace(/\+/g, " + ");

  const SYMBOLS: Record<string, string> = {
    Control: "⌃",
    Alt: "⌥",
    Shift: "⇧",
    Super: "⌘",
  };
  return shortcut
    .split("+")
    .map((part) => SYMBOLS[part] ?? part.replace(/^Key/, "").replace(/^Digit/, ""))
    .join("");
}
