<script lang="ts">
	import { Button } from "$lib/components/ui/button";
	import * as Dialog from "$lib/components/ui/dialog";
	import { Input } from "$lib/components/ui/input";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";
	import { preferences } from "$lib/ports/preferences.svelte";

	let { store }: { store: PortsStore } = $props();

	const id = $props.id();

	// Keeps the text in place while the dialog animates out.
	let last: PortsStore["renaming"] = null;
	const server = $derived.by(() => (last = store.renaming ?? last));
	const detected = $derived(server ? store.detectedName(server.port) : "");
	// Follows `store.renaming`, not `server`, so reopening the dialog drops cancelled text.
	let name = $derived(store.renaming ? (preferences.names.current[store.renaming.port] ?? "") : "");

	function save(event: SubmitEvent) {
		event.preventDefault();
		if (server) store.rename(server.port, name);
		store.renaming = null;
	}
</script>

<Dialog.Root
	bind:open={
		() => store.renaming !== null,
		(open) => {
			if (!open) store.renaming = null;
		}
	}
>
	<Dialog.Content class="sm:max-w-sm">
		{#if server}
			<form class="contents" onsubmit={save}>
				<Dialog.Header>
					<Dialog.Title>Rename :{server.port}</Dialog.Title>
					<Dialog.Description>Both lists show this name. Leave it blank for “{detected}”.</Dialog.Description>
				</Dialog.Header>
				<Input id="{id}-name" aria-label="Name" placeholder={detected} bind:value={name} />
				<Dialog.Footer>
					<Button type="button" variant="secondary" onclick={() => (store.renaming = null)}>Cancel</Button>
					<Button type="submit">Save</Button>
				</Dialog.Footer>
			</form>
		{/if}
	</Dialog.Content>
</Dialog.Root>
