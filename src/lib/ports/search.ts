import type { Server } from "./types";

/**
 * The port a search asks about: a bare number such as `3000` or `:3000`.
 * The same number also finds a server by PID.
 */
export function portQuery(query: string): number | null {
	const text = query.trim().replace(/^:/, "");
	if (!/^\d{1,5}$/.test(text)) return null;
	const port = Number(text);
	return port >= 1 && port <= 65_535 ? port : null;
}

/** Whether the number is the PID of the server's listener or of any process of its tree. */
export const hasPid = (server: Server, pid: number) =>
	server.pid === pid || server.resources.some((resource) => resource.process.pid === pid);

/** `~/web` and, once the home folder is known, `/Users/me/web`, so a full path finds its server. */
function paths(path: string | undefined, home: string | null): string[] {
	if (!path) return [];
	if (!home || !path.startsWith("~/")) return [path];
	return [path, home.replace(/\/$/, "") + path.slice(1)];
}

/**
 * Whether a search for text matches the server: its names, process, command,
 * framework, branch, session and page title, and its folders, so two
 * projects with the same name differ by path. A number matches the port or
 * a PID exactly. Matching text never counts as evidence of what a server is.
 */
export function matchesSearch(server: Server, query: string, home: string | null = null): boolean {
	const q = query.trim().toLowerCase();
	if (!q) return true;
	const number = portQuery(q);
	if (number !== null) return server.port === number || hasPid(server, number);
	return [
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
