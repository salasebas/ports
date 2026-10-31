export type ServerKind = "dev" | "database" | "container" | "other";

export type ServerStatus = "running" | "idle" | "attention";

export interface ProcessIdentity {
	pid: number;
	start: string;
}

export type ServerTarget =
	| { kind: "process"; port: number; listener: ProcessIdentity; root: ProcessIdentity; addresses: string[] }
	| { kind: "container"; port: number; id: string; engine: string; addresses: string[] };

export interface ProcessUsage {
	process: ProcessIdentity;
	parent?: number;
	name: string;
	ports: number[];
	memoryBytes: number;
	cpuPercent: number;
}

export interface Supervisor {
	kind: "homebrew" | "pm2" | "supervisord";
	name?: string;
	controllable: boolean;
}

export interface ParentProcess {
	process: ProcessIdentity;
	name: string;
}

export interface AffectedProcess {
	process: ProcessIdentity;
	name: string;
	ports: number[];
}

export interface ControlPreview {
	processes: AffectedProcess[];
	supervisor?: Supervisor;
}

export type Confidence = "confirmed" | "likely" | "unknown";

export type Evidence =
	| { source: "marker"; file: string }
	| { source: "command"; name: string }
	| { source: "dependency"; manifest: string; name: string }
	| { source: "image"; name: string }
	| { source: "wellKnownPort"; port: number };

export interface Attribution {
	confidence: Confidence;
	evidence: Evidence[];
}

export type SessionAgent = "cursor" | "claude" | "codex" | "terminal";

export type SessionEvidence = { source: "ancestor"; name: string } | { source: "environment"; name: string };

export type Resume =
	| { state: "available"; label: string; path: string }
	| { state: "missing"; message: string }
	| { state: "unsupported"; message: string };

export interface Session {
	agent: SessionAgent;
	label: string;
	evidence: SessionEvidence[];
	resume: Resume;
}

export type Web =
	| { state: "unknown" }
	| { state: "page"; scheme: "http" | "https"; title?: string }
	| { state: "none" };

export interface Server {
	id: string;
	target?: ServerTarget;
	resources: ProcessUsage[];
	port: number;
	pid?: number;
	owner?: string;
	container?: { id: string; image: string };
	process: string;
	command: string;
	name: string;
	folder?: string;
	cwd?: string;
	cwdDeleted: boolean;
	projectName?: string;
	branch?: string;
	worktree?: string;
	worktreePath?: string;
	repository?: string;
	repositoryName?: string;
	repositoryId?: string;
	session?: Session;
	framework?: string;
	frameworkVersion?: string;
	attribution: Attribution;
	web: Web;
	icon?: string;
	kind: ServerKind;
	status: ServerStatus;
	responding: boolean;
	addresses: string[];
	exposed: boolean;
	lanHost?: string;
	protected: boolean;
	hidden: boolean;
	restartable: boolean;
	supervisor?: Supervisor;
	parent?: ParentProcess;
	startedAt: number;
	lastActiveAt: number;
	memoryBytes: number;
	cpuPercent: number;
	memoryHistory: number[];
	cpuHistory: number[];
}

export interface Usage {
	memoryBytes: number;
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
