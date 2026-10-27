<script lang="ts">
	import { cn } from "$lib/utils";

	let { values, class: className }: { values: number[]; class?: string } = $props();

	const STEP = 3;
</script>

<svg
	viewBox="0 0 {values.length * STEP} 100"
	preserveAspectRatio="none"
	class={cn("text-muted-foreground", className)}
	aria-hidden="true"
>
	{#each values as value, i (i)}
		<rect
			x={i * STEP}
			y={100 - Math.max(value, 3)}
			width={STEP - 1}
			height={Math.max(value, 3)}
			class={cn("fill-current", i === values.length - 1 ? "text-foreground" : "opacity-50")}
		/>
	{/each}
</svg>
