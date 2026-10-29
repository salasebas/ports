import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { listRows } from "./grouping";
import { mockSnapshot } from "./mock";
import { hiddenPort, hiddenProcess, PortsStore, SNAPSHOT_EVENT, unhide } from "./ports-store.svelte";
import { preferences } from "./preferences.svelte";
import type { SavedProject } from "./projects";
import { processTree } from "./resources";
import type { ControlPreview, Server, Snapshot, ServerTarget } from "./types";

function server(overrides: Partial<Server>): Server {
	const value: Omit<Server, "id" | "target" | "resources"> = {
		port: 3000,
		pid: 100,
		process: "node",
		command: "vite dev",
		name: "web",
		kind: "dev",
		status: "running",
		responding: true,
		addresses: ["127.0.0.1"],
		exposed: false,
		protected: false,
		restartable: true,
		attribution: { confidence: "unknown", evidence: [] },
		web: { state: "unknown" },
		startedAt: 0,
		lastActiveAt: 0,
		memoryBytes: 0,
		cpuPercent: 0,
		memoryHistory: [0],
		cpuHistory: [0],
		...overrides,
	};
	const process = { pid: value.pid!, start: String(value.startedAt) };
	const target: ServerTarget | undefined = value.container
		? {
				kind: "container",
				port: value.port,
				id: value.container.id,
				engine: "/docker.sock",
				addresses: value.addresses,
			}
		: value.pid
			? { kind: "process", port: value.port, listener: process, root: process, addresses: value.addresses }
			: undefined;
	return {
		...value,
		id: `${value.port}:${value.pid ?? value.container?.id}:${value.startedAt}`,
		target,
		resources: value.pid
			? [
					{
						process,
						name: value.process,
						ports: [value.port],
						memoryBytes: value.memoryBytes,
						cpuPercent: value.cpuPercent,
					},
				]
			: [],
	};
}

const web = server({ port: 5173, name: "web", branch: "feat/login", framework: "Vite" });
const api = server({ port: 8080, pid: 200, name: "api", process: "bun", branch: "main" });
const db = server({ port: 5432, pid: 300, name: "postgres", process: "postgres", kind: "database" });
const editor = server({ port: 44438, pid: 400, name: "zed", process: "zed", kind: "other" });

let activeStore: PortsStore;
let backendRows: Server[];
let revision = 0;
function snapshot(servers: Server[]): Snapshot {
	return { ...mockSnapshot(), revision: ++revision, servers };
}

function storeWith(...servers: Server[]) {
	const store = new PortsStore();
	store.servers = servers;
	activeStore = store;
	backendRows = servers;
	return store;
}

function previewOf({ target }: { target: ServerTarget }): ControlPreview {
	const rows = backendRows.filter((row) => JSON.stringify(row.target) === JSON.stringify(target));
	return {
		processes: rows.flatMap((row) => row.resources.map(({ process, name, ports }) => ({ process, name, ports }))),
	};
}

let calls: [string, unknown][];

function backend(reply?: (cmd: string) => unknown) {
	calls = [];
	// Set by Tauri in its windows; mockIPC only fakes the IPC.
	vi.stubGlobal("isTauri", true);
	mockIPC(
		(cmd, args) => {
			if (cmd !== "list_projects") calls.push([cmd, args]);
			if (reply) return reply(cmd);
			if (cmd === "preview_control") return previewOf(args as { target: ServerTarget });
			if (cmd === "restart_server") return snapshot(activeStore.servers);
			if (cmd === "stop_server") {
				const { target } = args as { target: ServerTarget };
				backendRows = backendRows.filter((row) => {
					const other = row.target;
					if (!other || target.kind !== other.kind) return true;
					return target.kind === "process" && other.kind === "process"
						? target.root.pid !== other.root.pid || target.root.start !== other.root.start
						: target.kind === "container" &&
								other.kind === "container" &&
								(target.id !== other.id || target.engine !== other.engine);
				});
				return snapshot(backendRows);
			}
			return undefined;
		},
		{ shouldMockEvents: true },
	);
}

