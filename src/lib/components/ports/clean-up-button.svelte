<script lang="ts">
	import BrushCleaningIcon from "@lucide/svelte/icons/brush-cleaning";
	import { Button } from "$lib/components/ui/button";
	import * as Tooltip from "$lib/components/ui/tooltip";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";

	let { store }: { store: PortsStore } = $props();

	const count = $derived(store.idle.length);
	const label = $derived(
		count === 0 ? "No idle servers to clean up" : `Clean up ${count === 1 ? "1 idle server" : `${count} idle servers`}`,
	);
</script>

<Tooltip.Root>
	<Tooltip.Trigger>
		{#snippet child({ props })}
			<!-- Stays focusable while disabled so its tooltip still explains why. -->
			<Button
				{...props}
				variant="ghost"
				size="icon-sm"
				aria-label={label}
				aria-disabled={count === 0}
				class="aria-disabled:opacity-50"
				onclick={store.cleanUp}
			>
				<BrushCleaningIcon />
			</Button>
		{/snippet}
	</Tooltip.Trigger>
	<Tooltip.Content>{label}</Tooltip.Content>
</Tooltip.Root>
