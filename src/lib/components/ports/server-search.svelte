<script lang="ts">
	import SearchIcon from "@lucide/svelte/icons/search";
	import XIcon from "@lucide/svelte/icons/x";
	import * as InputGroup from "$lib/components/ui/input-group";

	let {
		value = $bindable(""),
		ref = $bindable(null),
		class: className,
	}: { value?: string; ref?: HTMLInputElement | null; class?: string } = $props();
</script>

<InputGroup.Root class={className}>
	<InputGroup.Input
		bind:ref
		bind:value
		placeholder="Search port, PID, project, path…"
		aria-label="Search servers by port, PID, name, command or folder"
	/>
	<InputGroup.Addon>
		<SearchIcon />
	</InputGroup.Addon>
	{#if value}
		<InputGroup.Addon align="inline-end">
			<InputGroup.Button size="icon-xs" aria-label="Clear search" onclick={() => (value = "")}>
				<XIcon />
			</InputGroup.Button>
		</InputGroup.Addon>
	{/if}
</InputGroup.Root>