beforeEach(() => {
	revision = 0;
	preferences.confirmActions.current = true;
	preferences.shownKinds.current = ["dev", "database", "container"];
	preferences.sort.current = { key: "port", descending: false };
	preferences.groupBy.current = "flat";
	preferences.collapsedGroups.current = [];
	preferences.pinned.current = [];
	preferences.names.current = {};
	preferences.hidden.current = [];
	preferences.protection.current = {};
	backend();
});

afterEach(() => {
	clearMocks();
	vi.unstubAllGlobals();
});

describe("live updates", () => {
	it("keeps a newer snapshot when an older refresh reply finishes later", async () => {
		const reply = Promise.withResolvers<ReturnType<typeof mockSnapshot>>();
		backend((cmd) => (cmd === "refresh" ? reply.promise : undefined));
		const store = new PortsStore();
		const old = { ...mockSnapshot(), revision: 1, servers: [web] };
		const newer = { ...old, revision: 2, servers: [api] };
		const refreshing = store.refresh();
		store.apply(newer);
		reply.resolve(old);
		await refreshing;
		expect(store.servers).toEqual([api]);
	});
	it("shows each snapshot the backend pushes", async () => {
		backend((cmd) => (cmd === "snapshot" ? { revision: 0, servers: [], system: mockSnapshot().system } : undefined));
		const store = new PortsStore();
		const disconnect = store.connect();
		const pushed = mockSnapshot();

		await vi.waitFor(async () => {
			await emit(SNAPSHOT_EVENT, pushed);
			expect(store.servers).toEqual(pushed.servers);
		});
		const pushedBytes = pushed.servers.reduce((sum, s) => sum + s.memoryBytes, 0);
		expect(store.usage.serversBytes).toBe(pushedBytes);
		disconnect();
	});

	it("shows a saved project again when its server stops", async () => {
		const project: SavedProject = {
			id: "p1",
			folder: "~/shop",
			command: "vite dev",
			favorites: [],
			expectedPort: 3000,
			expectedAddress: "127.0.0.1",
			recovery: "manual",
			manager: "npm",
			scripts: [],
			folderMissing: false,
			running: false,
		};
		let listening = true;
		backend((cmd) => {
			if (cmd === "snapshot") return { revision: 0, servers: [], system: mockSnapshot().system };
			if (cmd === "list_projects") return [listening ? { ...project, matchedServerId: web.id } : project];
			return undefined;
		});
		const store = new PortsStore();
		const disconnect = store.connect();
		await vi.waitFor(async () => {
			await emit(SNAPSHOT_EVENT, snapshot([web]));
			expect(store.saved[0]?.matchedServerId).toBe(web.id);
		});
		expect(store.savedProjects).toEqual([]);

		listening = false;
		await emit(SNAPSHOT_EVENT, snapshot([]));
		await vi.waitFor(() => expect(store.savedProjects.map((item) => item.id)).toEqual(["p1"]));
		disconnect();
	});

	it("tells the backend the saved refresh interval when connecting", () => {
		backend((cmd) => (cmd === "snapshot" ? mockSnapshot() : undefined));
		preferences.refreshInterval.current = 5000;
		const disconnect = new PortsStore().connect();
		expect(calls).toContainEqual(["set_refresh_interval", { ms: 5000 }]);
		disconnect();
	});
});

describe("search", () => {
	it.each([
		["port", "8080", [api]],
		["project", "POST", [db]],
		["branch", "feat/", [web]],
	])("matches by %s", (_field, query, expected) => {
		const store = storeWith(web, api, db);
		store.query = query;
		expect(store.filtered).toEqual(expected);
	});

	it("lists every server for a blank query", () => {
		const store = storeWith(web, api);
		store.query = "   ";
		expect(store.filtered).toEqual([web, api]);
	});
});

describe("kinds of server", () => {
	it("hides apps and system services until the user shows them", () => {
		const store = storeWith(web, editor);
		expect(store.filtered).toEqual([web]);
		preferences.shownKinds.current = [...preferences.shownKinds.current, "other"];
		expect(store.filtered).toEqual([web, editor]);
	});

	it("counts hidden servers as other apps in the usage summary", () => {
		const store = storeWith(server({ ...web, memoryBytes: 300, cpuPercent: 2 }), editor);
		store.system = {
			totalBytes: 16_000,
			usedBytes: 8_300,
			cpuPercent: 17,
		};
		expect(store.usage).toEqual({
			serversBytes: 300,
			serversCpuPercent: 2,
			otherAppsBytes: 8_000,
			otherAppsCpuPercent: 15,
		});
	});
});

