import { describe, expect, it } from "vitest";
import { formatBytesText, formatDuration, lanUrl, serverUrl } from "./format";
import type { Web } from "./types";

const minute = 60_000;
const hour = 60 * minute;

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
