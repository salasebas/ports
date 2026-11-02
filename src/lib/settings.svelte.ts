import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { goto } from "$app/navigation";
import { resolve } from "$app/paths";

export const SETTINGS_EVENT = "ports://settings";

export interface GrowthRule {
	windowMinutes: number;
	minGrowthBytes: number;
}

export interface AlertSettings {
	enabled: boolean;
	thresholdBytes: number | null;
	growth: GrowthRule;
}

export interface HealthSettings {
	enabled: boolean;
	everySeconds: number;
	timeoutSeconds: number;
}

export type TrayMetric = "off" | "servers" | "memory" | "cpu" | "serversAndMemory";

const MB = 1024 ** 2;
const GB = 1024 ** 3;

export const alertThresholds = [
	{ value: "off", label: "Off" },
	...[1, 2, 4, 8].map((gb) => ({ value: String(gb * GB), label: `Above ${gb} GB` })),
];

export const growthWindows = [5, 10].map((minutes) => ({ value: String(minutes), label: `${minutes} minutes` }));

export const growthMinimums = [
	{ value: String(50 * MB), label: "50 MB" },
	{ value: String(100 * MB), label: "100 MB" },
	{ value: String(200 * MB), label: "200 MB" },
	{ value: String(500 * MB), label: "500 MB" },
	{ value: String(GB), label: "1 GB" },
];

export const healthCadences = [5, 10, 30, 60].map((seconds) => ({
	value: String(seconds),
	label: seconds === 60 ? "Every minute" : `Every ${seconds} seconds`,
}));

export const healthTimeouts = [1, 3, 5, 10].map((seconds) => ({
	value: String(seconds),
	label: seconds === 1 ? "1 second" : `${seconds} seconds`,
}));

export const trayMetrics: { value: TrayMetric; label: string }[] = [
	{ value: "off", label: "Off" },
	{ value: "servers", label: "Servers" },
	{ value: "memory", label: "Memory" },
	{ value: "cpu", label: "CPU" },
	{ value: "serversAndMemory", label: "Servers and memory" },
];

export const stopGraces = [1, 3, 5, 10, 30, 60].map((seconds) => ({
	value: String(seconds),
	label: seconds === 60 ? "1 minute" : seconds === 1 ? "1 second" : `${seconds} seconds`,
}));

export const idleThresholds = [5, 10, 15, 30, 60, 120].map((minutes) => ({
	value: String(minutes),
	label: minutes < 60 ? `${minutes} minutes` : minutes === 60 ? "1 hour" : `${minutes / 60} hours`,
}));

export interface Settings {
	revision: number;
	shortcut: string | null;
	alerts: AlertSettings;
	health: HealthSettings;
	trayMetric: TrayMetric;
	stopGraceSeconds: number;
	idleAfterMinutes: number;
}

export class BackendSettings {
	current = $state.raw<Settings>({
		revision: -1,
		shortcut: null,
		alerts: {
			enabled: true,
			thresholdBytes: 2 * GB,
			growth: { windowMinutes: 10, minGrowthBytes: 50 * MB },
		},
		health: { enabled: true, everySeconds: 10, timeoutSeconds: 3 },
		trayMetric: "off",
		stopGraceSeconds: 3,
		idleAfterMinutes: 5,
	});
	#loaded = false;

	apply = (settings: Settings) => {
		if (settings.revision > this.current.revision) this.current = settings;
	};

	load = () => {
		if (this.#loaded || !isTauri()) return;
		this.#loaded = true;
		void (async () => {
			let stop: (() => void) | undefined;
			try {
				stop = await listen<Settings>(SETTINGS_EVENT, (event) => this.apply(event.payload));
				this.apply(await invoke<Settings>("get_settings"));
			} catch (error) {
				stop?.();
				this.#loaded = false;
				console.error(error);
			}
		})();
	};

	setShortcut = async (shortcut: string | null) => {
		this.apply(await invoke<Settings>("set_shortcut", { shortcut }));
	};

	setAlerts = async (alerts: Partial<AlertSettings>) => {
		this.apply(await invoke<Settings>("set_alerts", { alerts }));
	};

	setHealth = async (health: HealthSettings) => {
		this.apply(await invoke<Settings>("set_health", { health }));
	};

	setTrayMetric = async (metric: TrayMetric) => {
		this.apply(await invoke<Settings>("set_tray_metric", { metric }));
	};

	setStopGrace = async (seconds: number) => {
		this.apply(await invoke<Settings>("set_stop_grace", { seconds }));
	};

	setIdleAfter = async (minutes: number) => {
		this.apply(await invoke<Settings>("set_idle_after", { minutes }));
	};
}

export const settings = new BackendSettings();

export function openSettings() {
	if (isTauri()) void invoke("open_settings");
	else void goto(resolve("/settings"));
}
