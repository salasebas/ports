import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it, vi } from "vitest";
import { RulesStore, type RulesChange, type RulesView } from "./rules.svelte";

afterEach(() => {
	clearMocks();
	vi.unstubAllGlobals();
	localStorage.clear();
});

const view: RulesView = {
	revision: 0,
	rules: {
		version: 1,
		portRange: null,
		include: [],
		exclude: [],
		hidden: [],
		protection: {},
		autoCleanup: { policy: null, paused: false },
		shownKinds: ["dev", "database", "container"],
		snoozes: {},
		watches: {},
	},
};

it("moves hidden servers, protection and shown kinds from the window's storage to the backend once", async () => {
	vi.stubGlobal("isTauri", true);
	localStorage.setItem("ports.hidden", JSON.stringify(["port:5000"]));
	localStorage.setItem("ports.protection", JSON.stringify({ "port:3000": true }));
	localStorage.setItem("ports.shownKinds", JSON.stringify(["dev", "other"]));
	const changes: RulesChange[] = [];
	mockIPC(
		(command, args) => {
			if (command === "get_rules") return view;
			if (command === "update_rules") {
				changes.push((args as { change: RulesChange }).change);
				return { rules: { ...view, revision: 1 }, snapshot: { revision: 1, servers: [], system: {} } };
			}
		},
		{ shouldMockEvents: true },
	);
	new RulesStore().load();
	await vi.waitFor(() => expect(localStorage.getItem("ports.hidden")).toBeNull());
	expect(changes).toEqual([
		{ kind: "import", hidden: ["port:5000"], protection: { "port:3000": true }, shownKinds: ["dev", "other"] },
	]);
	expect(localStorage.getItem("ports.protection")).toBeNull();
	expect(localStorage.getItem("ports.shownKinds")).toBeNull();

	new RulesStore().load();
	await vi.waitFor(() => expect(changes).toHaveLength(1));
});
