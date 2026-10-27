import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mockSnapshot } from "./mock";
import { hiddenPort, hiddenProcess, PortsStore, SNAPSHOT_EVENT, unhide } from "./ports-store.svelte";
import { preferences } from "./preferences.svelte";
import type { Server } from "./types";

function server(overrides: Partial<Server>): Server {
	return {
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
		startedAt: 0,
		lastActiveAt: 0,
		memoryBytes: 0,
		cpuPercent: 0,
		memoryHistory: [0],
		cpuHistory: [0],
		...overrides,
	};
}

const web = server({ port: 5173, name: "web", branch: "feat/login", framework: "Vite" });
const api = server({ port: 8080, pid: 200, name: "api", process: "bun", branch: "main" });
const db = server({ port: 5432, pid: 300, name: "postgres", process: "postgres", kind: "database" });
const editor = server({ port: 44438, pid: 400, name: "zed", process: "zed", kind: "other" });

function storeWith(...servers: Server[]) {
	const store = new PortsStore();
	store.servers = servers;
	return store;
}

/** Commands the store sent to the Rust backend, with their arguments. */
let calls: [string, unknown][];

function backend(reply: (cmd: string) => unknown = () => undefined) {
	calls = [];
	// Set by Tauri in its windows; mockIPC only fakes the IPC.
	vi.stubGlobal("isTauri", true);
	mockIPC(
		(cmd, args) => {
			calls.push([cmd, args]);
			return reply(cmd);
		},
		{ shouldMockEvents: true },
	);
}

