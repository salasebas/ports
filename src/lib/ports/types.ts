/** Shapes sent by `src-tauri/src/ports/sampler.rs`; keep both in sync. */

/** `other` covers desktop apps and system services. */
export type ServerKind = "dev" | "database" | "container" | "other";

export type ServerStatus = "running" | "idle" | "attention";

export interface ProcessIdentity {
	pid: number;
	start: string;
}

export type ServerTarget =
	| { kind: "process"; port: number; listener: ProcessIdentity; root: ProcessIdentity; addresses: string[] }
	| { kind: "container"; port: number; id: string; engine: string; addresses: string[] };

/** One process of a server's tree. */
export interface ProcessUsage {
	process: ProcessIdentity;
	/** Parent PID; the tree's top process may have a parent outside the server. */
	parent?: number;
	name: string;
	/** The ports it listens on itself, e.g. each worker of a prefork server. */
	ports: number[];
	memoryBytes: number;
	cpuPercent: number;
}

/** What starts a server again when it exits. */
export interface Supervisor {
	kind: "homebrew" | "pm2" | "supervisord";
	/** The Homebrew formula, or the PM2 app once PM2 confirmed it. */
	name?: string;
	/** Ports stops and restarts it through the supervisor; otherwise the supervisor may start it again. */
	supported: boolean;
}

/** What started a server and may start it again, such as nodemon. */
export interface ParentProcess {
	process: ProcessIdentity;
	name: string;
}

/** One process a control ends. */
export interface AffectedProcess {
	process: ProcessIdentity;
	name: string;
	ports: number[];
}

/** What a stop, restart or Force Kill would end, from `preview_control`. */
export interface ControlPreview {
	/** Empty for a container, which Docker controls as a whole. */
	processes: AffectedProcess[];
	supervisor?: Supervisor;
}

/** How sure Ports is of a server's framework; never a score. */
export type Confidence = "confirmed" | "likely" | "unknown";

/** One reason behind what Ports says a server is. */
export type Evidence =
	/** A file that makes the folder a project, such as `package.json` or `.git`. */
	| { source: "marker"; file: string }
	/** The executable, or the script or Python module it runs. */
	| { source: "command"; name: string }
	/** A dependency the project declares: it refines a runner, or alone only suggests the framework. */
	| { source: "dependency"; manifest: string; name: string }
	| { source: "image"; name: string }
	/** What usually listens on the port, for a process Ports can't see. */
	| { source: "wellKnownPort"; port: number };

/** Why Ports names a server's project and framework. */
export interface Attribution {
	/** `confirmed` when the command or image names the framework, `likely` when only dependencies or the port do. */
	confidence: Confidence;
	evidence: Evidence[];
}

export type SessionAgent = "cursor" | "claude" | "codex" | "terminal";

/** Evidence that a process tree or an allowlisted variable names a session. Never a command argument. */
export type SessionEvidence = { source: "ancestor"; name: string } | { source: "environment"; name: string };

export type Resume =
	| { state: "available"; label: string; path: string }
	| { state: "missing"; message: string }
	| { state: "unsupported"; message: string };

/** Who started a server, when Ports can tell. Absent means the session is unknown. */
export interface Session {
	agent: SessionAgent;
	label: string;
	evidence: SessionEvidence[];
	resume: Resume;
}

/**
 * Whether the server answered a request for `/`. `unknown` until checked;
 * databases, desktop apps and other users' processes are never checked.
 * `none` means it answered neither HTTPS nor HTTP.
 */
export type Web =
	| { state: "unknown" }
	| { state: "page"; scheme: "http" | "https"; title?: string }
	| { state: "none" };

export interface Server {
	id: string;
	target?: ServerTarget;
	resources: ProcessUsage[];
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
	/** The project's root folder, e.g. `~/projects/web` or `/workspace/api`. */
	folder?: string;
	/** Where the process runs, which may be below `folder` or outside any project. */
	cwd?: string;
	/** The folder the process runs in was deleted, such as a removed worktree. False when Ports can't tell. */
	cwdDeleted: boolean;
	/** The name the project's manifest gives it, which may differ from the folder's. */
	projectName?: string;
	branch?: string;
	/** Linked Git worktree name. Absent on the main checkout. */
	worktree?: string;
	/** This checkout's folder. Different for each worktree of one repository. */
	worktreePath?: string;
	/** The repository's main folder, shared by its worktrees. */
	repository?: string;
	repositoryName?: string;
	/** Common git directory. The grouping key for one repository. */
	repositoryId?: string;
	/** Set only when ancestry or an allowlisted variable names an agent or terminal. */
	session?: Session;
	framework?: string;
	/** The installed version of `framework`, when the project pins one. */
	frameworkVersion?: string;
	attribution: Attribution;
	web: Web;
	/** Key of the page's favicon or the project's icon file, loaded through `iconUrl`. */
	icon?: string;
	kind: ServerKind;
	/** `attention` covers a memory leak and a server that stopped answering. */
	status: ServerStatus;
	/** False when the last health probe got no answer: the server holds its port but hangs. */
	responding: boolean;
	/** Bound addresses, e.g. `127.0.0.1` for this machine only or `0.0.0.0` for every interface. */
	addresses: string[];
	/** Reachable from other devices on the network. */
	exposed: boolean;
	/** Where another device on the network reaches it, e.g. `192.168.1.20:5173`; `lanUrl` adds the scheme. */
	lanHost?: string;
	/** Protected servers are skipped by bulk clean up. */
	protected: boolean;
	/** Left out of both lists by the user's rules: hidden entries, the port range and the filters. */
	hidden: boolean;
	/** Ports can launch it again the same way; false for system services. */
	restartable: boolean;
	supervisor?: Supervisor;
	/** Stop parent ends it too, so it can't start the server again. */
	parent?: ParentProcess;
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
	usedBytes: number;
	cpuPercent: number;
}

export interface Snapshot {
	revision: number;
	servers: Server[];
	system: SystemUsage;
}
