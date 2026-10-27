import { describe, expect, it } from "vitest";
import { formatBytesText, formatDuration, serverUrl } from "./format";

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

// Bound socket addresses define reachability; localhost cannot reach a NIC-only listener.
describe("serverUrl", () => {
	it.each([
		[["192.168.1.20"], "http://192.168.1.20:3000"],
		[["::"], "http://[::1]:3000"],
		[["2001:db8::1"], "http://[2001:db8::1]:3000"],
		[["0.0.0.0"], "http://127.0.0.1:3000"],
	])("uses a reachable binding for %s", (addresses, expected) => {
		expect(serverUrl({ port: 3000, addresses })).toBe(expected);
	});
});
