import { convertFileSrc, isTauri } from "@tauri-apps/api/core";
import type { Evidence, Health, HealthState, MemoryAlert, Server } from "./types";

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

export const formatClock = (at: number) =>
	new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });

export const formatSnoozeEnd = (at: number) =>
	new Date(at).toLocaleString([], { weekday: "short", hour: "2-digit", minute: "2-digit" });

export const isTimedOut = (server: Pick<Server, "health">) => server.health.state === "timedOut";

const healthLabels: Record<HealthState, string> = {
	unknown: "Not checked",
	warming: "Warming up",
	responsive: "Responding",
	timedOut: "Timed out",
};

export function healthLabel(health: Health): string {
	const label = healthLabels[health.state];
	return health.checkedAt === undefined ? label : `${label} · ${formatClock(health.checkedAt)}`;
}

export function describeMemoryAlert(alert: MemoryAlert): string {
	return alert.reason === "growth"
		? `Grew by ${formatBytesText(alert.bytes)} in ${alert.minutes} minutes. Steady growth can mean a leak, but doesn't prove one.`
		: `Uses ${formatBytesText(alert.usedBytes)}, above your ${formatBytesText(alert.thresholdBytes)} alert.`;
}

export function serverActivity(server: Server, now = Date.now()): string {
	if (isTimedOut(server)) return "not responding";
	if (server.health.state === "warming") return "warming up";
	if (server.growth) return `+${formatBytesText(server.growth.bytes)} in ${server.growth.minutes} min`;
	if (server.status === "idle") return `idle ${formatDuration(now - server.lastActiveAt)}`;
	return `up ${formatDuration(now - server.startedAt)}`;
}

export function serverProcessId(server: Server): string {
	if (server.pid !== undefined) return `PID ${server.pid}`;
	if (server.container) return `container ${server.container.id.slice(0, 12)}`;
	return `owned by ${server.owner}`;
}

export const isMeasured = (server: Server) => server.resources.length > 0;

export const canStop = (server: Server) => server.target !== undefined;

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

export const exposeFlag = (server: Server): string | undefined =>
	server.framework ? exposeFlags[server.framework] : undefined;

const reachesLocalhost = (ip: string) => ["0.0.0.0", "::", "127.0.0.1", "::1"].includes(ip);

const scheme = (server: Pick<Server, "web">) => (server.web.state === "page" ? server.web.scheme : "http");

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

export const lanUrl = (server: Pick<Server, "lanHost" | "web">) =>
	server.lanHost ? `${scheme(server)}://${server.lanHost}` : undefined;

export function placeLabel(server: Pick<Server, "worktree" | "branch">): string | undefined {
	const parts = [server.worktree, server.branch].filter((part): part is string => Boolean(part));
	const unique = parts.filter((part, index) => parts.indexOf(part) === index);
	return unique.length ? unique.join(" · ") : undefined;
}

export const confirmedFramework = (server: Server) =>
	server.attribution.confidence === "confirmed" ? server.framework : undefined;

export const frameworkLabel = (server: Server) =>
	server.framework && [server.framework, server.frameworkVersion].filter(Boolean).join(" ");

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

const ICON_SCHEME = "ports-icon";

export const iconUrl = (key: string | undefined) => (key && isTauri() ? convertFileSrc(key, ICON_SCHEME) : undefined);

export function portColor(port: number): string {
	return `var(--port-${(port % 6) + 1})`;
}
