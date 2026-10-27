<script lang="ts">
	import SearchXIcon from "@lucide/svelte/icons/search-x";
	import * as Empty from "$lib/components/ui/empty";
	import { Button } from "$lib/components/ui/button";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";

	let { store, class: className }: { store: PortsStore; class?: string } = $props();

	const searching = $derived(store.query !== "");
</script>

<Empty.Root class={className}>
	<Empty.Header>
		<Empty.Media variant="icon">
			<SearchXIcon />
		</Empty.Media>
		<Empty.Title>{searching ? "No matching servers" : "Nothing is listening"}</Empty.Title>
		<Empty.Description>
			{searching ? "Try another port, project or branch." : "Start a dev server and it shows up here."}
		</Empty.Description>
	</Empty.Header>
	{#if searching}
		<Empty.Content>
			<Button variant="secondary" size="sm" onclick={() => (store.query = "")}>Clear search</Button>
		</Empty.Content>
	{/if}
</Empty.Root>
