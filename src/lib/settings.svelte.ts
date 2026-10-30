import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { goto } from "$app/navigation";
import { resolve } from "$app/paths";

export const SETTINGS_EVENT = "ports://settings";

export interface AlertSettings {
	enabled: boolean;
	thresholdBytes: number | null;
}

const GB = 1024 ** 3;

export const alertThresholds = [
	{ value: "off", label: "Off" },
	...[1, 2, 4, 8].map((gb) => ({ value: String(gb * GB), label: `Above ${gb} GB` })),
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
	stopGraceSeconds: number;
	idleAfterMinutes: number;
}

export class BackendSettings {
	current = $state.raw<Settings>({
		revision: -1,
		shortcut: null,
		alerts: { enabled: true, thresholdBytes: 2 * GB },
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
