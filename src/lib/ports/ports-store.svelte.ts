import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SvelteSet } from "svelte/reactivity";
import { toast } from "svelte-sonner";
import { canStop, serverUrl } from "./format";
import { preferences, type SortKey } from "./preferences.svelte";
import type { Server, Snapshot, SystemUsage } from "./types";
import { allocateUsage, controlKey, sharesControl } from "./resources";

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
 * and restart share `control`: intersecting process scopes run one action at a time.
 */
type Busy = "control" | "editor" | "terminal" | "folder";

/** What the window saw on the port, so the backend never stops something that took its place. */
const target = (server: Server) => {
	if (!server.target) throw new Error("Ports cannot control this server");
	return server.target;
};

/** A project's ports share one key, so pinning or protecting one covers all of them. */
export const serverKey = (server: Server) => (server.folder ? `project:${server.folder}` : `port:${server.port}`);

/** Entries of `preferences.hidden`: one port, or every port of a process. */
export const hiddenPort = (port: number) => `port:${port}`;
export const hiddenProcess = (process: string) => `process:${process}`;

/** Ascending order for each sort key; `uptime` puts the longest running first. */
const ascending: Record<SortKey, (a: Server, b: Server) => number> = {
	port: (a, b) => a.port - b.port,
	name: (a, b) => a.name.localeCompare(b.name),
	memory: (a, b) => a.memoryBytes - b.memoryBytes,
	cpu: (a, b) => a.cpuPercent - b.cpuPercent,
	uptime: (a, b) => b.startedAt - a.startedAt,
};

/** The servers of one project folder; `folder` is undefined for servers outside any project. */
export interface ServerGroup {
	folder?: string;
	servers: Server[];
}

/** The server with the name and protection the user gave it, or the same object when there are none. */
function customize(server: Server): Server {
	const name = preferences.names.current[server.port];
	const isProtected = preferences.protection.current[serverKey(server)];
	if (name === undefined && isProtected === undefined) return server;
	return { ...server, name: name ?? server.name, protected: isProtected ?? server.protected };
}

export function unhide(entry: string) {
	preferences.hidden.current = preferences.hidden.current.filter((e) => e !== entry);
}

/** `:5000` or `ControlCenter`, for the list of hidden entries. */
export const hiddenLabel = (entry: string) => entry.replace(/^port:/, ":").replace(/^process:/, "");

const idleCount = (n: number) => (n === 1 ? "1 idle server" : `${n} idle servers`);

function isHidden(server: Server) {
	const hidden = preferences.hidden.current;
	return hidden.includes(hiddenPort(server.port)) || hidden.includes(hiddenProcess(server.process));
}

/** UI state for the server list, fed by the snapshots the Rust backend publishes. */
export class PortsStore {
	servers = $state.raw<Server[]>([]);
	system = $state.raw<SystemUsage>({ totalBytes: 0, usedBytes: 0, cpuPercent: 0 });
	#revision = -1;
	query = $state("");
	selectedId = $state<string | null>(null);
	/** Action waiting for the user to confirm it. */
	pending = $state.raw<{ action: ServerAction; server: Server } | null>(null);
	/** Idle servers clean up will stop once the user confirms. */
	cleaning = $state.raw<Server[] | null>(null);
	/** Server whose name the user is editing. */
	renaming = $state.raw<Server | null>(null);
	/** Work in progress, keyed by action and service identity. */
	#busy = new SvelteSet<string>();
	#controlling = $state.raw<Server[]>([]);

	#customized = $derived(this.servers.map(customize));

	/** Servers of the kinds the user shows, without the ones they hid. */
	shown = $derived(
		this.#customized.filter((server) => preferences.shownKinds.current.includes(server.kind) && !isHidden(server)),
	);

	/** The shown servers matching the search: pinned ones first, then in the chosen order. */
	filtered = $derived.by(() => {
		const q = this.query.trim().toLowerCase();
		const matching = q
			? this.shown.filter((server) =>
					[server.port, server.name, server.process, server.framework, server.branch]
						.filter(Boolean)
						.some((field) => String(field).toLowerCase().includes(q)),
				)
			: this.shown;
		const { key, descending } = preferences.sort.current;
		const pinned = preferences.pinned.current;
		const rank = (server: Server) => (pinned.includes(serverKey(server)) ? 0 : 1);
		return matching.toSorted(
			(a, b) => rank(a) - rank(b) || (descending ? -1 : 1) * ascending[key](a, b) || ascending.port(a, b),
		);
	});

