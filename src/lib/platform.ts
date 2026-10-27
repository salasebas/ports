export const isMac = navigator.userAgent.includes("Mac");

/** Matches the shortcut registered in `src-tauri/src/lib.rs`. */
export const popoverShortcut = isMac ? ["⌥", "⌘", "P"] : ["Ctrl", "Alt", "P"];

export const fileManagerName = isMac ? "Finder" : "Files";
