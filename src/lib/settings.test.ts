import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it, vi } from "vitest";
import { BackendSettings, SETTINGS_EVENT, type Settings } from "./settings.svelte";

vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
const value = (revision: number): Settings => ({
	revision,
	shortcut: `shortcut-${revision}`,
	alerts: { enabled: true, thresholdBytes: null },
	stopGraceSeconds: 3,
	idleAfterMinutes: 5,
});
afterEach(() => {
	clearMocks();
	vi.unstubAllGlobals();
});

// Initial reads and IPC replies are allowed to arrive after newer window events.
it("subscribes before reading and keeps the newer settings event", async () => {
	vi.stubGlobal("isTauri", true);
	const initial = Promise.withResolvers<Settings>();
	const commands: string[] = [];
	mockIPC(
		(command) => {
			commands.push(command);
			return command === "get_settings" ? initial.promise : undefined;
		},
		{ shouldMockEvents: true },
	);
	const settings = new BackendSettings();
	settings.load();
	await vi.waitFor(() => expect(commands).toContain("get_settings"));
	expect(commands.indexOf("plugin:event|listen")).toBeLessThan(commands.indexOf("get_settings"));
	await emit(SETTINGS_EVENT, value(2));
	initial.resolve(value(1));
	await vi.waitFor(() => expect(settings.current.revision).toBe(2));
	await Promise.resolve();
	expect(settings.current).toEqual(value(2));
});

it("keeps settings ordered across concurrent setters and sends only the changed alert field", async () => {
	const first = Promise.withResolvers<Settings>();
	const second = Promise.withResolvers<Settings>();
	const sent: unknown[] = [];
	mockIPC((command, args) => {
		sent.push(args);
		return command === "set_shortcut" ? first.promise : second.promise;
	});
	const settings = new BackendSettings();
	const shortcut = settings.setShortcut("Control+Alt+KeyP");
	const alerts = settings.setAlerts({ enabled: false });
	second.resolve(value(2));
	await alerts;
	first.resolve(value(1));
	await shortcut;
	expect(settings.current).toEqual(value(2));
	expect(sent).toContainEqual({ alerts: { enabled: false } });
});

it("does not report a failed save as an applied change", async () => {
	mockIPC(() => {
		throw new Error("disk full");
	});
	const settings = new BackendSettings();
	settings.apply(value(1));
	await expect(settings.setAlerts({ enabled: false })).rejects.toThrow("disk full");
	expect(settings.current).toEqual(value(1));
});
