import type { ProcessIdentity } from "./types";

export type Recovery = "manual";

export type LaunchOutcome = "started" | "exited" | "failed" | "conflict" | "missingFolder" | "stopped";

export interface ConflictOwner {
	port: number;
	name: string;
	pid?: number;
	serverId?: string;
}

export interface LaunchResult {
	at: number;
	outcome: LaunchOutcome;
	message: string;
	exitCode?: number;
	owner?: ConflictOwner;
}

export interface ScriptView {
	name: string;
	body: string;
	runCommand: string;
	favorite: boolean;
	missing: boolean;
}

export interface SavedProject {
	id: string;
	folder: string;
	command: string;
	favorites: string[];
	expectedPort: number;
	expectedAddress: string;
	recovery: Recovery;
	lastResult?: LaunchResult;
	process?: ProcessIdentity;
	manager: "npm" | "pnpm" | "yarn" | "bun";
	scripts: ScriptView[];
	folderMissing: boolean;
	running: boolean;
	matchedServerId?: string;
}

export interface ProjectLog {
	text: string;
	truncated: boolean;
}

export interface Pull {
	number: number;
	title: string;
	url: string;
	branch: string;
}

export interface Preview {
	provider: string;
	url: string;
	label: string;
}

export type PullLookup =
	| { state: "found"; pull: Pull; preview?: Preview }
	| { state: "none"; message: string }
	| { state: "ambiguous"; message: string; pulls: Pull[] }
	| { state: "unauthenticated"; message: string }
	| { state: "offline"; message: string }
	| { state: "unavailable"; message: string }
	| { state: "unsupported"; message: string };

export function projectName(project: Pick<SavedProject, "folder">): string {
	return project.folder.split("/").filter(Boolean).at(-1) ?? project.folder;
}

export function savedMatches(project: SavedProject, query: string): boolean {
	const q = query.trim().toLowerCase();
	if (!q) return true;
	return [projectName(project), project.folder, project.command, project.expectedPort, project.expectedAddress].some(
		(field) => String(field).toLowerCase().includes(q),
	);
}
