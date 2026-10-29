import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { homeDir } from "@tauri-apps/api/path";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SvelteSet } from "svelte/reactivity";
import { toast } from "svelte-sonner";
import { canStop, serverUrl } from "./format";
import { preferences, type SortKey } from "./preferences.svelte";
import { rules, type RulesChange } from "$lib/rules.svelte";
import { savedMatches, type SavedProject } from "./projects";
import type { ControlPreview, ProcessIdentity, Server, Snapshot, SystemUsage } from "./types";
import { allocateUsage, controlKey, sharesControl } from "./resources";
import { hasPid, matchesSearch, portQuery } from "./search";

/** Must match `SNAPSHOT_EVENT` in `src-tauri/src/ports/mod.rs`. */
export const SNAPSHOT_EVENT = "ports://snapshot";
/** Must match `PROJECTS_EVENT` in `src-tauri/src/ports/saved.rs`. */
export const PROJECTS_EVENT = "ports://projects";

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

/**
 * `forceKill` skips the graceful shutdown; `stopParent` also ends what
 * started the server, such as nodemon; `stopAll` stops every server of the
 * same program.
 */
export type ServerAction = "stop" | "restart" | "forceKill" | "stopParent" | "stopAll";

/** Broad or abrupt actions always show what they end, whatever the preference. */
const alwaysConfirmed: ServerAction[] = ["forceKill", "stopParent", "stopAll"];

/**
 * Whether the action asks first even with confirmation off: broad or abrupt
 * ones, and Homebrew's stop, which also keeps the service from starting at login.
 */
export const mustConfirm = (action: ServerAction, server: Server) =>
	alwaysConfirmed.includes(action) ||
	(action === "stop" && server.supervisor?.kind === "homebrew" && server.supervisor.supported);

/** An action waiting for the user, with what the backend says it would end. */
export interface PendingAction {
	action: ServerAction;
	server: Server;
	/** Every server the action controls: the one chosen, or each server of its program for `stopAll`. */
	servers: Server[];
	/** One per server, in the same order; null while the backend works them out. */
	previews: ControlPreview[] | null;
	error: string | null;
}

interface StopOptions {
	force?: boolean;
	parent?: ProcessIdentity;
	/** The processes of the preview the user confirmed; the backend refuses to reach others. */
	confirmed?: ProcessIdentity[];
}

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

/**
 * A project's ports share one key, so pinning or protecting one covers all of
 * them. Must match `rules::key` in `src-tauri/src/ports/rules.rs`.
 */
export const serverKey = (server: Server) => (server.folder ? `project:${server.folder}` : `port:${server.port}`);

/** Hidden entries of the rules: one port, or every port of a process. */
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

/** The server with the name the user gave it, or the same object when there is none. */
function customize(server: Server): Server {
	const name = preferences.names.current[server.port];
	return name === undefined ? server : { ...server, name };
}

/** `:5000` or `ControlCenter`, for the list of hidden entries. */
export const hiddenLabel = (entry: string) => entry.replace(/^port:/, ":").replace(/^process:/, "");

const serverCount = (n: number) => (n === 1 ? "1 server" : `${n} servers`);

/** Why clean up offers a server. */
export const cleanUpReason = (server: Server) => (server.cwdDeleted ? "folder deleted" : "idle");

/** UI state for the server list, fed by the snapshots the Rust backend publishes. */
export class PortsStore {
	servers = $state.raw<Server[]>([]);
	system = $state.raw<SystemUsage>({ totalBytes: 0, usedBytes: 0, cpuPercent: 0 });
	#revision = -1;
	/** The listener ids of the last snapshot, to refresh saved projects when they change. */
	#listeners: string | null = null;
	query = $state("");
	selectedId = $state<string | null>(null);
	/** Saved project open in the drawer. Separate from a live server: Start is not Restart. */
	selectedProjectId = $state<string | null>(null);
	saved = $state.raw<SavedProject[]>([]);
	/** Action waiting for the user to confirm it. */
	pending = $state.raw<PendingAction | null>(null);
	/** Servers clean up offers, while the user chooses which ones to stop. */
	cleaning = $state.raw<Server[] | null>(null);
	/** The ids of the offered servers the user chose to stop. Belongs to one clean up; never saved. */
	cleanSelection = new SvelteSet<string>();
	/** The home folder, so a search for a full path finds `~/…` folders. */
	#home = $state<string | null>(null);
	/** Server whose name the user is editing. */
	renaming = $state.raw<Server | null>(null);
	/** Work in progress, keyed by action and service identity. */
	#busy = new SvelteSet<string>();
	#controlling = $state.raw<Server[]>([]);

