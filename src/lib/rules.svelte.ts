import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Snapshot } from "$lib/ports/types";

/** Must match `RULES_EVENT` in `src-tauri/src/ports/rules.rs`. */
export const RULES_EVENT = "ports://rules";

/** Mirrors `Field` in `src-tauri/src/ports/rules.rs`. */
export type RuleField = "process" | "command";

/** Matches when `pattern` appears in the server's process name or command, ignoring case. */
export interface Rule {
	field: RuleField;
	pattern: string;
}

export interface PortRange {
	start: number;
	end: number;
}

/** `deleted`: only idle servers whose folder was deleted. `idle`: every idle dev server. */
export type AutoCleanupScope = "deleted" | "idle";

export interface AutoCleanupPolicy {
	scope: AutoCleanupScope;
	idleMinutes: number;
}

export interface AutoCleanup {
	/** Null while automatic clean up is off, its default. */
	policy: AutoCleanupPolicy | null;
	paused: boolean;
}

/** Mirrors `Rules` in `src-tauri/src/ports/rules.rs`. */
export interface Rules {
	version: number;
	portRange: PortRange | null;
	include: Rule[];
	exclude: Rule[];
	/** Ports (`port:5000`) and processes (`process:ControlCenter`) left out of the lists. */
	hidden: string[];
	/** The user's choice of which servers clean up skips, by `serverKey`, over the default (databases). */
	protection: Record<string, boolean>;
	autoCleanup: AutoCleanup;
}

/** Mirrors `RulesView` in `src-tauri/src/ports/rules.rs`. */
export interface RulesView {
	revision: number;
	rules: Rules;
}

/** Mirrors `Change` in `src-tauri/src/ports/rules.rs`. */
export type RulesChange =
	| { kind: "hide"; entry: string }
	| { kind: "unhide"; entry: string }
	| { kind: "protect"; key: string; value: boolean }
	| { kind: "portRange"; range: PortRange | null }
	| { kind: "filters"; include: Rule[]; exclude: Rule[] }
	| { kind: "autoCleanup"; policy: AutoCleanupPolicy | null; paused: boolean }
	| { kind: "import"; hidden: string[]; protection: Record<string, boolean> };

/** One automatic stop. Mirrors `Entry` in `src-tauri/src/ports/autoclean.rs`. */
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

/** Choices for `AutoCleanupPolicy.idleMinutes`, within the backend's 15 minutes to 24 hours. */
export const autoIdleChoices = [30, 60, 120, 240, 480, 1440].map((minutes) => ({
	value: String(minutes),
	label: minutes < 60 ? `${minutes} minutes` : minutes === 60 ? "1 hour" : `${minutes / 60} hours`,
}));

/** Where the windows kept hidden entries and protection before the backend owned them. */
const legacyKeys = { hidden: "ports.hidden", protection: "ports.protection" };

function readLegacy<T>(key: string): T | undefined {
	try {
		const value = localStorage.getItem(key);
		return value === null ? undefined : (JSON.parse(value) as T);
	} catch {
		return undefined;
	}
}

/**
 * The rules the backend applies to every snapshot: filters, hidden servers,
 * protection and automatic clean up. Each window loads them once and follows
 * the changes made in any window.
 */
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

	/** Moves hidden entries and protection out of this window's storage, once. */
	async #importLegacy() {
		const hidden = readLegacy<string[]>(legacyKeys.hidden);
		const protection = readLegacy<Record<string, boolean>>(legacyKeys.protection);
		if (hidden === undefined && protection === undefined) return;
		await this.change({ kind: "import", hidden: hidden ?? [], protection: protection ?? {} });
		localStorage.removeItem(legacyKeys.hidden);
		localStorage.removeItem(legacyKeys.protection);
	}

	/** Rejects with the reason when the backend refuses the change; the saved rules stay. */
	change = async (change: RulesChange): Promise<Snapshot> => {
		const update = await invoke<{ rules: RulesView; snapshot: Snapshot }>("update_rules", { change });
		this.apply(update.rules);
		return update.snapshot;
	};

	autoCleanupLog = () => invoke<AutoCleanupEntry[]>("auto_cleanup_log");
}

export const rules = new RulesStore();