describe("stopping a server", () => {
	it("keeps a replacement published before the stopped process replies", async () => {
		const reply = Promise.withResolvers<Snapshot>();
		backend((cmd) => (cmd === "stop_server" ? reply.promise : undefined));
		const store = storeWith(web);
		const stopping = store.stop(web);
		const replacement = server({ ...web, pid: 999, name: "replacement" });
		store.apply({ ...snapshot([replacement]), revision: 2 });
		reply.resolve({ ...snapshot([]), revision: 1 });
		await stopping;
		expect(store.servers).toEqual([replacement]);
	});
	it("asks for confirmation instead of stopping when the preference is on", async () => {
		const store = storeWith(web, api);
		await store.request("stop", web);
		expect(store.pending).toEqual({
			action: "stop",
			server: web,
			servers: [web],
			previews: [previewOf({ target: web.target! })],
			error: null,
		});
		expect(store.servers).toEqual([web, api]);
		expect(calls.map(([cmd]) => cmd)).toEqual(["preview_control"]);
	});

	it("asks before restarting too, and restarts only what the preview showed", async () => {
		const store = storeWith(web);
		await store.request("restart", web);
		expect(store.pending?.action).toBe("restart");
		expect(calls.map(([cmd]) => cmd)).toEqual(["preview_control"]);
		await store.confirm();
		expect(calls.at(-1)).toEqual([
			"restart_server",
			{ target: web.target, scope: { confirmed: [web.resources[0].process] } },
		]);
	});

	it("can't confirm an action whose preview failed", async () => {
		backend((cmd) => {
			if (cmd === "preview_control") throw "The server on :5173 changed; refresh before trying again";
		});
		const store = storeWith(web);
		await store.request("stop", web);
		expect(store.pending?.error).toContain("changed");
		await store.confirm();
		expect(calls.map(([cmd]) => cmd)).toEqual(["preview_control"]);
	});

	it("asks before a Force Kill even with confirmation off, then kills only the previewed scope", async () => {
		preferences.confirmActions.current = false;
		const store = storeWith(web);
		await store.request("forceKill", web);
		expect(store.pending?.previews).toHaveLength(1);
		await store.confirm();
		expect(calls.at(-1)).toEqual([
			"stop_server",
			{ target: web.target, onlyIfIdle: false, force: true, scope: { confirmed: [web.resources[0].process] } },
		]);
	});

	it("asks before stopping a Homebrew service even with confirmation off", async () => {
		preferences.confirmActions.current = false;
		const redis = server({ ...db, supervisor: { kind: "homebrew", name: "redis", controllable: true } });
		const store = storeWith(redis);
		await store.request("stop", redis);
		expect(store.pending?.action).toBe("stop");
		expect(calls.map(([cmd]) => cmd)).toEqual(["preview_control"]);
	});

	it("previews and stops the parent of a server", async () => {
		const nodemon = { process: { pid: 90, start: "1" }, name: "nodemon" };
		const watched = server({ ...web, parent: nodemon });
		const parentScope = { processes: [{ process: nodemon.process, name: "nodemon", ports: [] }] };
		backend((cmd) => (cmd === "preview_control" ? parentScope : snapshot([])));
		const store = storeWith(watched);
		await store.request("stopParent", watched);
		expect(calls[0]).toEqual(["preview_control", { target: watched.target, parent: nodemon.process }]);
		await store.confirm();
		expect(calls.at(-1)).toEqual([
			"stop_server",
			{
				target: watched.target,
				onlyIfIdle: false,
				scope: { parent: nodemon.process, confirmed: [nodemon.process] },
			},
		]);
	});

	it("stops every unprotected server of the same program once per process tree", async () => {
		const second = server({ port: 3001, pid: 101, name: "docs" });
		const secondPort = server({ ...second, port: 3002 });
		const guarded = server({ port: 3003, pid: 102, name: "admin", protected: true });
		const store = storeWith(web, second, secondPort, guarded, api);
		await store.request("stopAll", web);
		expect(store.pending?.servers).toEqual([web, second]);
		await store.confirm();
		const stopped = calls
			.filter(([cmd]) => cmd === "stop_server")
			.map(([, args]) => (args as { target: unknown }).target);
		expect(stopped).toEqual([web.target, second.target]);
		expect(store.servers).toEqual([guarded, api]);
	});

	it("stops the process on the port right away when confirmation is turned off", async () => {
		preferences.confirmActions.current = false;
		const store = storeWith(web, api);
		await store.request("stop", web);
		expect(calls).toEqual([["stop_server", { target: web.target, onlyIfIdle: false }]]);
		expect(store.pending).toBeNull();
		expect(store.servers).toEqual([api]);
	});

	// Docker owns the port forwarder; stopping its process would take every container down.
	it("stops a container by its id", async () => {
		const container = server({
			port: 5433,
			pid: undefined,
			kind: "container",
			container: { id: "4a1b", image: "postgres" },
		});
		await storeWith(container).stop(container);
		expect(calls).toEqual([["stop_server", { target: container.target, onlyIfIdle: false }]]);
	});

	it("ignores another stop of a server that is still stopping", async () => {
		const stopping = Promise.withResolvers<Snapshot>();
		backend((cmd) => (cmd === "stop_server" ? stopping.promise : undefined));
		const store = storeWith(web);
		const first = store.stop(web);
		expect(store.isBusy(web, "control")).toBe(true);
		await store.request("stop", web);
		await store.stop(web);
		stopping.resolve(snapshot([]));
		await first;
		expect(calls.filter(([cmd]) => cmd === "stop_server")).toHaveLength(1);
		expect(store.isBusy(web, "control")).toBe(false);
	});

	it("keeps the details of a restarted server open", async () => {
		const restarted = server({ ...web, pid: 101, startedAt: 5 });
		backend((cmd) => (cmd === "restart_server" ? snapshot([restarted, api]) : undefined));
		const store = storeWith(web, api);
		store.select(web.id);
		await store.restart(web);
		expect(store.selected?.id).toBe(restarted.id);
	});

	it("closes the details of the stopped server and clears the pending confirmation", async () => {
		const store = storeWith(web, api);
		store.select(web.id);
		await store.request("stop", web);
		await store.confirm();
		expect(store.selected).toBeNull();
		expect(store.pending).toBeNull();
		expect(store.servers).toEqual([api]);
	});

	it("keeps the server listed when the backend can't stop it", async () => {
		backend((cmd) => {
			if (cmd === "stop_server") throw "Can't stop postgres (PID 300): it belongs to root.";
		});
		const store = storeWith(web, db);
		await store.stop(db);
		expect(store.servers).toEqual([web, db]);
	});
});

