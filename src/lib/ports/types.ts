/** Shapes sent by `src-tauri/src/ports/sampler.rs`; keep both in sync. */

/** `other` covers desktop apps and system services. */
export type ServerKind = "dev" | "database" | "container" | "other";

export type ServerStatus = "running" | "idle" | "attention";

export interface Server {
	port: number;
	/**
	 * Missing when another user's process holds the port and the system
	 * hides it (Linux): then only `owner` is known, there's no memory or
	 * CPU, and Ports can't stop it.
	 */
	pid?: number;
	owner?: string;
	/** A port a container publishes: Ports stops and restarts the container through Docker. */
	container?: { id: string; image: string };
	/** Executable that owns the listening socket, e.g. `node` or `postgres`. */
	process: string;
	/** What was launched, e.g. `npm run dev` above the `node` that listens. */
	command: string;
	/** Project folder name, or the process name when no project is detected. */
	name: string;
	folder?: string;
	branch?: string;
	framework?: string;
	kind: ServerKind;
	/** `attention` covers a memory leak and a server that stopped answering. */
	status: ServerStatus;
	/** False when the last health probe got no answer: the server holds its port but hangs. */
	responding: boolean;
	/** Bound addresses, e.g. `127.0.0.1` for this machine only or `0.0.0.0` for every interface. */
	addresses: string[];
	/** Reachable from other devices on the network. */
	exposed: boolean;
	/** Where another device on the network opens it, e.g. `http://192.168.1.20:5173`. */
	lanUrl?: string;
	/** Protected servers are skipped by bulk clean up. */
	protected: boolean;
	/** Ports can launch it again the same way; false for system services. */
	restartable: boolean;
	startedAt: number;
	lastActiveAt: number;
	/** The server's whole process tree. */
	memoryBytes: number;
	/** Share of the whole machine's CPU. */
	cpuPercent: number;
	/** Samples over the last ten minutes, oldest first. */
	memoryHistory: number[];
	cpuHistory: number[];
}

export interface Usage {
	memoryBytes: number;
	/** Share of the whole machine's CPU. */
	cpuPercent: number;
}

export interface SystemUsage {
	totalBytes: number;
	/** The servers of each kind, counting a process that holds several ports once. */
	kinds: Partial<Record<ServerKind, Usage>>;
	/** Everything else in use, outside every server. */
	otherAppsBytes: number;
	/** Share of the whole machine's CPU, like `Server.cpuPercent`. */
	otherAppsCpuPercent: number;
}

export interface Snapshot {
	servers: Server[];
	system: SystemUsage;
}
