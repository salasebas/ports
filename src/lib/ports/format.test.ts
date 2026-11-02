import { describe, expect, it } from "vitest";
import { mockServers } from "./mock";
import {
	describeMemoryAlert,
	formatBytesText,
	formatClock,
	formatDuration,
	healthLabel,
	lanUrl,
	serverActivity,
	serverUrl,
} from "./format";
import type { Server, Web } from "./types";

const minute = 60_000;
const hour = 60 * minute;

const server = (overrides: Partial<Server>): Server => ({
	...mockServers(0)[0],
	status: "running",
	startedAt: -2 * hour,
	...overrides,
});

describe("formatDuration", () => {
	it.each([
		[30_000, "now"],
		[5 * minute, "5m"],
		[2 * hour, "2h"],
		[2 * hour + 5 * minute, "2h 5m"],
		[3 * 24 * hour + hour, "3d"],
	])("shows %i ms as %s", (ms, expected) => {
		expect(formatDuration(ms)).toBe(expected);
	});
});

describe("formatBytesText", () => {
	it.each([
		[512 * 1024 ** 2, "512 MB"],
		[1.5 * 1024 ** 3, "1.50 GB"],
		[12 * 1024 ** 3, "12.0 GB"],
	])("shows %i bytes as %s", (bytes, expected) => {
		expect(formatBytesText(bytes)).toBe(expected);
	});
});

describe("serverActivity", () => {
	it("shows a server warming up rather than its uptime", () => {
		expect(serverActivity(server({ health: { state: "warming" } }), 0)).toBe("warming up");
		expect(serverActivity(server({ health: { state: "responsive" } }), 0)).toBe("up 2h");
	});

	it("puts a timeout ahead of growth", () => {
		const growth = { bytes: 100 * 1024 ** 2, minutes: 5 };
		expect(serverActivity(server({ status: "attention", growth }), 0)).toBe("+100 MB in 5 min");
		expect(serverActivity(server({ status: "attention", growth, health: { state: "timedOut" } }), 0)).toBe(
			"not responding",
		);
	});
});

describe("healthLabel", () => {
	it("names every state and adds the time of the last check", () => {
		expect(healthLabel({ state: "unknown" })).toBe("Not checked");
		expect(healthLabel({ state: "warming" })).toBe("Warming up");
		expect(healthLabel({ state: "timedOut", checkedAt: 0 })).toBe(`Timed out · ${formatClock(0)}`);
		expect(healthLabel({ state: "responsive", checkedAt: 0 })).toBe(`Responding · ${formatClock(0)}`);
	});
});

describe("describeMemoryAlert", () => {
	it("tells growth from high usage and doesn't call growth a leak", () => {
		expect(describeMemoryAlert({ reason: "growth", bytes: 600 * 1024 ** 2, minutes: 10 })).toBe(
			"Grew by 600 MB in 10 minutes. Steady growth can mean a leak, but doesn't prove one.",
		);
		expect(describeMemoryAlert({ reason: "threshold", usedBytes: 3 * 1024 ** 3, thresholdBytes: 2 * 1024 ** 3 })).toBe(
			"Uses 3.00 GB, above your 2.00 GB alert.",
		);
	});
});

describe("serverUrl", () => {
	it.each([
		[["192.168.1.20"], "http://192.168.1.20:3000"],
		[["2001:db8::1"], "http://[2001:db8::1]:3000"],
		[["::"], "http://localhost:3000"],
		[["0.0.0.0"], "http://localhost:3000"],
		[["127.0.0.1", "::1"], "http://localhost:3000"],
		[[], "http://localhost:3000"],
		[["127.0.0.2"], "http://127.0.0.2:3000"],
	])("uses a reachable binding for %s", (addresses, expected) => {
		expect(serverUrl({ port: 3000, addresses, web: { state: "unknown" } })).toBe(expected);
	});

	it("opens a verified HTTPS page with https on the same host", () => {
		const web: Web = { state: "page", scheme: "https" };
		expect(serverUrl({ port: 5173, addresses: ["::1"], web })).toBe("https://localhost:5173");
		expect(serverUrl({ port: 5173, addresses: ["2001:db8::1"], web })).toBe("https://[2001:db8::1]:5173");
	});
});

describe("lanUrl", () => {
	it.each<[Web, string]>([
		[{ state: "page", scheme: "https" }, "https://192.168.1.20:5173"],
		[{ state: "page", scheme: "http" }, "http://192.168.1.20:5173"],
		[{ state: "none" }, "http://192.168.1.20:5173"],
	])("uses the scheme of %o", (web, expected) => {
		expect(lanUrl({ lanHost: "192.168.1.20:5173", web })).toBe(expected);
	});

	it("has none when other devices can't reach it", () => {
		expect(lanUrl({ web: { state: "page", scheme: "https" } })).toBeUndefined();
	});
});
