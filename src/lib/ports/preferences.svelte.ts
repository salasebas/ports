import { PersistedState } from "runed";
import type { ServerKind } from "./types";

export type Metric = "memory" | "cpu";

export type SortKey = "port" | "name" | "memory" | "cpu" | "uptime";

export interface Sort {
	key: SortKey;
	descending: boolean;
}

export const sortKeys: { value: SortKey; label: string; descending: boolean }[] = [
	{ value: "port", label: "Port", descending: false },
	{ value: "name", label: "Name", descending: false },
	{ value: "memory", label: "Memory", descending: true },
	{ value: "cpu", label: "CPU", descending: true },
	{ value: "uptime", label: "Uptime", descending: true },
];

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
	/** Order of both lists, after the pinned servers. */
	sort: new PersistedState<Sort>("ports.sort", { key: "port", descending: false }),
	/** Groups the main window's table by project folder. */
	groupByProject: new PersistedState("ports.groupByProject", false),
	/** Servers listed first, by `serverKey`. */
	pinned: new PersistedState<string[]>("ports.pinned", []),
	/** Names the user gave ports, by port number. */
	names: new PersistedState<Record<string, string>>("ports.names", {}),
	/** Ports (`port:5000`) and processes (`process:ControlCenter`) left out of the lists. */
	hidden: new PersistedState<string[]>("ports.hidden", []),
	/** The user's choice of which servers clean up skips, by `serverKey`, over the default (databases). */
	protection: new PersistedState<Record<string, boolean>>("ports.protection", {}),
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
