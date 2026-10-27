import type { ProcessIdentity, Server, Usage } from "./types";

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
