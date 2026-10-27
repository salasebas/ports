import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SvelteSet } from "svelte/reactivity";
import { toast } from "svelte-sonner";
import { serverUrl } from "./format";
import { preferences } from "./preferences.svelte";
import type { Server, ServerKind, Snapshot, SystemUsage, Usage } from "./types";

/** Must match `SNAPSHOT_EVENT` in `src-tauri/src/ports/mod.rs`. */
export const SNAPSHOT_EVENT = "ports://snapshot";

/** Runs a system action and reports its failure, e.g. a missing folder or editor, as a toast. */
async function attempt(action: Promise<unknown>): Promise<boolean> {
	try {
		await action;
		return true;
	} catch (error) {
		toast.error(String(error));
		return false;
	}
}

/** Saves the interval and tells the backend; the other window picks the preference up through storage events. */
export function setRefreshInterval(ms: number) {
	preferences.refreshInterval.current = ms;
	return attempt(invoke("set_refresh_interval", { ms }));
}

export type ServerAction = "stop" | "restart";

/**
 * Work a button starts; its button stays disabled until the work ends. Stop
 * and restart share `control`: the backend runs one of them per port at a time.
 */
type Busy = "control" | "editor" | "terminal" | "folder";

/** What the window saw on the port, so the backend never stops something that took its place. */
const target = (server: Server) => ({ port: server.port, pid: server.pid, container: server.container?.id });

/** UI state for the server list, fed by the snapshots the Rust backend publishes. */
export class PortsStore {
	servers = $state.raw<Server[]>([]);
	system = $state.raw<SystemUsage>({ totalBytes: 0, kinds: {}, otherAppsBytes: 0, otherAppsCpuPercent: 0 });
	query = $state("");
	selectedPort = $state<number | null>(null);
	/** Action waiting for the user to confirm it. */
	pending = $state.raw<{ action: ServerAction; server: Server } | null>(null);
	/** Work in progress, keyed `action:port`. */
	#busy = new SvelteSet<string>();

	/** Servers of the kinds the user shows. */
	shown = $derived(this.servers.filter((server) => preferences.shownKinds.current.includes(server.kind)));

	filtered = $derived.by(() => {
		const q = this.query.trim().toLowerCase();
		if (!q) return this.shown;
		return this.shown.filter((server) =>
			[server.port, server.name, server.process, server.framework, server.branch]
				.filter(Boolean)
				.some((field) => String(field).toLowerCase().includes(q)),
		);
	});

	selected = $derived(this.servers.find((server) => server.port === this.selectedPort) ?? null);

	/** Totals for the shown kinds; hidden servers count as other apps. */
	usage = $derived.by(() => {
		const shown = preferences.shownKinds.current;
		const usage = {
			serversBytes: 0,
			serversCpuPercent: 0,
			otherAppsBytes: this.system.otherAppsBytes,
			otherAppsCpuPercent: this.system.otherAppsCpuPercent,
		};
		for (const [kind, { memoryBytes, cpuPercent }] of Object.entries(this.system.kinds) as [ServerKind, Usage][]) {
			if (shown.includes(kind)) {
				usage.serversBytes += memoryBytes;
				usage.serversCpuPercent += cpuPercent;
			} else {
				usage.otherAppsBytes += memoryBytes;
				usage.otherAppsCpuPercent += cpuPercent;
			}
		}
		return usage;
	});

	/** Follows the backend's snapshots until the returned cleanup runs. */
	connect = () => {
		// A plain browser tab on the dev server has no backend to follow.
		if (!isTauri()) return () => {};
		let pushed = false;
		const unlisten = listen<Snapshot>(SNAPSHOT_EVENT, (event) => {
			pushed = true;
			this.apply(event.payload);
		});
		invoke<Snapshot>("snapshot").then(
			// A pushed snapshot is newer than the one this call started from.
			(snapshot) => pushed || this.apply(snapshot),
			(error) => toast.error(String(error)),
		);
		void attempt(invoke("set_refresh_interval", { ms: preferences.refreshInterval.current }));
		return () => void unlisten.then((stop) => stop());
	};

	apply = (snapshot: Snapshot) => {
		this.servers = snapshot.servers;
		this.system = snapshot.system;
	};

	select = (port: number | null) => {
		this.selectedPort = port;
	};

	refresh = async () => {
		try {
			this.apply(await invoke<Snapshot>("refresh"));
		} catch (error) {
			toast.error(String(error));
		}
	};

	isBusy = (server: Server, work: Busy) => this.#busy.has(`${work}:${server.port}`);

	/** Runs `work` unless the same work on the same server is still running. */
	async #run(server: Server, work: Busy, run: () => Promise<unknown>) {
		const key = `${work}:${server.port}`;
		if (this.#busy.has(key)) return;
		this.#busy.add(key);
		try {
			await run();
		} finally {
			this.#busy.delete(key);
		}
	}

	open = async (server: Server) => {
		await openUrl(serverUrl(server.port));
	};

	copyUrl = async (server: Server) => {
		const url = serverUrl(server.port);
		if (await attempt(navigator.clipboard.writeText(url))) {
			toast.success("Copied", { id: "copy-url", description: url });
		}
	};

	openInEditor = (server: Server) =>
		this.#run(server, "editor", () =>
			attempt(invoke("open_in_editor", { folder: server.folder, editor: preferences.editor.current })),
		);

	openInTerminal = (server: Server) =>
		this.#run(server, "terminal", () => attempt(invoke("open_in_terminal", { folder: server.folder })));

	revealFolder = (server: Server) =>
		this.#run(server, "folder", () => attempt(invoke("reveal_folder", { folder: server.folder })));

	/** Entry point for every stop and restart button: asks first when the preference is on. */
	request = async (action: ServerAction, server: Server) => {
		if (this.isBusy(server, "control")) return;
		if (preferences.confirmActions.current) this.pending = { action, server };
		else await this[action](server);
	};

	confirm = async () => {
		const pending = this.pending;
		this.pending = null;
		if (pending) await this[pending.action](pending.server);
	};

	restart = (server: Server) =>
		this.#run(server, "control", async () => {
			const id = toast.loading(`Restarting ${server.name}…`, { id: `restart-${server.port}` });
			try {
				await invoke("restart_server", { target: target(server) });
				toast.success(`Restarted ${server.name}`, { id, description: `:${server.port}` });
			} catch (error) {
				toast.error(String(error), { id });
			}
		});

	stop = (server: Server) =>
		this.#run(server, "control", async () => {
			const id = toast.loading(`Stopping ${server.name}…`, { id: `stop-${server.port}` });
			try {
				await invoke("stop_server", { target: target(server) });
			} catch (error) {
				toast.error(String(error), { id });
				return;
			}
			this.servers = this.servers.filter((s) => s.port !== server.port);
			if (this.selectedPort === server.port) this.selectedPort = null;
			toast.success(`Stopped ${server.name}`, { id, description: `:${server.port} is free` });
		});
}
