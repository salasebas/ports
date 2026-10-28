import type { Server } from "./types";

export type GroupBy = "flat" | "project" | "repository";

export interface GroupRow {
	kind: "project" | "repository" | "worktree";
	key: string;
	name: string;
	path?: string;
	detail?: string;
	count: number;
	memoryBytes: number;
	cpuPercent: number;
	/** Worktrees sit under their repository. */
	depth: 0 | 1;
}

export type ListRow = { kind: "server"; key: string; server: Server } | GroupRow;

interface Bucket {
	key: string;
	name: string;
	path?: string;
	detail?: string;
	servers: Server[];
	children: Map<string, Bucket>;
}

const bucket = (key: string, name: string, path?: string, detail?: string): Bucket => ({
	key,
	name,
	path,
	detail,
	servers: [],
	children: new Map(),
});

const usage = (servers: Server[]) => ({
	memoryBytes: servers.reduce((sum, server) => sum + server.memoryBytes, 0),
	cpuPercent: servers.reduce((sum, server) => sum + server.cpuPercent, 0),
});

/** Reads a stored grouping, including the previous on/off project setting. */
export function groupByFromStorage(stored: string | null, legacy: string | null): GroupBy {
	const value = stored ? parseStored(stored) : null;
	if (value) return value;
	if (legacy === "true") return "project";
	return "flat";
}

function parseStored(stored: string): GroupBy | null {
	try {
		const parsed: unknown = JSON.parse(stored);
		if (parsed === "flat" || parsed === "project" || parsed === "repository") return parsed;
	} catch {
		if (stored === "flat" || stored === "project" || stored === "repository") return stored;
	}
	return null;
}

export function toggleCollapsed(current: readonly string[], key: string): string[] {
	return current.includes(key) ? current.filter((item) => item !== key) : [...current, key];
}

/**
 * The list, in `servers` order, grouped when asked.
 * Pinning and hiding stay on each server; a group's totals are the sum of the
 * servers it still contains, so changing the grouping does not move or drop one.
 */
export function listRows(servers: Server[], mode: GroupBy, collapsed: readonly string[]): ListRow[] {
	if (mode === "flat") return servers.map((server) => ({ kind: "server", key: server.id, server }));
	if (mode === "project") return projectRows(servers, collapsed);
	return repositoryRows(servers, collapsed);
}

function projectRows(servers: Server[], collapsed: readonly string[]): ListRow[] {
	const groups = new Map<string, Bucket>();
	for (const server of servers) {
		const key = `project:${server.folder ?? ""}`;
		const group =
			groups.get(key) ??
			bucket(key, server.folder?.split("/").filter(Boolean).at(-1) ?? "Outside a project", server.folder);
		group.servers.push(server);
		groups.set(key, group);
	}
	const ordered = [...groups.values()].toSorted((a, b) => Number(!a.path) - Number(!b.path));
	return ordered.flatMap((group) => headerAndServers("project", group, 0, collapsed));
}

function repositoryRows(servers: Server[], collapsed: readonly string[]): ListRow[] {
	const repos = new Map<string, Bucket>();
	for (const server of servers) {
		const repoId = server.repositoryId ?? "";
		const repoKey = `repo:${repoId}`;
		const repo =
			repos.get(repoKey) ?? bucket(repoKey, server.repositoryName ?? "Outside a repository", server.repository);
		repo.servers.push(server);
		const treeId = server.worktreePath ?? server.folder ?? server.id;
		const treeKey = `worktree:${repoId}:${treeId}`;
		const tree =
			repo.children.get(treeKey) ??
			bucket(
				treeKey,
				server.worktree ?? server.repositoryName ?? server.folder?.split("/").filter(Boolean).at(-1) ?? server.name,
				server.worktreePath ?? server.folder,
				server.branch,
			);
		tree.servers.push(server);
		repo.children.set(treeKey, tree);
		repos.set(repoKey, repo);
	}
	const ordered = [...repos.values()].toSorted((a, b) => Number(a.key === "repo:") - Number(b.key === "repo:"));
	return ordered.flatMap((repo) => {
		const rows: ListRow[] = [groupRow("repository", repo, 0)];
		if (collapsed.includes(repo.key)) return rows;
		for (const tree of repo.children.values()) {
			rows.push(...headerAndServers("worktree", tree, 1, collapsed));
		}
		return rows;
	});
}

function headerAndServers(
	kind: GroupRow["kind"],
	group: Bucket,
	depth: 0 | 1,
	collapsed: readonly string[],
): ListRow[] {
	const rows: ListRow[] = [groupRow(kind, group, depth)];
	if (collapsed.includes(group.key)) return rows;
	return [...rows, ...group.servers.map((server) => ({ kind: "server" as const, key: server.id, server }))];
}

function groupRow(kind: GroupRow["kind"], group: Bucket, depth: 0 | 1): GroupRow {
	const totals = usage(group.servers);
	return {
		kind,
		key: group.key,
		name: group.name,
		path: group.path,
		detail: group.detail,
		count: group.servers.length,
		depth,
		...totals,
	};
}
