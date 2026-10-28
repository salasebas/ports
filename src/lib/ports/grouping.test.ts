import { describe, expect, it } from "vitest";
import { groupByFromStorage, listRows, toggleCollapsed, type GroupRow, type ListRow } from "./grouping";
import type { Server } from "./types";

function server(overrides: Partial<Server> & Pick<Server, "port" | "name">): Server {
	return {
		process: "node",
		command: "vite",
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
		resources: [],
		...overrides,
		id: `${overrides.port}`,
		pid: overrides.pid ?? overrides.port,
	};
}

const named = (rows: ListRow[], kind: GroupRow["kind"]) => rows.filter((row): row is GroupRow => row.kind === kind);

const web = server({
	port: 3000,
	name: "web",
	folder: "~/repo/apps/web",
	repositoryId: "~/repo/.git",
	repositoryName: "repo",
	repository: "~/repo",
	worktreePath: "~/repo",
	branch: "main",
	memoryBytes: 40,
	cpuPercent: 1,
});
const api = server({
	port: 4000,
	name: "api",
	folder: "~/repo/apps/api",
	repositoryId: "~/repo/.git",
	repositoryName: "repo",
	repository: "~/repo",
	worktreePath: "~/repo",
	branch: "main",
	memoryBytes: 60,
	cpuPercent: 2,
});
const hotfix = server({
	port: 5000,
	name: "web",
	folder: "~/hotfix/apps/web",
	repositoryId: "~/repo/.git",
	repositoryName: "repo",
	repository: "~/repo",
	worktree: "hotfix",
	worktreePath: "~/hotfix",
	branch: "hotfix",
	memoryBytes: 10,
	cpuPercent: 3,
});
const nested = server({
	port: 6000,
	name: "pkg",
	folder: "~/repo/pkg",
	repositoryId: "~/repo/.git/modules/pkg",
	repositoryName: "pkg",
	repository: "~/repo/pkg",
	worktreePath: "~/repo/pkg",
	memoryBytes: 5,
	cpuPercent: 1,
});

describe("repository grouping", () => {
	it("keeps two worktrees of one repository apart and a monorepo's services together", () => {
		const rows = listRows([web, hotfix, api], "repository", []);
		const repo = rows.find((row) => row.kind === "repository");
		expect(repo).toMatchObject({ name: "repo", count: 3, memoryBytes: 110, cpuPercent: 6 });
		const trees = named(rows, "worktree");
		expect(trees.map((row) => row.name)).toEqual(["repo", "hotfix"]);
		expect(trees[0]).toMatchObject({ count: 2, memoryBytes: 100 });
		expect(rows.flatMap((row) => (row.kind === "server" ? [row.server.port] : []))).toEqual([3000, 4000, 5000]);
	});

	it("gives a nested repository its own group", () => {
		const rows = listRows([web, nested], "repository", []);
		expect(named(rows, "repository").map((row) => row.name)).toEqual(["repo", "pkg"]);
	});

	it("hides a collapsed worktree's servers and still counts them in the repository", () => {
		const treeKey = "worktree:~/repo/.git:~/hotfix";
		const rows = listRows([web, hotfix], "repository", [treeKey]);
		expect(rows.some((row) => row.kind === "server" && row.server.port === 5000)).toBe(false);
		expect(rows.find((row) => row.kind === "repository")).toMatchObject({ memoryBytes: 50, count: 2 });
		expect(toggleCollapsed([treeKey], treeKey)).toEqual([]);
	});

	it("lists servers outside a repository under one header, without worktrees", () => {
		const postgres = server({ port: 5432, name: "postgres", kind: "database" });
		const redis = server({ port: 6379, name: "redis", kind: "database" });
		const rows = listRows([web, postgres, redis], "repository", []);
		expect(named(rows, "worktree").map((row) => row.name)).toEqual(["repo"]);
		expect(rows.slice(-3)).toMatchObject([
			{ kind: "repository", name: "Outside a repository", count: 2 },
			{ kind: "server", key: postgres.id },
			{ kind: "server", key: redis.id },
		]);
	});

	// The list arrives pinned first; grouping may gather servers but never drop or reorder a pin.
	it.each(["project", "repository"] as const)(
		"keeps every server and the pinned one first when grouped by %s",
		(mode) => {
			const loose = server({ port: 1000, name: "loose" });
			const servers = [hotfix, web, loose, api];
			const rows = listRows(servers, mode, []);
			const listed = rows.flatMap((row) => (row.kind === "server" ? [row.server.id] : []));
			expect(listed.toSorted()).toEqual(servers.map((item) => item.id).toSorted());
			expect(listed[0]).toBe(hotfix.id);
		},
	);

	it("lists project groups in the order of their first server, servers outside a project last", () => {
		const loose = server({ port: 1000, name: "loose" });
		const a1 = server({ port: 2000, name: "a", folder: "~/a" });
		const b1 = server({ port: 3000, name: "b", folder: "~/b" });
		const a2 = server({ port: 4000, name: "a", folder: "~/a" });
		const rows = listRows([loose, a1, b1, a2], "project", []);
		expect(named(rows, "project").map((row) => row.name)).toEqual(["a", "b", "Outside a project"]);
		expect(rows.flatMap((row) => (row.kind === "server" ? [row.server.port] : []))).toEqual([2000, 4000, 3000, 1000]);
	});

	it("sums the same servers in project groups", () => {
		const rows = listRows([web, api], "project", []);
		const totals = named(rows, "project").reduce((sum, row) => sum + row.memoryBytes, 0);
		expect(totals).toBe(web.memoryBytes + api.memoryBytes);
	});
});

describe("groupByFromStorage", () => {
	it("keeps the previous project grouping and otherwise stays flat", () => {
		expect(groupByFromStorage(null, "true")).toBe("project");
		expect(groupByFromStorage(null, "false")).toBe("flat");
		expect(groupByFromStorage(JSON.stringify("repository"), "true")).toBe("repository");
	});
});
