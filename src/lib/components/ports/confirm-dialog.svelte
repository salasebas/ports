<script lang="ts">
	import * as AlertDialog from "$lib/components/ui/alert-dialog";
	import { Checkbox } from "$lib/components/ui/checkbox";
	import * as Field from "$lib/components/ui/field";
	import { serverProcessId } from "$lib/ports/format";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";
	import { preferences } from "$lib/ports/preferences.svelte";

	let { store }: { store: PortsStore } = $props();

	const id = $props.id();
	let dontAskAgain = $state(false);

	// Keeps the text in place while the dialog animates out.
	let last: PortsStore["pending"] = null;
	const pending = $derived.by(() => (last = store.pending ?? last));

	function confirm() {
		if (dontAskAgain) preferences.confirmActions.current = false;
		dontAskAgain = false;
		store.confirm();
	}
</script>

<AlertDialog.Root
	bind:open={
		() => store.pending !== null,
		(open) => {
			if (open) return;
			store.pending = null;
			dontAskAgain = false;
		}
	}
>
	<AlertDialog.Content size="sm">
		{#if pending}
			{@const { action, server } = pending}
			<AlertDialog.Header>
				<AlertDialog.Title>{action === "stop" ? "Stop" : "Restart"} {server.name}?</AlertDialog.Title>
				<AlertDialog.Description>
					{#if action === "stop"}
						This ends {server.process} ({serverProcessId(server)}) and frees :{server.port}.
						{#if server.protected}It is marked as protected.{/if}
					{:else}
						This stops {server.process} ({serverProcessId(server)}) and starts it again on :{server.port}.
					{/if}
				</AlertDialog.Description>
			</AlertDialog.Header>
			<Field.Field orientation="horizontal" class="justify-center">
				<Checkbox id="{id}-dont-ask" bind:checked={dontAskAgain} />
				<Field.Label for="{id}-dont-ask" class="font-normal text-muted-foreground">Don't ask again</Field.Label>
			</Field.Field>
			<AlertDialog.Footer>
				<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
				<AlertDialog.Action variant={action === "stop" ? "destructive" : "default"} onclick={confirm}>
					{action === "stop" ? "Stop" : "Restart"}
				</AlertDialog.Action>
			</AlertDialog.Footer>
		{/if}
	</AlertDialog.Content>
</AlertDialog.Root>
