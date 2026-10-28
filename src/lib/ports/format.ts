import { convertFileSrc, isTauri } from "@tauri-apps/api/core";
import type { Evidence, Server } from "./types";

const MB = 1024 ** 2;
const GB = 1024 ** 3;

export function formatBytes(bytes: number): { value: string; unit: string } {
	if (bytes >= GB) {
		const value = bytes / GB;
		return { value: value.toFixed(value >= 10 ? 1 : 2), unit: "GB" };
	}
	return { value: Math.round(bytes / MB).toString(), unit: "MB" };
}

export function formatBytesText(bytes: number): string {
	const { value, unit } = formatBytes(bytes);
	return `${value} ${unit}`;
}

export function formatPercent(percent: number): string {
	return `${Math.round(percent)}%`;
}

export function formatDuration(ms: number): string {
	const minutes = Math.floor(ms / 60_000);
	if (minutes < 1) return "now";
	if (minutes < 60) return `${minutes}m`;
	const hours = Math.floor(minutes / 60);
	if (hours < 24) return minutes % 60 ? `${hours}h ${minutes % 60}m` : `${hours}h`;
	return `${Math.floor(hours / 24)}d`;
}

export function serverActivity(server: Server, now = Date.now()): string {
	if (!server.responding) return "not responding";
	if (server.status === "attention") {
		const growth = server.memoryHistory.at(-1)! - server.memoryHistory[0];
		return `+${formatBytesText(growth)} in 10 min`;
	}
	if (server.status === "idle") return `idle ${formatDuration(now - server.lastActiveAt)}`;
	return `up ${formatDuration(now - server.startedAt)}`;
}

/** `PID 4821`, the container, or who owns the process when Ports can't see it. */
export function serverProcessId(server: Server): string {
	if (server.pid !== undefined) return `PID ${server.pid}`;
	if (server.container) return `container ${server.container.id.slice(0, 12)}`;
	return `owned by ${server.owner}`;
}

/** Memory and CPU are measured for processes Ports can see, not for containers or other users' processes. */
export const isMeasured = (server: Server) => server.resources.length > 0;

export const canStop = (server: Server) => server.target !== undefined;

/** How to start each framework's dev server on every interface instead of loopback only. */
const exposeFlags: Record<string, string> = {
	Vite: "--host",
	SvelteKit: "--host",
	Astro: "--host",
	Nuxt: "--host",
	Remix: "--host",
	"React Router": "--host",
	Angular: "--host 0.0.0.0",
	"Next.js": "-H 0.0.0.0",
	Gatsby: "-H 0.0.0.0",
	Django: "0.0.0.0:8000",
	Flask: "--host=0.0.0.0",
	FastAPI: "--host 0.0.0.0",
	Uvicorn: "--host 0.0.0.0",
	Rails: "-b 0.0.0.0",
	Puma: "-b tcp://0.0.0.0",
	Laravel: "--host=0.0.0.0",
	Hugo: "--bind 0.0.0.0",
};

/** The flag that exposes the server, e.g. `--host` for Vite, when Ports knows it. */
export const exposeFlag = (server: Server): string | undefined =>
	server.framework ? exposeFlags[server.framework] : undefined;

const reachesLocalhost = (ip: string) => ["0.0.0.0", "::", "127.0.0.1", "::1"].includes(ip);

/** The scheme of the page Ports found; plain HTTP when it found none, which is what most servers speak. */
const scheme = (server: Pick<Server, "web">) => (server.web.state === "page" ? server.web.scheme : "http");

/** The server's page, when Ports verified it answers HTTPS or HTTP. */
export const verifiedPage = (server: Pick<Server, "web">) => (server.web.state === "page" ? server.web : undefined);

/**
 * Where Open and Copy go: `localhost` whenever it reaches the server, since
 * apps set up cookies, CORS and OAuth for that origin, with the verified scheme.
 */
export function serverUrl(server: Pick<Server, "port" | "addresses" | "web">): string {
	const concrete =
		server.addresses.length && !server.addresses.some(reachesLocalhost) ? server.addresses[0] : "localhost";
	const host = concrete.includes(":") ? `[${concrete}]` : concrete;
	return `${scheme(server)}://${host}:${server.port}`;
}

/** Where another device opens the server, with the same scheme as `serverUrl`. */
export const lanUrl = (server: Pick<Server, "lanHost" | "web">) =>
	server.lanHost ? `${scheme(server)}://${server.lanHost}` : undefined;

/** The framework when its command or image names it; a guess from dependencies stays in the details. */
export const confirmedFramework = (server: Server) =>
	server.attribution.confidence === "confirmed" ? server.framework : undefined;

/** `SvelteKit 2.21.0`, or the framework alone when its version is unknown. */
export const frameworkLabel = (server: Server) =>
	server.framework && [server.framework, server.frameworkVersion].filter(Boolean).join(" ");

/** One line of the details' "Identified by" list. */
export function describeEvidence(evidence: Evidence): string {
	switch (evidence.source) {
		case "marker":
			return `${evidence.file} marks the project folder`;
		case "command":
			return `Runs ${evidence.name}`;
		case "dependency":
			return `${evidence.manifest} depends on ${evidence.name}`;
		case "image":
			return `Container image ${evidence.name}`;
		case "wellKnownPort":
			return `Port ${evidence.port} usually belongs to it`;
	}
}

/** Must match `SCHEME` in `src-tauri/src/ports/icons.rs`. */
const ICON_SCHEME = "ports-icon";

/** The URL of a page's or project's icon; a plain browser tab has no backend to serve it. */
export const iconUrl = (key: string | undefined) => (key && isTauri() ? convertFileSrc(key, ICON_SCHEME) : undefined);

/** Stable color slot so a port keeps its color across refreshes. */
export function portColor(port: number): string {
	return `var(--port-${(port % 6) + 1})`;
}
