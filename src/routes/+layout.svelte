<script lang="ts">
	import "./layout.css";
	import { onMount } from "svelte";
	import { ModeWatcher } from "mode-watcher";
	import { Toaster } from "$lib/components/ui/sonner";
	import * as Tooltip from "$lib/components/ui/tooltip";
	import { isMac } from "$lib/platform";
	import { openSettings, settings } from "$lib/settings.svelte";

	let { children } = $props();

	onMount(settings.load);

	// ⌘, opens Settings from every window, as in other Mac apps.
	function onkeydown(event: KeyboardEvent) {
		if (event.key === "," && (isMac ? event.metaKey : event.ctrlKey) && !event.altKey && !event.shiftKey) {
			event.preventDefault();
			openSettings();
		}
	}
</script>

<svelte:window {onkeydown} />

<ModeWatcher />
<Toaster position="bottom-right" />
<Tooltip.Provider delayDuration={400}>
	{@render children()}
</Tooltip.Provider>
