import { describe, expect, it } from "vitest";
import { formatBytesText, formatDuration } from "./format";

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
