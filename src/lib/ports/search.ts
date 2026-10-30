import type { Server } from "./types";

export function portQuery(query: string): number | null {
	const text = query.trim().replace(/^:/, "");
	if (!/^\d{1,5}$/.test(text)) return null;
	const port = Number(text);
	return port >= 1 && port <= 65_535 ? port : null;
}

export const hasPid = (server: Server, pid: number) =>
	server.pid === pid || server.resources.some((resource) => resource.process.pid === pid);

function paths(path: string | undefined, home: string | null): string[] {
	if (!path) return [];
	if (!home || !path.startsWith("~/")) return [path];
	return [path, home.replace(/\/$/, "") + path.slice(1)];
}

export function matchesSearch(server: Server, query: string, home: string | null = null): boolean {
	const q = query.trim().toLowerCase();
	if (!q) return true;
	return [
		server.port,
		server.pid,
		server.name,
		server.process,
		server.command,
		server.framework,
		server.branch,
		server.worktree,
		server.repositoryName,
		server.session?.label,
		server.projectName,
		server.web.state === "page" && server.web.title,
		...paths(server.folder, home),
		...paths(server.cwd, home),
		...paths(server.worktreePath, home),
		...paths(server.repository, home),
	]
		.filter(Boolean)
		.some((field) => String(field).toLowerCase().includes(q));
}
