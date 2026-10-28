import type { ProcessIdentity, ProcessUsage, Server, Usage } from "./types";

export const processKey = (process: ProcessIdentity) => `${process.pid}:${process.start}`;

export function controlKey(server: Server): string {
	const target = server.target;
	if (!target) return server.id;
	return target.kind === "process" ? `process:${processKey(target.root)}` : `container:${target.engine}:${target.id}`;
}

export function sharesControl(a: Server, b: Server): boolean {
	if (controlKey(a) === controlKey(b)) return true;
	const processes = new Set(a.resources.map((resource) => processKey(resource.process)));
	return b.resources.some((resource) => processes.has(processKey(resource.process)));
}

/** Each process contributes to one visible slice, including overlapping service trees. */
export function allocateUsage(servers: readonly Server[]): { server: Server; usage: Usage }[] {
	const counted = new Set<string>();
	return servers.map((server) => {
		const usage = { memoryBytes: 0, cpuPercent: 0 };
		for (const resource of server.resources) {
			const key = processKey(resource.process);
			if (counted.has(key)) continue;
			counted.add(key);
			usage.memoryBytes += resource.memoryBytes;
			usage.cpuPercent += resource.cpuPercent;
		}
		return { server, usage };
	});
}

/**
 * The server's processes as a tree, parents before their children, with
 * each one's depth. A process whose parent isn't part of the server is a top.
 */
export function processTree(server: Server): { usage: ProcessUsage; depth: number }[] {
	const pids = new Set(server.resources.map((resource) => resource.process.pid));
	const children = new Map<number | undefined, ProcessUsage[]>();
	for (const resource of server.resources.toSorted((a, b) => a.process.pid - b.process.pid)) {
		const parent =
			resource.parent !== undefined && resource.parent !== resource.process.pid && pids.has(resource.parent)
				? resource.parent
				: undefined;
		children.set(parent, [...(children.get(parent) ?? []), resource]);
	}
	const rows: { usage: ProcessUsage; depth: number }[] = [];
	const visit = (parent: number | undefined, depth: number) => {
		for (const usage of children.get(parent) ?? []) {
			rows.push({ usage, depth });
			visit(usage.process.pid, depth + 1);
		}
	};
	visit(undefined, 0);
	return rows;
}