	#customized = $derived(this.servers.map(customize));

	/** Servers of the kinds the user shows, without the ones their rules hide. */
	shown = $derived(
		this.#customized.filter((server) => preferences.shownKinds.current.includes(server.kind) && !server.hidden),
	);

	/** The port a bare number searches for, or null. */
	searchedPort = $derived(portQuery(this.query));

	/**
	 * Every server listening on the searched port, hidden ones included: the
	 * question is who holds the port. Several when processes share it.
	 */
	portOwners = $derived.by(() => {
		const port = this.searchedPort;
		return port === null ? [] : this.#customized.filter((server) => server.port === port);
	});

	/**
	 * The servers matching the search: pinned ones first, then in the chosen
	 * order. A port number lists every owner of the port, and the shown
	 * servers with that PID.
	 */
	filtered = $derived.by(() => {
		const port = this.searchedPort;
		const matching =
			port !== null
				? [...this.portOwners, ...this.shown.filter((server) => server.port !== port && hasPid(server, port))]
				: this.shown.filter((server) => matchesSearch(server, this.query, this.#home));
		const { key, descending } = preferences.sort.current;
		const pinned = preferences.pinned.current;
		const rank = (server: Server) => (pinned.includes(serverKey(server)) ? 0 : 1);
		return matching.toSorted(
			(a, b) => rank(a) - rank(b) || (descending ? -1 : 1) * ascending[key](a, b) || ascending.port(a, b),
		);
	});

	selected = $derived(this.#customized.find((server) => server.id === this.selectedId) ?? null);

	selectedProject = $derived(this.saved.find((project) => project.id === this.selectedProjectId) ?? null);

	/** Saved projects that are not currently the listener on their expected port. */
	savedProjects = $derived(
		this.saved.filter((project) => !project.matchedServerId && savedMatches(project, this.query)),
	);

	projectFor = (server: Server) =>
		this.saved.find((project) => project.matchedServerId === server.id) ??
		this.saved.find((project) => project.folder === server.folder && project.expectedPort === server.port);

	/**
	 * What clean up offers: shown servers Ports can stop that are idle or whose
	 * folder was deleted, unless protected or kept running by a supervisor.
	 * A stop ends the whole process tree, so every server sharing it must qualify.
	 */
	candidates = $derived(
		this.shown.filter(
			(server) =>
				canStop(server) &&
				this.#customized
					.filter((sibling) => sharesControl(server, sibling))
					.every(
						(sibling) =>
							(sibling.status === "idle" || sibling.cwdDeleted) &&
							!sibling.protected &&
							!sibling.supervisor &&
							!sibling.hidden &&
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
		// A run Ports started can exit before it ever listens, which no snapshot shows.
		const unlistenProjects = listen(PROJECTS_EVENT, () => {
			if (connected) void this.refreshSaved();
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
		void homeDir()
			.then((home) => {
				if (connected) this.#home = home;
			})
			.catch(() => {});
		return () => {
			connected = false;
			void unlisten.then((stop) => stop()).catch(() => {});
			void unlistenProjects.then((stop) => stop()).catch(() => {});
		};
	};

	apply = (snapshot: Snapshot) => {
		if (snapshot.revision <= this.#revision) return;
		this.#revision = snapshot.revision;
		this.servers = snapshot.servers;
		this.system = snapshot.system;
		// A saved project's match is computed against the listeners, so it changes with them.
		const listeners = snapshot.servers
			.map((server) => server.id)
			.toSorted()
			.join("\n");
		if (listeners !== this.#listeners) {
			this.#listeners = listeners;
			void this.refreshSaved();
		}
	};

	select = (id: string | null) => {
		this.selectedId = id;
		if (id) this.selectedProjectId = null;
	};

	selectProject = (id: string | null) => {
		this.selectedProjectId = id;
		if (id) this.selectedId = null;
	};

	refreshSaved = async () => {
		if (!isTauri()) return;
		try {
			this.saved = await invoke<SavedProject[]>("list_projects");
		} catch (error) {
			toast.error(String(error));
		}
	};

	saveProject = async (server: Server) => {
		if (!server.folder) return;
		const address = server.addresses.find((item) => item === "127.0.0.1") ?? server.addresses[0] ?? "127.0.0.1";
		const ok = await attempt(
			invoke("save_project", {
				request: {
					folder: server.folder,
					command: server.command,
					expectedPort: server.port,
					expectedAddress: address,
				},
			}),
		);
		await this.refreshSaved();
		if (ok) toast.success(`Saved ${server.name}`, { description: `Starts with ${server.command}` });
	};

	startSaved = async (project: SavedProject) => {
		const ok = await attempt(invoke("start_project", { id: project.id }));
		await this.refreshSaved();
		if (ok) toast.success(`Started ${project.folder.split("/").filter(Boolean).at(-1) ?? "project"}`);
	};

	stopSaved = async (project: SavedProject) => {
		const ok = await attempt(invoke("stop_project", { id: project.id }));
		await this.refreshSaved();
		if (ok) toast.success("Stopped. It will not start again until you ask.");
	};

	forgetSaved = async (project: SavedProject) => {
		const ok = await attempt(invoke("forget_project", { id: project.id }));
		if (this.selectedProjectId === project.id) this.selectedProjectId = null;
		await this.refreshSaved();
		if (ok) toast.success("Removed from saved projects");
	};

	setSavedCommand = async (project: SavedProject, command: string) => {
		const ok = await attempt(invoke("set_project_command", { request: { id: project.id, command } }));
		await this.refreshSaved();
		return ok;
	};

	setSavedBinding = async (project: SavedProject, expectedPort: number, expectedAddress: string) => {
		const ok = await attempt(
			invoke("set_project_binding", { request: { id: project.id, expectedPort, expectedAddress } }),
		);
		await this.refreshSaved();
		return ok;
	};

	setFavorite = async (project: SavedProject, script: string, favorite: boolean) => {
		await attempt(invoke("set_favorite", { request: { id: project.id, script, favorite } }));
		await this.refreshSaved();
	};

	openRemote = (url: string) => attempt(openUrl(url));

	revealSession = (path: string) => attempt(invoke("reveal_session", { path }));

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

	/** Saves a rule change and shows the servers listed under the new rules. */
	changeRules = async (change: RulesChange) => {
		try {
			this.apply(await rules.change(change));
			return true;
		} catch (error) {
			toast.error(String(error));
			return false;
		}
	};

	setProtected = (server: Server, value: boolean) =>
		this.changeRules({ kind: "protect", key: serverKey(server), value });

	hide = async (entry: string) => {
		await this.changeRules({ kind: "hide", entry });
		if (this.selected?.hidden) this.selectedId = null;
	};

	unhide = (entry: string) => this.changeRules({ kind: "unhide", entry });

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

	/** The other servers `stopAll` reaches: each shown, unprotected server of the same program, once per process tree. */
	sameProgram = (server: Server) => {
		const servers: Server[] = [server];
		for (const other of this.shown) {
			if (other.process !== server.process || other.target?.kind !== "process" || other.protected) continue;
			if (!servers.some((chosen) => sharesControl(chosen, other))) servers.push(other);
		}
		return servers;
	};

	/** Entry point for every control button: asks first, with a preview, when the action or the preference says so. */
	request = async (action: ServerAction, server: Server) => {
		if (this.isBusy(server, "control")) return;
		const servers = action === "stopAll" ? this.sameProgram(server) : [server];
		if (!mustConfirm(action, server) && !preferences.confirmActions.current) {
			await this.#execute(action, servers, null);
			return;
		}
		const pending: PendingAction = { action, server, servers, previews: null, error: null };
		this.pending = pending;
		try {
			const previews = await Promise.all(
				servers.map((row) =>
					invoke<ControlPreview>("preview_control", {
						target: target(row),
						parent: action === "stopParent" ? (row.parent?.process ?? null) : null,
					}),
				),
			);
			if (this.pending === pending) this.pending = { ...pending, previews };
		} catch (error) {
			if (this.pending === pending) this.pending = { ...pending, error: String(error) };
		}
	};

	confirm = async () => {
		const pending = this.pending;
		this.pending = null;
		if (pending?.previews) await this.#execute(pending.action, pending.servers, pending.previews);
	};

	async #execute(action: ServerAction, servers: Server[], previews: ControlPreview[] | null) {
		const confirmed = (i: number) => previews?.[i].processes.map((affected) => affected.process);
		if (action === "restart") return this.restart(servers[0], confirmed(0));
		if (action === "stopAll") {
			await Promise.all(servers.map((server, i) => this.stop(server, { confirmed: confirmed(i) })));
			return;
		}
		const [server] = servers;
		await this.stop(server, {
			force: action === "forceKill",
			parent: action === "stopParent" ? server.parent?.process : undefined,
			confirmed: confirmed(0),
		});
	}

	restart = (server: Server, confirmed?: ProcessIdentity[]) =>
		this.#run(server, "control", async () => {
			const id = toast.loading(`Restarting ${server.name}…`, { id: `restart-${server.id}` });
			try {
				this.apply(
					await invoke<Snapshot>("restart_server", {
						target: target(server),
						...(confirmed ? { scope: { confirmed } } : {}),
					}),
				);
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

	stop = (server: Server, options: StopOptions = {}) =>
		this.#run(server, "control", async () => {
			const what = options.parent ? `${server.parent?.name ?? "the parent"} of ${server.name}` : server.name;
			const verb = options.force ? ["Force killing", "Force killed"] : ["Stopping", "Stopped"];
			const id = toast.loading(`${verb[0]} ${what}…`, { id: `stop-${server.id}` });
			const error = await this.#stop(server, null, options);
			if (error) toast.error(error, { id });
			else toast.success(`${verb[1]} ${what}`, { id, description: `:${server.port}` });
		});

	/**
	 * Stops the server and takes it off the list; returns why it couldn't.
	 * For clean up, the backend checks again that every server the stop
	 * ends is one of `allowedIds` and still qualifies.
	 */
	async #stop(server: Server, allowedIds: string[] | null = null, options: StopOptions = {}): Promise<string | null> {
		const { force, parent, confirmed } = options;
		try {
			this.apply(
				await invoke<Snapshot>("stop_server", {
					target: target(server),
					...(allowedIds ? { cleanUp: true, allowedIds } : {}),
					...(force ? { force } : {}),
					...(parent || confirmed ? { scope: { parent, confirmed } } : {}),
				}),
			);
		} catch (error) {
			return String(error);
		}
		if (this.selectedId === server.id && !this.selected) this.selectedId = null;
		return null;
	}

	/** Offers every candidate, all chosen; protected ones are never listed. */
	cleanUp = () => {
		if (!this.candidates.length) return;
		this.cleaning = this.candidates;
		this.cleanSelection.clear();
		for (const server of this.candidates) this.cleanSelection.add(server.id);
	};

	/** Chooses or leaves out a server, with every server sharing its processes: one stop ends them all. */
	chooseForCleanUp = (server: Server, chosen: boolean) => {
		for (const other of this.cleaning ?? []) {
			if (!sharesControl(server, other)) continue;
			if (chosen) this.cleanSelection.add(other.id);
			else this.cleanSelection.delete(other.id);
		}
	};

	/** Stops the chosen servers that still qualify; one that changed since the dialog opened keeps running. */
	confirmCleanUp = async () => {
		const captured = (this.cleaning ?? []).filter((server) => this.cleanSelection.has(server.id));
		this.cleaning = null;
		this.cleanSelection.clear();
		const eligible = captured
			.flatMap((old) => {
				const current = this.candidates.find(
					(server) => server.id === old.id && controlKey(server) === controlKey(old),
				);
				return current ? [current] : [];
			})
			.toSorted((a, b) => b.resources.length - a.resources.length);
		const servers: Server[] = [];
		for (const server of eligible) {
			if (!servers.some((chosen) => sharesControl(chosen, server))) servers.push(server);
		}
		if (!servers.length) return;
		const allowed = eligible.map((server) => server.id);
		// Count rows, like the dialog: one stop can end several ports of a process.
		const rows = (stopped: Server[]) => eligible.filter((row) => stopped.some((s) => sharesControl(s, row))).length;
		const id = toast.loading("Stopping " + serverCount(eligible.length) + "…", { id: "clean-up" });
		const results = await Promise.all(
			servers.map((server) => this.#run(server, "control", () => this.#stop(server, allowed))),
		);
		const attempted = servers.filter((_, i) => results[i] !== undefined);
		const errors = results.filter((result): result is string => typeof result === "string");
		const failed = servers.filter((_, i) => typeof results[i] === "string");
		if (!attempted.length) toast.info("No servers were stopped", { id });
		else if (errors.length)
			toast.error("Couldn't stop " + rows(failed) + " of " + serverCount(rows(attempted)), {
				id,
				description: errors[0],
			});
		else toast.success("Stopped " + serverCount(rows(attempted)), { id });
	};
}
