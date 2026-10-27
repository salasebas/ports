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

export function formatDuration(ms: number): string {
	const minutes = Math.floor(ms / 60_000);
	if (minutes < 1) return "now";
	if (minutes < 60) return `${minutes}m`;
	const hours = Math.floor(minutes / 60);
	if (hours < 24) return minutes % 60 ? `${hours}h ${minutes % 60}m` : `${hours}h`;
	return `${Math.floor(hours / 24)}d`;
}

export function serverActivity(server: Server, now = Date.now()): string {
	if (server.status === "attention") {
		const growth = server.memoryHistory.at(-1)! - server.memoryHistory[0];
		return `+${formatBytesText(growth)} in 10 min`;
	}
	if (server.status === "idle") return `idle ${formatDuration(now - server.lastActiveAt)}`;
	return `up ${formatDuration(now - server.startedAt)}`;
}

export function serverUrl(port: number): string {
	return `http://localhost:${port}`;
}

/** Stable color slot so a port keeps its color across refreshes. */
export function portColor(port: number): string {
	return `var(--port-${(port % 6) + 1})`;
}
