/** Shapes of `list_projects` and `pull_request`. Keep them in sync with the Rust commands. */

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
	/** The script body from package.json. */
	command: string;
	/** The command Ports runs for this script. */
	run: string;
	favorite: boolean;
	/** A favorite whose script is no longer in package.json. */
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
	/** Process Ports started, when it may still be running. */
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
	/** A remote deployment URL, never the local server. */
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
