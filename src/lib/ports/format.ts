import type { Server } from "./types";

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

/** `localhost` whenever it reaches the server, since apps set up cookies, CORS and OAuth for that origin. */
export function serverUrl(server: Pick<Server, "port" | "addresses">): string {
	const concrete =
		server.addresses.length && !server.addresses.some(reachesLocalhost) ? server.addresses[0] : "localhost";
	const host = concrete.includes(":") ? `[${concrete}]` : concrete;
	return `http://${host}:${server.port}`;
}

/** Stable color slot so a port keeps its color across refreshes. */
export function portColor(port: number): string {
	return `var(--port-${(port % 6) + 1})`;
}
