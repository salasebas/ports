<script lang="ts">
	import SearchXIcon from "@lucide/svelte/icons/search-x";
	import * as Empty from "$lib/components/ui/empty";
	import { Button } from "$lib/components/ui/button";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";
	import { preferences, serverKinds } from "$lib/ports/preferences.svelte";

	let { store, class: className }: { store: PortsStore; class?: string } = $props();

	const searching = $derived(store.query !== "");
	const hidden = $derived(store.servers.length - store.shown.length);
</script>

<Empty.Root class={className}>
	<Empty.Header>
		<Empty.Media variant="icon">
			<SearchXIcon />
		</Empty.Media>
		{#if searching}
			<Empty.Title>No matching servers</Empty.Title>
			<Empty.Description>Try another port, project or branch.</Empty.Description>
		{:else if hidden}
			<Empty.Title>No servers to show</Empty.Title>
			<Empty.Description
				>{hidden === 1 ? "1 server is" : `${hidden} servers are`} hidden by the filter.</Empty.Description
			>
		{:else}
			<Empty.Title>Nothing is listening</Empty.Title>
			<Empty.Description>Start a dev server and it shows up here.</Empty.Description>
		{/if}
	</Empty.Header>
	{#if searching}
		<Empty.Content>
			<Button variant="secondary" size="sm" onclick={() => (store.query = "")}>Clear search</Button>
		</Empty.Content>
	{:else if hidden}
		<Empty.Content>
			<Button
				variant="secondary"
				size="sm"
				onclick={() => (preferences.shownKinds.current = serverKinds.map((kind) => kind.value))}
			>
				Show all
			</Button>
		</Empty.Content>
	{/if}
</Empty.Root>
