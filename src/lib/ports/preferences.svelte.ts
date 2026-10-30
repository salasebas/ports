import { PersistedState } from "runed";
import { groupByFromStorage, type GroupBy } from "./grouping";
import type { ServerKind } from "./types";

export type Metric = "memory" | "cpu";

export type SortKey = "port" | "name" | "memory" | "cpu" | "uptime";

export type { GroupBy } from "./grouping";

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

export type Editor = "cursor" | "vscode" | "zed";

export const editors: { value: Editor; label: string }[] = [
	{ value: "cursor", label: "Cursor" },
	{ value: "vscode", label: "VS Code" },
	{ value: "zed", label: "Zed" },
];

export const preferences = {
	metric: new PersistedState<Metric>("ports.metric", "memory"),
	confirmActions: new PersistedState("ports.confirmActions", true),
	shownKinds: new PersistedState<ServerKind[]>("ports.shownKinds", ["dev", "database", "container"]),
	editor: new PersistedState<Editor>("ports.editor", "cursor"),
	refreshInterval: new PersistedState<number>("ports.refreshInterval", 2000),
	sort: new PersistedState<Sort>("ports.sort", { key: "port", descending: false }),
	groupBy: new PersistedState<GroupBy>(
		"ports.groupBy",
		groupByFromStorage(null, typeof localStorage === "undefined" ? null : localStorage.getItem("ports.groupByProject")),
	),
	collapsedGroups: new PersistedState<string[]>("ports.collapsedGroups", []),
	pinned: new PersistedState<string[]>("ports.pinned", []),
	names: new PersistedState<Record<string, string>>("ports.names", {}),
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
