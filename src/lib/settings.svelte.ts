import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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

/** Mirrors `Settings` in `src-tauri/src/settings.rs`. */
export interface Settings {
	/** The shortcut that shows the popover, e.g. `alt+super+KeyP`; null when there's none. */
	shortcut: string | null;
	alerts: AlertSettings;
}

/**
 * Settings the backend keeps, since it needs them while every window is
 * hidden: the shortcut and the memory alerts. Each window loads them once
 * and follows the changes made in any window.
 */
class BackendSettings {
	current = $state.raw<Settings>({ shortcut: null, alerts: { enabled: true, thresholdBytes: 2 * GB } });
	#loaded = false;

	load = () => {
		if (this.#loaded || !isTauri()) return;
		this.#loaded = true;
		void listen<Settings>(SETTINGS_EVENT, (event) => (this.current = event.payload));
		invoke<Settings>("get_settings").then(
			(settings) => (this.current = settings),
			(error) => console.error(error),
		);
	};

	/** Rejects with the reason when the shortcut can't be used; the previous one stays. */
	setShortcut = async (shortcut: string | null) => {
		this.current = await invoke<Settings>("set_shortcut", { shortcut });
	};

	setAlerts = async (alerts: AlertSettings) => {
		this.current = await invoke<Settings>("set_alerts", { alerts });
	};
}

export const settings = new BackendSettings();

/** Brings up the settings window, from any window. */
export function openSettings() {
	if (isTauri()) void invoke("open_settings");
}
