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
