import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Snapshot } from "$lib/ports/types";

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

export interface Rules {
	version: number;
	portRange: PortRange | null;
	include: Rule[];
	exclude: Rule[];
	hidden: string[];
	protection: Record<string, boolean>;
	autoCleanup: AutoCleanup;
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
	| { kind: "import"; hidden: string[]; protection: Record<string, boolean> };

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

const legacyKeys = { hidden: "ports.hidden", protection: "ports.protection" };

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
		if (hidden === undefined && protection === undefined) return;
		await this.change({ kind: "import", hidden: hidden ?? [], protection: protection ?? {} });
		localStorage.removeItem(legacyKeys.hidden);
		localStorage.removeItem(legacyKeys.protection);
	}

	change = async (change: RulesChange): Promise<Snapshot> => {
		const update = await invoke<{ rules: RulesView; snapshot: Snapshot }>("update_rules", { change });
		this.apply(update.rules);
		return update.snapshot;
	};

	autoCleanupLog = () => invoke<AutoCleanupEntry[]>("auto_cleanup_log");
}

export const rules = new RulesStore();