	/** `filtered` by project folder, in the order of each group's first server; servers outside a project come last. */
	groups = $derived.by(() => {
		const groups = new Map<string | undefined, ServerGroup>();
		for (const server of this.filtered) {
			const group = groups.get(server.folder) ?? { folder: server.folder, servers: [] };
			group.servers.push(server);
			groups.set(server.folder, group);
		}
		return [...groups.values()].toSorted((a, b) => Number(!a.folder) - Number(!b.folder));
	});

	selected = $derived(this.#customized.find((server) => server.id === this.selectedId) ?? null);

	/** What clean up stops: shown idle servers Ports can stop, unless protected. */
	idle = $derived(
		this.shown.filter(
			(server) =>
				canStop(server) &&
				this.#customized
					.filter((sibling) => sharesControl(server, sibling))
					.every(
						(sibling) =>
							sibling.status === "idle" &&
							!sibling.protected &&
							!isHidden(sibling) &&
							preferences.shownKinds.current.includes(sibling.kind),
					),
		),
	);

	allocations = $derived(allocateUsage(this.shown));

	/** Hidden processes belong to Other apps; shared trees count once. */
	usage = $derived.by(() => {
		const serversBytes = this.allocations.reduce((sum, row) => sum + row.usage.memoryBytes, 0);
		const serversCpuPercent = this.allocations.reduce((sum, row) => sum + row.usage.cpuPercent, 0);
		return {
			serversBytes,
			serversCpuPercent,
			otherAppsBytes: Math.max(0, this.system.usedBytes - serversBytes),
			otherAppsCpuPercent: Math.max(0, this.system.cpuPercent - serversCpuPercent),
		};
	});

	/** Follows the backend's snapshots until the returned cleanup runs. */
	connect = () => {
		// A plain browser tab on the dev server has no backend to follow.
		if (!isTauri()) return () => {};
		let connected = true;
		const unlisten = listen<Snapshot>(SNAPSHOT_EVENT, (event) => {
			if (connected) this.apply(event.payload);
		});
		void unlisten
			.then(() => (connected ? invoke<Snapshot>("snapshot") : null))
			.then((snapshot) => {
				if (connected && snapshot) this.apply(snapshot);
			})
			.catch((error: unknown) => {
				if (connected) toast.error(String(error));
			});
		void attempt(invoke("set_refresh_interval", { ms: preferences.refreshInterval.current }));
		return () => {
			connected = false;
			void unlisten.then((stop) => stop()).catch(() => {});
		};
	};

	apply = (snapshot: Snapshot) => {
		if (snapshot.revision <= this.#revision) return;
		this.#revision = snapshot.revision;
		this.servers = snapshot.servers;
		this.system = snapshot.system;
	};

	select = (id: string | null) => {
		this.selectedId = id;
	};

	refresh = async () => {
		try {
			this.apply(await invoke<Snapshot>("refresh"));
		} catch (error) {
			toast.error(String(error));
		}
	};

	isBusy = (server: Server, work: Busy) =>
		work === "control"
			? this.#controlling.some((busy) => sharesControl(busy, server))
			: this.#busy.has(`${work}:${server.id}`);

	/** Runs `work` unless the same work on the same server, or a control of its processes, is still running. */
	async #run<T>(server: Server, work: Busy, run: () => Promise<T>): Promise<T | undefined> {
		if (this.isBusy(server, work)) return;
		const key = `${work}:${server.id}`;
		if (work === "control") this.#controlling = [...this.#controlling, server];
		else this.#busy.add(key);
		try {
			return await run();
		} finally {
			if (work === "control") this.#controlling = this.#controlling.filter((busy) => busy !== server);
			else this.#busy.delete(key);
		}
	}

	isPinned = (server: Server) => preferences.pinned.current.includes(serverKey(server));

	togglePin = (server: Server) => {
		const key = serverKey(server);
		const pinned = preferences.pinned.current;
		preferences.pinned.current = pinned.includes(key) ? pinned.filter((k) => k !== key) : [...pinned, key];
	};

	/** The name Ports detected, before the user renamed the port. */
	detectedName = (port: number) => this.servers.find((server) => server.port === port)?.name ?? "";

	/** Gives the port a name; a blank one, or the detected one, goes back to the detected name. */
	rename = (port: number, name: string) => {
		const names = { ...preferences.names.current };
		delete names[port];
		const trimmed = name.trim();
		preferences.names.current = trimmed && trimmed !== this.detectedName(port) ? { ...names, [port]: trimmed } : names;
	};

	setProtected = (server: Server, value: boolean) => {
		preferences.protection.current = { ...preferences.protection.current, [serverKey(server)]: value };
	};

	hide = (entry: string) => {
		if (!preferences.hidden.current.includes(entry))
			preferences.hidden.current = [...preferences.hidden.current, entry];
		if (this.selected && isHidden(this.selected)) this.selectedId = null;
	};

	open = async (server: Server) => {
		await attempt(openUrl(serverUrl(server)));
	};

	copyUrl = async (server: Server, url = serverUrl(server)) => {
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
			const id = toast.loading(`Restarting ${server.name}…`, { id: `restart-${server.id}` });
			try {
				this.apply(await invoke<Snapshot>("restart_server", { target: target(server) }));
				// The new process has a new identity; keep its details open.
				if (this.selectedId === server.id && !this.selected) {
					this.selectedId =
						this.#customized.find(
							(row) => row.port === server.port && row.process === server.process && row.folder === server.folder,
						)?.id ?? null;
				}
				toast.success(`Restarted ${server.name}`, { id, description: `:${server.port}` });
			} catch (error) {
				toast.error(String(error), { id });
			}
		});

	stop = (server: Server) =>
		this.#run(server, "control", async () => {
			const id = toast.loading(`Stopping ${server.name}…`, { id: `stop-${server.id}` });
			const error = await this.#stop(server);
			if (error) toast.error(error, { id });
			else toast.success(`Stopped ${server.name}`, { id, description: `:${server.port}` });
		});

	/** Stops the server and takes it off the list; returns why it couldn't. */
	async #stop(server: Server, onlyIfIdle = false): Promise<string | null> {
		try {
			this.apply(
				await invoke<Snapshot>("stop_server", {
					target: target(server),
					onlyIfIdle,
					...(onlyIfIdle ? { allowedIds: this.idle.map((row) => row.id) } : {}),
				}),
			);
		} catch (error) {
			return String(error);
		}
		if (this.selectedId === server.id && !this.selected) this.selectedId = null;
		return null;
	}

	/** Asks once before stopping every idle server; protected ones are never listed. */
	cleanUp = () => {
		if (this.idle.length) this.cleaning = this.idle;
	};

	confirmCleanUp = async () => {
		const captured = this.cleaning ?? [];
		this.cleaning = null;
		const eligible = captured
			.flatMap((old) => {
				const current = this.idle.find((server) => server.id === old.id && controlKey(server) === controlKey(old));
				return current ? [current] : [];
			})
			.toSorted((a, b) => b.resources.length - a.resources.length);
		const servers: Server[] = [];
		for (const server of eligible) {
			if (!servers.some((chosen) => sharesControl(chosen, server))) servers.push(server);
		}
		if (!servers.length) return;
		// Count rows, like the dialog: one stop can end several ports of a process.
		const rows = (stopped: Server[]) => eligible.filter((row) => stopped.some((s) => sharesControl(s, row))).length;
		const id = toast.loading("Stopping " + idleCount(eligible.length) + "…", { id: "clean-up" });
		const results = await Promise.all(
			servers.map((server) => this.#run(server, "control", () => this.#stop(server, true))),
		);
		const attempted = servers.filter((_, i) => results[i] !== undefined);
		const errors = results.filter((result): result is string => typeof result === "string");
		const failed = servers.filter((_, i) => typeof results[i] === "string");
		if (!attempted.length) toast.info("No idle servers were stopped", { id });
		else if (errors.length)
			toast.error("Couldn't stop " + rows(failed) + " of " + idleCount(rows(attempted)), {
				id,
				description: errors[0],
			});
		else toast.success("Stopped " + idleCount(rows(attempted)), { id });
	};
}
