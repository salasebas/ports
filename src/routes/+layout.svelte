<script lang="ts">
	import "./layout.css";
	import { ModeWatcher } from "mode-watcher";
	import { Toaster } from "$lib/components/ui/sonner";
	import * as Tooltip from "$lib/components/ui/tooltip";

	let { children } = $props();
</script>

<ModeWatcher />
<Toaster position="bottom-right" />
<Tooltip.Provider delayDuration={400}>
	{@render children()}
</Tooltip.Provider>
