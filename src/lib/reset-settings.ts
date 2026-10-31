import { invoke } from "@tauri-apps/api/core";
import { disable } from "@tauri-apps/plugin-autostart";
import { setMode } from "mode-watcher";
import { preferenceDefaults, preferences } from "./ports/preferences.svelte";
import { setRefreshInterval } from "./ports/ports-store.svelte";
import { rules } from "./rules.svelte";
import { settings, type Settings } from "./settings.svelte";

export const resetSections = {
	appearance: "Appearance",
	general: "General",
	servers: "Servers",
	cleanUp: "Clean up",
	memoryAlerts: "Memory alerts",
	lists: "Lists",
};

export type SettingsSection = keyof typeof resetSections;
export type ResetScope = SettingsSection | "all";

export async function resetSettings(scope: ResetScope): Promise<void> {
	const defaults = await invoke<Omit<Settings, "revision">>("get_settings_defaults");
	const local = structuredClone(preferenceDefaults);
	const sections: SettingsSection[] =
		scope === "all" ? ["cleanUp", "appearance", "servers", "memoryAlerts", "lists", "general"] : [scope];
	if (scope === "all") preferences.metric.current = local.metric;
	for (const section of sections) {
		switch (section) {
			case "appearance":
				setMode("system");
				preferences.keepHeaderVisible.current = local.keepHeaderVisible;
				break;
			case "general":
				await disable();
				await settings.setShortcut(defaults.shortcut);
				break;
			case "servers":
				await settings.setStopGrace(defaults.stopGraceSeconds);
				if (!(await setRefreshInterval(local.refreshInterval))) throw new Error("Could not reset the refresh interval");
				preferences.editor.current = local.editor;
				preferences.confirmActions.current = local.confirmActions;
				break;
			case "cleanUp":
				await rules.change({ kind: "autoCleanup", policy: null, paused: false });
				await settings.setIdleAfter(defaults.idleAfterMinutes);
				break;
			case "memoryAlerts":
				await settings.setAlerts(defaults.alerts);
				break;
			case "lists":
				await rules.change({ kind: "resetLists" });
				preferences.shownKinds.current = local.shownKinds;
				preferences.sort.current = local.sort;
				preferences.groupBy.current = local.groupBy;
				preferences.collapsedGroups.current = local.collapsedGroups;
				preferences.groupHeaders.current = local.groupHeaders;
				break;
			default: {
				const unexpected: never = section;
				throw new Error(`Unknown settings section: ${unexpected}`);
			}
		}
	}
}