describe("order", () => {
	const light = server({ port: 3000, name: "light", memoryBytes: 100, cpuPercent: 9, startedAt: 30 });
	const heavy = server({ port: 4000, pid: 201, name: "heavy", memoryBytes: 900, cpuPercent: 1, startedAt: 10 });
	const medium = server({ port: 5000, pid: 202, name: "medium", memoryBytes: 500, cpuPercent: 5, startedAt: 20 });

	it.each([
		["memory", true, [heavy, medium, light]],
		["cpu", true, [light, medium, heavy]],
		["uptime", true, [heavy, medium, light]],
		["name", false, [heavy, light, medium]],
		["port", true, [medium, heavy, light]],
	] as const)("sorts by %s", (key, descending, expected) => {
		preferences.sort.current = { key, descending };
		expect(storeWith(light, heavy, medium).filtered).toEqual(expected);
	});

	it("keeps pinned servers on top whatever the order", () => {
		preferences.sort.current = { key: "memory", descending: true };
		const store = storeWith(light, heavy, medium);
		store.togglePin(light);
		expect(store.filtered).toEqual([light, heavy, medium]);
		store.togglePin(light);
		expect(store.filtered).toEqual([heavy, medium, light]);
	});

	it("pins every port of a project together", () => {
		const app = server({ port: 3000, name: "shop", folder: "~/shop" });
		const appApi = server({ port: 9000, pid: 201, name: "shop", folder: "~/shop" });
		const store = storeWith(web, app, appApi);
		store.togglePin(appApi);
		expect(store.filtered).toEqual([app, appApi, web]);
	});

	it("keeps a pin and a protection when the list is grouped by repository", () => {
		const main = server({
			port: 3000,
			name: "web",
			folder: "~/repo",
			repositoryId: "~/repo/.git",
			worktreePath: "~/repo",
			memoryBytes: 10,
		});
		const hotfix = server({
			port: 3001,
			pid: 201,
			name: "web",
			folder: "~/hotfix",
			repositoryId: "~/repo/.git",
			worktreePath: "~/hotfix",
			worktree: "hotfix",
			memoryBytes: 4,
		});
		const store = storeWith(main, hotfix);
		store.togglePin(hotfix);
		store.setProtected(main, true);
		const rows = listRows(store.filtered, "repository", []);
		const listed = rows.flatMap((row) => (row.kind === "server" ? [row.server] : []));
		expect(listed.map((item) => item.port)).toEqual([3001, 3000]);
		expect(listed.find((item) => item.port === 3000)?.protected).toBe(true);
		expect(rows.find((row) => row.kind === "repository")).toMatchObject({ count: 2, memoryBytes: 14 });
	});
});

