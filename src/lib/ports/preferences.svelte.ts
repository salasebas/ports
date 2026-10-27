import { PersistedState } from "runed";

export type Metric = "memory" | "cpu";

/** Must match `folders::Editor` in the Rust backend. */
export type Editor = "cursor" | "vscode" | "zed";

export const editors: { value: Editor; label: string }[] = [
	{ value: "cursor", label: "Cursor" },
	{ value: "vscode", label: "VS Code" },
	{ value: "zed", label: "Zed" },
];

/**
 * User preferences. Both windows share this origin's localStorage, and
 * PersistedState listens to storage events, so a change in one window
 * reaches the other.
 */
export const preferences = {
	/** Which resource the summary and the popover rows show. */
	metric: new PersistedState<Metric>("ports.metric", "memory"),
	/** Ask before stopping a server; a stopped process can't be brought back. */
	confirmStop: new PersistedState("ports.confirmStop", true),
	/** Editor that "Open in editor" launches. */
	editor: new PersistedState<Editor>("ports.editor", "cursor"),
};

export const editorLabel = (editor: Editor) => editors.find((e) => e.value === editor)?.label ?? editor;
