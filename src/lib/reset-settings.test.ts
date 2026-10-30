import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it, vi } from "vitest";
import { preferences } from "./ports/preferences.svelte";
import { resetSettings } from "./reset-settings";

vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("@tauri-apps/plugin-autostart", () => ({ disable: vi.fn() }));
vi.mock("mode-watcher", () => ({ setMode: vi.fn() }));

const defaults = {
	shortcut: "Control+Alt+KeyP",
	alerts: { enabled: true, thresholdBytes: 2 * 1024 ** 3 },
	stopGraceSeconds: 3,
	idleAfterMinutes: 5,
};

afterEach(() => {
	clearMocks();
	vi.clearAllMocks();
	localStorage.clear();
});

it("resets Servers without changing list choices or server customizations", async () => {
	const calls: [string, unknown][] = [];
	mockIPC((command, args) => {
		calls.push([command, args]);
		return command === "get_settings_defaults" ? defaults : { ...defaults, revision: 100 };
	});
	preferences.editor.current = "zed";
	preferences.refreshInterval.current = 0;
	preferences.confirmActions.current = false;
	preferences.groupBy.current = "repository";
	preferences.shownKinds.current = ["other"];
	preferences.pinned.current = ["port:3000"];
	preferences.names.current = { 3000: "Store" };

	await resetSettings("servers");

	expect(preferences.editor.current).toBe("cursor");
	expect(preferences.refreshInterval.current).toBe(2000);
	expect(preferences.confirmActions.current).toBe(true);
	expect(preferences.groupBy.current).toBe("repository");
	expect(preferences.shownKinds.current).toEqual(["other"]);
	expect(preferences.pinned.current).toEqual(["port:3000"]);
	expect(preferences.names.current).toEqual({ 3000: "Store" });
	expect(calls).toContainEqual(["set_stop_grace", { seconds: 3 }]);
	expect(calls).toContainEqual(["set_refresh_interval", { ms: 2000 }]);
	expect(calls.some(([command]) => command === "update_rules" || command === "set_alerts")).toBe(false);
});

it("does not lower the idle threshold if automatic clean up could not be disabled", async () => {
	const commands: string[] = [];
	mockIPC((command) => {
		commands.push(command);
		if (command === "get_settings_defaults") return defaults;
		if (command === "update_rules") throw new Error("disk full");
		return { ...defaults, revision: 101 };
	});
	await expect(resetSettings("cleanUp")).rejects.toThrow("disk full");
	expect(commands).not.toContain("set_idle_after");
});
