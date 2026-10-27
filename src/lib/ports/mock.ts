import type { Server, SystemMemory } from "./types";

const MB = 1024 ** 2;
const GB = 1024 ** 3;
const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const SAMPLES = 60;

/** Deterministic wobble so the mock charts look alive without randomness. */
function series(start: number, end: number, jitter: number, seed: number): number[] {
	return Array.from({ length: SAMPLES }, (_, i) => {
		const t = i / (SAMPLES - 1);
		const wobble = Math.sin(i * 0.9 + seed) * Math.cos(i * 0.31 + seed * 2);
		return Math.max(0, start + (end - start) * t + wobble * jitter);
	});
}

function cpuSeries(base: number, seed: number): number[] {
	return Array.from({ length: SAMPLES }, (_, i) => {
		const spike = (i + seed) % 23 < 2 ? 40 : 0;
		const wobble = Math.abs(Math.sin(i * 1.7 + seed)) * base;
		return Math.min(100, base * 0.4 + wobble + spike);
	});
}

export function mockServers(now = Date.now()): Server[] {
	const servers: Server[] = [
		{
			port: 1420,
			pid: 48211,
			process: "bun",
			command: "bun run dev",
			name: "ports",
			folder: "~/projects/ports",
			branch: "main",
			framework: "SvelteKit",
			kind: "dev",
			status: "running",
			protected: false,
			startedAt: now - 3 * HOUR - 12 * MINUTE,
			lastActiveAt: now - MINUTE,
			memoryBytes: 1.21 * GB,
			cpuPercent: 12,
			memoryHistory: series(1.1 * GB, 1.21 * GB, 30 * MB, 1),
			cpuHistory: cpuSeries(10, 1),
		},
		{
			port: 3000,
			pid: 39120,
			process: "node",
			command: "next dev",
			name: "storefront",
			folder: "~/projects/storefront",
			branch: "feat/checkout",
			framework: "Next.js",
			kind: "dev",
			status: "running",
			protected: false,
			startedAt: now - 26 * HOUR,
			lastActiveAt: now - 4 * MINUTE,
			memoryBytes: 612 * MB,
			cpuPercent: 4,
			memoryHistory: series(600 * MB, 612 * MB, 12 * MB, 2),
			cpuHistory: cpuSeries(4, 2),
		},
		{
			port: 5173,
			pid: 51877,
			process: "node",
			command: "vite",
			name: "design-docs",
			folder: "~/projects/design-docs",
			branch: "docs/tokens",
			framework: "Vite",
			kind: "dev",
			status: "idle",
			protected: false,
			startedAt: now - 9 * HOUR,
			lastActiveAt: now - 5 * HOUR,
			memoryBytes: 184 * MB,
			cpuPercent: 0,
			memoryHistory: series(184 * MB, 184 * MB, 2 * MB, 3),
			cpuHistory: cpuSeries(0.5, 3),
		},
		{
			port: 8000,
			pid: 22904,
			process: "python3",
			command: "uvicorn app.main:app --reload",
			name: "billing-api",
			folder: "~/projects/billing-api",
			branch: "fix/auth-timeout",
			framework: "FastAPI",
			kind: "dev",
			status: "attention",
			protected: false,
			startedAt: now - 2 * HOUR - 40 * MINUTE,
			lastActiveAt: now,
			memoryBytes: 2.74 * GB,
			cpuPercent: 34,
			memoryHistory: series(1.64 * GB, 2.74 * GB, 40 * MB, 4),
			cpuHistory: cpuSeries(30, 4),
		},
		{
			port: 5432,
			pid: 812,
			process: "postgres",
			command: "postgres -D /opt/homebrew/var/postgresql@16",
			name: "postgres",
			framework: "PostgreSQL",
			kind: "database",
			status: "running",
			protected: true,
			startedAt: now - 4 * 24 * HOUR,
			lastActiveAt: now - 2 * MINUTE,
			memoryBytes: 96 * MB,
			cpuPercent: 1,
			memoryHistory: series(96 * MB, 96 * MB, 1 * MB, 5),
			cpuHistory: cpuSeries(1, 5),
		},
		{
			port: 6379,
			pid: 815,
			process: "redis-server",
			command: "redis-server /opt/homebrew/etc/redis.conf",
			name: "redis",
			framework: "Redis",
			kind: "database",
			status: "running",
			protected: true,
			startedAt: now - 4 * 24 * HOUR,
			lastActiveAt: now - 30 * MINUTE,
			memoryBytes: 14 * MB,
			cpuPercent: 0,
			memoryHistory: series(14 * MB, 14 * MB, 0.5 * MB, 6),
			cpuHistory: cpuSeries(0.3, 6),
		},
	];
	return servers.sort((a, b) => a.port - b.port);
}

export const mockSystemMemory: SystemMemory = {
	totalBytes: 16 * GB,
	otherAppsBytes: 7 * GB,
};
