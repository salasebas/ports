<script lang="ts">
	import "./layout.css";
	import { onMount } from "svelte";
	import { ModeWatcher } from "mode-watcher";
	import { Toaster } from "$lib/components/ui/sonner";
	import * as Tooltip from "$lib/components/ui/tooltip";
	import { settings } from "$lib/settings.svelte";

	let { children } = $props();

	onMount(settings.load);
</script>

<ModeWatcher />
<Toaster position="bottom-right" />
<Tooltip.Provider delayDuration={400}>
	{@render children()}
</Tooltip.Provider>
