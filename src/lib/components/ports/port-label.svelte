<script lang="ts">
	import { portColor } from "$lib/ports/format";
	import { cn } from "$lib/utils";

	let { port, class: className }: { port: number; class?: string } = $props();
</script>

<span class={cn("font-mono font-medium tracking-tight tabular-nums", className)} style:color={portColor(port)}
	><span class="opacity-60">:</span>{port}</span
>
