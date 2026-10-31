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

export const SNAPSHOT_EVENT = "ports://snapshot";
export const PROJECTS_EVENT = "ports://projects";

async function attempt(action: Promise<unknown>): Promise<boolean> {
	try {
		await action;
		return true;
	} catch (error) {
		toast.error(String(error));
		return false;
	}
}

export async function setRefreshInterval(ms: number) {
	const updated = await attempt(invoke("set_refresh_interval", { ms }));
	if (updated) preferences.refreshInterval.current = ms;
	return updated;
}

export type ServerAction = "stop" | "restart" | "forceKill" | "stopParent" | "stopAll";

const alwaysConfirmed: ServerAction[] = ["forceKill", "stopParent", "stopAll"];

/**
 * Whether the action asks first even with confirmation off: broad or abrupt
 * ones, and Homebrew's stop, which also keeps the service from starting at login.
 */
export const mustConfirm = (action: ServerAction, server: Server) =>
	alwaysConfirmed.includes(action) ||
	(action === "stop" && server.supervisor?.kind === "homebrew" && server.supervisor.controllable);

export interface PendingAction {
	action: ServerAction;
	server: Server;
	servers: Server[];
	previews: ControlPreview[] | null;
	error: string | null;
}

interface StopOptions {
	force?: boolean;
	parent?: ProcessIdentity;
	confirmed?: ProcessIdentity[];
}

type Busy = "control" | "editor" | "terminal" | "folder";

const target = (server: Server) => {
	if (!server.target) throw new Error("Ports cannot control this server");
	return server.target;
};

export const serverKey = (server: Server) =>
	server.folder && server.kind !== "container" ? `project:${server.folder}` : `port:${server.port}`;

export const hiddenPort = (port: number) => `port:${port}`;
export const hiddenProcess = (process: string) => `process:${process}`;

const ascending: Record<SortKey, (a: Server, b: Server) => number> = {
	port: (a, b) => a.port - b.port,
	name: (a, b) => a.name.localeCompare(b.name),
	memory: (a, b) => a.memoryBytes - b.memoryBytes,
	cpu: (a, b) => a.cpuPercent - b.cpuPercent,
	uptime: (a, b) => b.startedAt - a.startedAt,
};

function customize(server: Server): Server {
	const name = preferences.names.current[server.port];
	return name === undefined ? server : { ...server, name };
}

export const hiddenLabel = (entry: string) => entry.replace(/^port:/, ":").replace(/^process:/, "");

const serverCount = (n: number) => (n === 1 ? "1 server" : `${n} servers`);

export const cleanUpReason = (server: Server) => (server.cwdDeleted ? "folder deleted" : "idle");

export class PortsStore {
	servers = $state.raw<Server[]>([]);
	system = $state.raw<SystemUsage>({ totalBytes: 0, usedBytes: 0, cpuPercent: 0 });
	#revision = -1;
	#listeners: string | null = null;
	query = $state("");
	selectedId = $state<string | null>(null);
	selectedProjectId = $state<string | null>(null);
	saved = $state.raw<SavedProject[]>([]);
	pending = $state.raw<PendingAction | null>(null);
	cleaning = $state.raw<Server[] | null>(null);
	cleanSelection = new SvelteSet<string>();
	#home = $state<string | null>(null);
	renaming = $state.raw<Server | null>(null);
	#busy = new SvelteSet<string>();
	#controlling = $state.raw<Server[]>([]);

	#customized = $derived(this.servers.map(customize));

	shown = $derived(
		this.#customized.filter((server) => preferences.shownKinds.current.includes(server.kind) && !server.hidden),
	);

	searchedPort = $derived(portQuery(this.query));

	portOwners = $derived.by(() => {
		const port = this.searchedPort;
		return port === null ? [] : this.#customized.filter((server) => server.port === port);
	});

