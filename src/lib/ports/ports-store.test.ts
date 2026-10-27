import { beforeEach, describe, expect, it } from "vitest";
import { PortsStore } from "./ports-store.svelte";
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
		protected: false,
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
const api = server({ port: 8080, name: "api", process: "bun", branch: "main" });
const db = server({ port: 5432, name: "postgres", process: "postgres", kind: "database" });

function storeWith(...servers: Server[]) {
	const store = new PortsStore();
	store.servers = servers;
	return store;
}

beforeEach(() => {
	preferences.confirmStop.current = true;
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

describe("stopping a server", () => {
	it("asks for confirmation instead of stopping when the preference is on", () => {
		const store = storeWith(web, api);
		store.requestStop(web);
		expect(store.pendingStop).toBe(web);
		expect(store.servers).toEqual([web, api]);
	});

	it("stops right away when confirmation is turned off", () => {
		preferences.confirmStop.current = false;
		const store = storeWith(web, api);
		store.requestStop(web);
		expect(store.pendingStop).toBeNull();
		expect(store.servers).toEqual([api]);
	});

	it("closes the details of the stopped server and clears the pending confirmation", () => {
		const store = storeWith(web, api);
		store.select(web.port);
		store.requestStop(web);
		store.stop(web);
		expect(store.selected).toBeNull();
		expect(store.pendingStop).toBeNull();
		expect(store.servers).toEqual([api]);
	});
});
