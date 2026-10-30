<script lang="ts">
	import "./layout.css";
	import { isTauri } from "@tauri-apps/api/core";
	import { listen } from "@tauri-apps/api/event";
	import { getCurrentWindow } from "@tauri-apps/api/window";
	import { onMount } from "svelte";
	import { ModeWatcher } from "mode-watcher";
	import { goto } from "$app/navigation";
	import { resolve } from "$app/paths";
	import { Toaster } from "$lib/components/ui/sonner";
	import * as Tooltip from "$lib/components/ui/tooltip";
	import { isMac } from "$lib/platform";
	import { rules } from "$lib/rules.svelte";
	import { openSettings, settings } from "$lib/settings.svelte";

	let { children } = $props();

	const OPEN_SETTINGS_EVENT = "ports://open-settings";
	const DETAILS_EVENT = "ports://details";

	onMount(() => {
		settings.load();
		rules.load();
		if (!isTauri() || getCurrentWindow().label !== "main") return;
		const listening = [
			listen(OPEN_SETTINGS_EVENT, () => goto(resolve("/settings"))),
			listen<string>(DETAILS_EVENT, ({ payload: id }) => goto(`${resolve("/")}?details=${encodeURIComponent(id)}`)),
		];
		return () => listening.forEach((unlisten) => void unlisten.then((stop) => stop()));
	});

	function onkeydown(event: KeyboardEvent) {
		if (event.key === "," && (isMac ? event.metaKey : event.ctrlKey) && !event.altKey && !event.shiftKey) {
			event.preventDefault();
			openSettings();
		}
	}
</script>

<svelte:window {onkeydown} />

<ModeWatcher />
<Toaster position="bottom-center" closeButton />
<Tooltip.Provider delayDuration={400}>
	{@render children()}
</Tooltip.Provider>