	filtered = $derived.by(() => {
		const port = this.searchedPort;
		const exact =
			port === null
				? []
				: [...this.portOwners, ...this.shown.filter((server) => server.port !== port && hasPid(server, port))];
		const matching = exact.length
			? exact
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

	savedProjects = $derived(
		this.saved.filter((project) => !project.matchedServerId && savedMatches(project, this.query)),
	);

	projectFor = (server: Server) =>
		this.saved.find((project) => project.matchedServerId === server.id) ??
		this.saved.find((project) => project.folder === server.folder && project.expectedPort === server.port);

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

	connect = () => {
		if (!isTauri()) return () => {};
		let connected = true;
		const unlisten = listen<Snapshot>(SNAPSHOT_EVENT, (event) => {
			if (connected) this.apply(event.payload);
		});
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
		if (ok) toast.success("Command saved");
		return ok;
	};

	setSavedBinding = async (project: SavedProject, expectedPort: number, expectedAddress: string) => {
		const ok = await attempt(
			invoke("set_project_binding", { request: { id: project.id, expectedPort, expectedAddress } }),
		);
		await this.refreshSaved();
		if (ok) toast.success("Port saved");
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
			toast.success("Refreshed", { id: "refresh" });
		} catch (error) {
			toast.error(String(error));
		}
	};

	isBusy = (server: Server, work: Busy) =>
		work === "control"
			? this.#controlling.some((busy) => sharesControl(busy, server))
			: this.#busy.has(`${work}:${server.id}`);

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

	detectedName = (port: number) => this.servers.find((server) => server.port === port)?.name ?? "";

	rename = (port: number, name: string) => {
		const { [port]: previous, ...others } = preferences.names.current;
		const trimmed = name.trim();
		const custom = trimmed && trimmed !== this.detectedName(port) ? trimmed : undefined;
		if (custom === previous) return;
		preferences.names.current = custom ? { ...others, [port]: custom } : others;
		toast.success(custom ? `Renamed to ${custom}` : "Name reset");
	};

	changeRules = async (change: RulesChange, confirmation?: string) => {
		try {
			this.apply(await rules.change(change));
			if (confirmation) toast.success(confirmation);
			return true;
		} catch (error) {
			toast.error(String(error));
			return false;
		}
	};

	setProtected = (server: Server, value: boolean) =>
		this.changeRules({ kind: "protect", key: serverKey(server), value });

	hide = async (entry: string) => {
		await this.changeRules({ kind: "hide", entry }, `Hidden ${hiddenLabel(entry)}`);
		if (this.selected?.hidden) this.selectedId = null;
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

	sameProgram = (server: Server) => {
		const servers: Server[] = [server];
		for (const other of this.shown) {
			if (other.process !== server.process || other.target?.kind !== "process" || other.protected) continue;
			if (!servers.some((chosen) => sharesControl(chosen, other))) servers.push(other);
		}
		return servers;
	};

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
				this.#keepRestartedSelected(server);
				toast.success(`Restarted ${server.name}`, { id, description: `:${server.port}` });
			} catch (error) {
				toast.error(String(error), { id });
			}
		});

	#keepRestartedSelected(server: Server) {
		if (this.selectedId !== server.id || this.selected) return;
		this.selectedId =
			this.#customized.find(
				(row) => row.port === server.port && row.process === server.process && row.folder === server.folder,
			)?.id ?? null;
	}

	stop = (server: Server, options: StopOptions = {}) =>
		this.#run(server, "control", async () => {
			const what = options.parent ? `${server.parent?.name ?? "the parent"} of ${server.name}` : server.name;
			const verb = options.force ? ["Force killing", "Force killed"] : ["Stopping", "Stopped"];
			const id = toast.loading(`${verb[0]} ${what}…`, { id: `stop-${server.id}` });
			const error = await this.#stop(server, null, options);
			if (error) toast.error(error, { id });
			else toast.success(`${verb[1]} ${what}`, { id, description: `:${server.port}` });
		});

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

	cleanUp = () => {
		if (!this.candidates.length) return;
		this.cleaning = this.candidates;
		this.cleanSelection.clear();
		for (const server of this.candidates) this.cleanSelection.add(server.id);
	};

	chooseForCleanUp = (server: Server, chosen: boolean) => {
		for (const other of this.cleaning ?? []) {
			if (!sharesControl(server, other)) continue;
			if (chosen) this.cleanSelection.add(other.id);
			else this.cleanSelection.delete(other.id);
		}
	};

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
