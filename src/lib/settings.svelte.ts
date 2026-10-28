import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { goto } from "$app/navigation";
import { resolve } from "$app/paths";

/** Must match `SETTINGS_EVENT` in `src-tauri/src/settings.rs`. */
export const SETTINGS_EVENT = "ports://settings";

/** Mirrors `AlertSettings` in `src-tauri/src/ports/alerts.rs`. */
export interface AlertSettings {
	/** Notify when a server's memory keeps growing. */
	enabled: boolean;
	/** Also notify when a server uses more than this; null turns it off. */
	thresholdBytes: number | null;
}

const GB = 1024 ** 3;

/** Choices for `AlertSettings.thresholdBytes`, as select values. */
export const alertThresholds = [
	{ value: "off", label: "Off" },
	...[1, 2, 4, 8].map((gb) => ({ value: String(gb * GB), label: `Above ${gb} GB` })),
];

/** Choices for `Settings.stopGraceSeconds`, within the backend's 1 to 60 seconds. */
export const stopGraces = [1, 3, 5, 10, 30, 60].map((seconds) => ({
	value: String(seconds),
	label: seconds === 60 ? "1 minute" : seconds === 1 ? "1 second" : `${seconds} seconds`,
}));

/** Mirrors `Settings` in `src-tauri/src/settings.rs`. */
export interface Settings {
	revision: number;
	/** The shortcut that shows the popover, e.g. `alt+super+KeyP`; null when there's none. */
	shortcut: string | null;
	alerts: AlertSettings;
	/** How long Stop waits for a server to exit before it kills it. */
	stopGraceSeconds: number;
}

/**
 * Settings the backend keeps, since it needs them while every window is
 * hidden: the shortcut, the memory alerts and the shutdown grace period. Each window loads them once
 * and follows the changes made in any window.
 */
export class BackendSettings {
	current = $state.raw<Settings>({
		revision: -1,
		shortcut: null,
		alerts: { enabled: true, thresholdBytes: 2 * GB },
		stopGraceSeconds: 3,
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

	/** Rejects with the reason when the shortcut can't be used; the previous one stays. */
	setShortcut = async (shortcut: string | null) => {
		this.apply(await invoke<Settings>("set_shortcut", { shortcut }));
	};

	setAlerts = async (alerts: Partial<AlertSettings>) => {
		this.apply(await invoke<Settings>("set_alerts", { alerts }));
	};

	setStopGrace = async (seconds: number) => {
		this.apply(await invoke<Settings>("set_stop_grace", { seconds }));
	};
}

export const settings = new BackendSettings();

/** Shows the settings page of the main window, from any window. */
export function openSettings() {
	if (isTauri()) void invoke("open_settings");
	else void goto(resolve("/settings"));
}