describe("renaming and hiding", () => {
	it("shows and finds a port by the name the user gave it", () => {
		const store = storeWith(web, api);
		store.rename(web.port, "  Storefront ");
		expect(store.filtered[0].name).toBe("Storefront");
		store.query = "storefront";
		expect(store.filtered.map((s) => s.port)).toEqual([web.port]);
		store.rename(web.port, "");
		expect(store.filtered).toEqual([]);
		expect(preferences.names.current).toEqual({});
	});

	it("hides a port or every port of a process until unhidden", () => {
		const airplay = server({ port: 5000, pid: 500, process: "ControlCenter", kind: "dev" });
		const airplay2 = server({ port: 7000, pid: 500, process: "ControlCenter", kind: "dev" });
		const store = storeWith(web, airplay, airplay2);
		store.hide(hiddenProcess("ControlCenter"));
		expect(store.filtered).toEqual([web]);
		store.hide(hiddenPort(web.port));
		expect(store.filtered).toEqual([]);
		unhide(hiddenProcess("ControlCenter"));
		expect(store.filtered).toEqual([airplay, airplay2]);
	});
});

describe("clean up", () => {
	const idleWeb = server({ port: 5173, name: "web", status: "idle" });
	const idleApi = server({ port: 8080, pid: 200, name: "api", status: "idle", folder: "~/api" });
	const busy = server({ port: 3000, pid: 201, name: "busy" });
	const idleDb = server({ port: 5432, pid: 300, kind: "database", status: "idle", protected: true });

	it.each(["protected", "active", "hidden"])("rechecks a %s server before confirming cleanup", async (change) => {
		const store = storeWith(idleWeb);
		store.cleanUp();
		if (change === "protected") store.setProtected(idleWeb, true);
		if (change === "active") store.servers = [{ ...idleWeb, status: "running" }];
		if (change === "hidden") store.hide(hiddenPort(idleWeb.port));
		await store.confirmCleanUp();
		expect(calls.filter(([cmd]) => cmd === "stop_server")).toEqual([]);
	});

	it("protects all ports affected by the same process", () => {
		const sibling = server({ ...idleWeb, port: 5174 });
		const store = storeWith(idleWeb, sibling);
		store.setProtected(sibling, true);
		expect(store.idle).toEqual([]);
	});

	it("stops a process with multiple ports once", async () => {
		const sibling = server({ ...idleWeb, port: 5174 });
		const store = storeWith(idleWeb, sibling);
		store.cleanUp();
		await store.confirmCleanUp();
		expect(calls.filter(([cmd]) => cmd === "stop_server")).toHaveLength(1);
	});

	it("leaves servers a supervisor runs out of clean up", () => {
		const brewed = server({ ...idleApi, supervisor: { kind: "homebrew", name: "redis", controllable: true } });
		expect(storeWith(idleWeb, brewed).idle).toEqual([idleWeb]);
	});

	it("stops every idle server after one confirmation, leaving the rest running", async () => {
		const store = storeWith(idleWeb, idleApi, busy, idleDb);
		store.cleanUp();
		expect(store.cleaning).toEqual([idleWeb, idleApi]);
		expect(calls).toEqual([]);
		await store.confirmCleanUp();
		expect(calls).toEqual([
			["stop_server", { target: idleWeb.target, onlyIfIdle: true, allowedIds: [idleWeb.id, idleApi.id] }],
			["stop_server", { target: idleApi.target, onlyIfIdle: true, allowedIds: [idleWeb.id, idleApi.id] }],
		]);
		expect(store.servers).toEqual([busy, idleDb]);
	});

	it("skips what the user protects and stops a database the user unprotects", () => {
		const store = storeWith(idleWeb, idleApi, idleDb);
		store.setProtected(idleApi, true);
		store.setProtected(idleDb, false);
		expect(store.idle.map((s) => s.port)).toEqual([idleWeb.port, idleDb.port]);
	});

	it("leaves hidden servers alone", () => {
		const idleApp = server({ port: 44438, pid: 400, kind: "other", status: "idle" });
		const store = storeWith(idleWeb, idleApp);
		store.hide(hiddenPort(idleWeb.port));
		store.cleanUp();
		expect(store.cleaning).toBeNull();
	});
});

