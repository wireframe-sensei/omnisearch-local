import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";

// Opens System Preferences to the Files & Folders privacy section where the user can grant permission.
export async function openFilesPermissionsSettings(): Promise<void> {
  const isMac = navigator.platform.toUpperCase().includes("MAC");
  if (isMac) {
    // x-apple.systempreferences URLs open System Preferences to specific sections.
    // This opens Security & Privacy > Files and Folders.
    await openUrl("x-apple.systempreferences:com.apple.preference.security?Files");
  }
  // Windows/Linux typically auto-grant filesystem permissions, so no action needed.
}

// Shows a dialog prompting the user to grant file access permissions, with a button to open settings.
export async function promptForFilePermissions(): Promise<void> {
  const confirmed = await open({
    title: "Permission Required",
    message:
      "OmniSearch needs permission to access your files to index them.\n\nPlease open System Preferences and grant file access under Security & Privacy.",
    okLabel: "Open Settings",
    cancelLabel: "Skip for now",
  });

  if (confirmed) {
    await openFilesPermissionsSettings();
  }
}
