import { PersistedState } from "runed";
import type { ServerKind } from "./types";

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
	/** Ask before stopping or restarting a server; a stopped process can't be brought back. */
	confirmActions: new PersistedState("ports.confirmActions", true),
	/** Which kinds of server the lists and the usage summary include. */
	shownKinds: new PersistedState<ServerKind[]>("ports.shownKinds", ["dev", "database", "container"]),
	/** Editor that "Open in editor" launches. */
	editor: new PersistedState<Editor>("ports.editor", "cursor"),
	/** How often the backend samples the servers, in milliseconds; 0 only on request. */
	refreshInterval: new PersistedState<number>("ports.refreshInterval", 2000),
};

export const refreshIntervals: { value: number; label: string }[] = [
	{ value: 1000, label: "Every 1 s" },
	{ value: 2000, label: "Every 2 s" },
	{ value: 5000, label: "Every 5 s" },
	{ value: 0, label: "Never" },
];

export const serverKinds: { value: ServerKind; label: string }[] = [
	{ value: "dev", label: "Dev servers" },
	{ value: "database", label: "Databases" },
	{ value: "container", label: "Containers" },
	{ value: "other", label: "Apps and system" },
];

export const editorLabel = (editor: Editor) => editors.find((e) => e.value === editor)?.label ?? editor;
