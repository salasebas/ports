import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ServerKind, Snapshot } from "$lib/ports/types";

export const RULES_EVENT = "ports://rules";

export type RuleField = "process" | "command";

export interface Rule {
	field: RuleField;
	pattern: string;
}

export interface PortRange {
	start: number;
	end: number;
}

export type AutoCleanupScope = "deleted" | "idle";

export interface AutoCleanupPolicy {
	scope: AutoCleanupScope;
	idleMinutes: number;
}

export interface AutoCleanup {
	policy: AutoCleanupPolicy | null;
	paused: boolean;
}

export interface Snooze {
	until: number;
	process: string;
}

export type WatchEvent = "started" | "stopped" | "health";

export interface Rules {
	version: number;
	portRange: PortRange | null;
	include: Rule[];
	exclude: Rule[];
	hidden: string[];
	protection: Record<string, boolean>;
	autoCleanup: AutoCleanup;
	shownKinds: ServerKind[];
	snoozes: Record<string, Snooze>;
	watches: Record<string, WatchEvent[]>;
}

export interface RulesView {
	revision: number;
	rules: Rules;
}

export type RulesChange =
	| { kind: "resetLists" }
	| { kind: "hide"; entry: string }
	| { kind: "unhide"; entry: string }
	| { kind: "protect"; key: string; value: boolean }
	| { kind: "portRange"; range: PortRange | null }
	| { kind: "filters"; include: Rule[]; exclude: Rule[] }
	| { kind: "autoCleanup"; policy: AutoCleanupPolicy | null; paused: boolean }
	| { kind: "import"; hidden: string[]; protection: Record<string, boolean>; shownKinds: ServerKind[] | null }
	| { kind: "shownKinds"; kinds: ServerKind[] }
	| { kind: "snooze"; key: string; process: string; minutes: number }
	| { kind: "unsnooze"; key: string }
	| { kind: "clearSnoozes" }
	| { kind: "watch"; key: string; events: WatchEvent[] }
	| { kind: "clearWatches" };

export interface AutoCleanupEntry {
	at: number;
	port: number;
	name: string;
	folder?: string;
	cwdDeleted: boolean;
	idleMinutes: number;
	outcome: "stopped" | "skipped" | "failed";
	message?: string;
}

export const autoIdleChoices = [30, 60, 120, 240, 480, 1440].map((minutes) => ({
	value: String(minutes),
	label: minutes < 60 ? `${minutes} minutes` : minutes === 60 ? "1 hour" : `${minutes / 60} hours`,
}));

export const snoozeChoices = [
	{ minutes: 60, label: "1 hour" },
	{ minutes: 8 * 60, label: "8 hours" },
	{ minutes: 24 * 60, label: "24 hours" },
];

export const watchEvents: { value: WatchEvent; label: string }[] = [
	{ value: "started", label: "Starts" },
	{ value: "stopped", label: "Stops" },
	{ value: "health", label: "Health" },
];

export const watchLabel = (key: string) =>
	key.startsWith("port:") ? `:${key.slice("port:".length)}` : (key.split("/").at(-1) ?? key);

const legacyKeys = { hidden: "ports.hidden", protection: "ports.protection", shownKinds: "ports.shownKinds" };

function readLegacy<T>(key: string): T | undefined {
	try {
		const value = localStorage.getItem(key);
		return value === null ? undefined : (JSON.parse(value) as T);
	} catch {
		return undefined;
	}
}

export class RulesStore {
	current = $state.raw<RulesView>({
		revision: -1,
		rules: {
			version: 1,
			portRange: null,
			include: [],
			exclude: [],
			hidden: [],
			protection: {},
			autoCleanup: { policy: null, paused: false },
			shownKinds: ["dev", "database", "container"],
			snoozes: {},
			watches: {},
		},
	});
	#loaded = false;

	apply = (view: RulesView) => {
		if (view.revision > this.current.revision) this.current = view;
	};

	load = () => {
		if (this.#loaded || !isTauri()) return;
		this.#loaded = true;
		void (async () => {
			let stop: (() => void) | undefined;
			try {
				stop = await listen<RulesView>(RULES_EVENT, (event) => this.apply(event.payload));
				this.apply(await invoke<RulesView>("get_rules"));
				await this.#importLegacy();
			} catch (error) {
				stop?.();
				this.#loaded = false;
				console.error(error);
			}
		})();
	};

	async #importLegacy() {
		const hidden = readLegacy<string[]>(legacyKeys.hidden);
		const protection = readLegacy<Record<string, boolean>>(legacyKeys.protection);
		const shownKinds = readLegacy<ServerKind[]>(legacyKeys.shownKinds);
		if (hidden === undefined && protection === undefined && shownKinds === undefined) return;
		await this.change({
			kind: "import",
			hidden: hidden ?? [],
			protection: protection ?? {},
			shownKinds: shownKinds ?? null,
		});
		localStorage.removeItem(legacyKeys.hidden);
		localStorage.removeItem(legacyKeys.protection);
		localStorage.removeItem(legacyKeys.shownKinds);
	}

	change = async (change: RulesChange): Promise<Snapshot> => {
		const update = await invoke<{ rules: RulesView; snapshot: Snapshot }>("update_rules", { change });
		this.apply(update.rules);
		return update.snapshot;
	};

	autoCleanupLog = () => invoke<AutoCleanupEntry[]>("auto_cleanup_log");
}

export const rules = new RulesStore();