const workerUsage = (pid: number, parent: number, memoryBytes: number, ports: number[] = []) => ({
	process: { pid, start: "1" },
	parent,
	name: "gunicorn",
	ports,
	memoryBytes,
	cpuPercent: 1,
});

describe("resource allocation and row identity", () => {
	it("counts shared descendants once across headline totals and chart slices", () => {
		const child = server({ pid: 50, port: 3001, memoryBytes: 100, cpuPercent: 1 });
		const parent = server({ pid: 51, port: 3002, memoryBytes: 200, cpuPercent: 2 });
		parent.resources.push(...child.resources);
		const store = storeWith(child, parent);
		expect(store.usage.serversBytes).toBe(300);
		expect(store.allocations.reduce((sum, row) => sum + row.usage.memoryBytes, 0)).toBe(300);
		store.hide(hiddenPort(child.port));
		expect(store.usage.serversBytes).toBe(300);
		store.hide(hiddenPort(parent.port));
		expect(store.usage.serversBytes).toBe(0);
	});

	it("lists a server's processes as a tree that adds up to its total", () => {
		const master = server({ port: 8000, pid: 10, process: "gunicorn", memoryBytes: 600 });
		master.resources = [workerUsage(12, 10, 200, [8000]), workerUsage(10, 1, 300), workerUsage(11, 10, 100, [8000])];
		const rows = processTree(master);
		expect(rows.map(({ usage, depth }) => [usage.process.pid, depth, usage.ports])).toEqual([
			[10, 0, []],
			[11, 1, [8000]],
			[12, 1, [8000]],
		]);
		expect(rows.reduce((sum, row) => sum + row.usage.memoryBytes, 0)).toBe(master.memoryBytes);
	});

	it("selects independent owners on the same numeric port", () => {
		const second = server({ ...web, pid: 501, addresses: ["::1"] });
		const store = storeWith(web, second);
		store.select(second.id);
		expect(store.selected).toBe(second);
	});
});

it("blocks controls on an overlapping process tree with a different root", async () => {
	const parent = server({ pid: 600, port: 3000 });
	const child = server({ pid: 601, port: 3001 });
	parent.resources.push(...child.resources);
	const stopped = Promise.withResolvers<Snapshot>();
	backend((command) => (command === "stop_server" ? stopped.promise : snapshot([])));
	const store = storeWith(parent, child);
	const stopping = store.stop(parent);
	expect(store.isBusy(child, "control")).toBe(true);
	await store.restart(child);
	expect(calls.map(([command]) => command)).toEqual(["stop_server"]);
	stopped.resolve(snapshot([]));
	await stopping;
});

it("moves an individually hidden server's resources into Other apps", () => {
	const row = server({ ...web, memoryBytes: 300, cpuPercent: 2 });
	const store = storeWith(row);
	store.system = { totalBytes: 1600, usedBytes: 1000, cpuPercent: 10 };
	store.hide(hiddenPort(row.port));
	expect(store.usage).toEqual({ serversBytes: 0, serversCpuPercent: 0, otherAppsBytes: 1000, otherAppsCpuPercent: 10 });
});
