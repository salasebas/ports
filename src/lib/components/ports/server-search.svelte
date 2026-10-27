<script lang="ts">
	import SearchIcon from "@lucide/svelte/icons/search";
	import XIcon from "@lucide/svelte/icons/x";
	import * as InputGroup from "$lib/components/ui/input-group";

	let { value = $bindable(""), class: className }: { value?: string; class?: string } = $props();
</script>

<InputGroup.Root class={className}>
	<InputGroup.Input bind:value placeholder="Search port, project, branch…" aria-label="Search servers" />
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
