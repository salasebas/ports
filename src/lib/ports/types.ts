export type ServerKind = "dev" | "database" | "other";

export type ServerStatus = "running" | "idle" | "attention";

export interface Server {
	port: number;
	pid: number;
	/** Executable that owns the listening socket, e.g. `node` or `postgres`. */
	process: string;
	command: string;
	/** Project folder name, or the process name when no project is detected. */
	name: string;
	folder?: string;
	branch?: string;
	framework?: string;
	kind: ServerKind;
	status: ServerStatus;
	/** Protected servers are skipped by bulk clean up. */
	protected: boolean;
	startedAt: number;
	lastActiveAt: number;
	memoryBytes: number;
	cpuPercent: number;
	/** Samples over the last ten minutes, oldest first. */
	memoryHistory: number[];
	cpuHistory: number[];
}

export interface SystemUsage {
	totalBytes: number;
	otherAppsBytes: number;
	/** Share of the whole machine's CPU, like `Server.cpuPercent`. */
	otherAppsCpuPercent: number;
}