beforeEach(() => {
	preferences.confirmActions.current = true;
	preferences.shownKinds.current = ["dev", "database", "container"];
	preferences.sort.current = { key: "port", descending: false };
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
	// A server started in a terminal must reach an open window without pressing refresh.
	it("shows each snapshot the backend pushes", async () => {
		backend((cmd) => (cmd === "snapshot" ? { servers: [], system: mockSnapshot().system } : undefined));
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

	// The interval is saved in the webview, so the backend only learns it from the windows.
	it("tells the backend the saved refresh interval when connecting", () => {
		backend((cmd) => (cmd === "snapshot" ? mockSnapshot() : undefined));
		preferences.refreshInterval.current = 5000;
		const disconnect = new PortsStore().connect();
		expect(calls).toContainEqual(["set_refresh_interval", { ms: 5000 }]);
		disconnect();
	});
});

describe("search", () => {
	// The search field promises "port, project, branch".
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
	// Desktop apps like Zed listen on ports too, but they aren't what Ports is for.
	it("hides apps and system services until the user shows them", () => {
		const store = storeWith(web, editor);
		expect(store.filtered).toEqual([web]);
		preferences.shownKinds.current = [...preferences.shownKinds.current, "other"];
		expect(store.filtered).toEqual([web, editor]);
	});

	// The summary must add up to what the list shows.
	it("counts hidden servers as other apps in the usage summary", () => {
		const store = storeWith(web, editor);
		store.system = {
			totalBytes: 16_000,
			kinds: { dev: { memoryBytes: 300, cpuPercent: 2 }, other: { memoryBytes: 2_000, cpuPercent: 5 } },
			otherAppsBytes: 6_000,
			otherAppsCpuPercent: 10,
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
	it("asks for confirmation instead of stopping when the preference is on", async () => {
		const store = storeWith(web, api);
		await store.request("stop", web);
		expect(store.pending).toEqual({ action: "stop", server: web });
		expect(store.servers).toEqual([web, api]);
		expect(calls).toEqual([]);
	});

	it("asks before restarting too", async () => {
		const store = storeWith(web);
		await store.request("restart", web);
		expect(store.pending).toEqual({ action: "restart", server: web });
		expect(calls).toEqual([]);
		await store.confirm();
		expect(calls).toEqual([["restart_server", { target: { port: web.port, pid: web.pid } }]]);
	});

	it("stops the process on the port right away when confirmation is turned off", async () => {
		preferences.confirmActions.current = false;
		const store = storeWith(web, api);
		await store.request("stop", web);
		expect(calls).toEqual([["stop_server", { target: { port: web.port, pid: web.pid } }]]);
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
		expect(calls).toEqual([["stop_server", { target: { port: 5433, container: "4a1b" } }]]);
	});

	// Stopping waits up to seconds for the server to exit; a second click must not stop it twice.
	it("ignores another stop of a server that is still stopping", async () => {
		const stopping = Promise.withResolvers<void>();
		backend((cmd) => (cmd === "stop_server" ? stopping.promise : undefined));
		const store = storeWith(web);
		const first = store.stop(web);
		expect(store.isBusy(web, "control")).toBe(true);
		await store.request("stop", web);
		await store.stop(web);
		stopping.resolve();
		await first;
		expect(calls.filter(([cmd]) => cmd === "stop_server")).toHaveLength(1);
		expect(store.isBusy(web, "control")).toBe(false);
	});

	it("closes the details of the stopped server and clears the pending confirmation", async () => {
		const store = storeWith(web, api);
		store.select(web.port);
		await store.request("stop", web);
		await store.confirm();
		expect(store.selected).toBeNull();
		expect(store.pending).toBeNull();
		expect(store.servers).toEqual([api]);
	});

	// A root-owned process can't be signalled; the row must not pretend it stopped.
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

	// Clicking the Memory header should bring the heaviest server to the top.
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

	// Pinning the main app should pin its API port too.
	it("pins every port of a project together", () => {
		const app = server({ port: 3000, name: "shop", folder: "~/shop" });
		const appApi = server({ port: 9000, pid: 201, name: "shop", folder: "~/shop" });
		const store = storeWith(web, app, appApi);
		store.togglePin(appApi);
		expect(store.filtered).toEqual([app, appApi, web]);
	});

	it("groups servers by project, servers outside a project last", () => {
		const loose = server({ port: 1000, pid: 203, name: "loose" });
		const a1 = server({ port: 2000, name: "a", folder: "~/a" });
		const b1 = server({ port: 3000, pid: 201, name: "b", folder: "~/b" });
		const a2 = server({ port: 4000, pid: 202, name: "a", folder: "~/a" });
		expect(storeWith(loose, a1, b1, a2).groups).toEqual([
			{ folder: "~/a", servers: [a1, a2] },
			{ folder: "~/b", servers: [b1] },
			{ folder: undefined, servers: [loose] },
		]);
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

	// AirPlay's ControlCenter holds 5000 and 7000; hiding the process hides both.
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

	it("stops every idle server after one confirmation, leaving the rest running", async () => {
		const store = storeWith(idleWeb, idleApi, busy, idleDb);
		store.cleanUp();
		expect(store.cleaning).toEqual([idleWeb, idleApi]);
		expect(calls).toEqual([]);
		await store.confirmCleanUp();
		expect(calls).toEqual([
			["stop_server", { target: { port: 5173, pid: 100 } }],
			["stop_server", { target: { port: 8080, pid: 200 } }],
		]);
		expect(store.servers).toEqual([busy, idleDb]);
	});

	it("skips what the user protects and stops a database the user unprotects", () => {
		const store = storeWith(idleWeb, idleApi, idleDb);
		store.setProtected(idleApi, true);
		store.setProtected(idleDb, false);
		expect(store.idle.map((s) => s.port)).toEqual([idleWeb.port, idleDb.port]);
	});

	// Desktop apps idle on ports too; clean up must not quit what the list doesn't show.
	it("leaves hidden servers alone", () => {
		const idleApp = server({ port: 44438, pid: 400, kind: "other", status: "idle" });
		const store = storeWith(idleWeb, idleApp);
		store.hide(hiddenPort(idleWeb.port));
		store.cleanUp();
		expect(store.cleaning).toBeNull();
	});
});
