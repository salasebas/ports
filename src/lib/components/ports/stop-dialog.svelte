<script lang="ts">
	import * as AlertDialog from "$lib/components/ui/alert-dialog";
	import { Checkbox } from "$lib/components/ui/checkbox";
	import * as Field from "$lib/components/ui/field";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";
	import { preferences } from "$lib/ports/preferences.svelte";
	import type { Server } from "$lib/ports/types";

	let { store }: { store: PortsStore } = $props();

	const id = $props.id();
	let dontAskAgain = $state(false);

	// Keeps the text in place while the dialog animates out.
	let last: Server | null = null;
	const server = $derived.by(() => (last = store.pendingStop ?? last));

	function confirm() {
		if (!store.pendingStop) return;
		if (dontAskAgain) preferences.confirmStop.current = false;
		dontAskAgain = false;
		store.stop(store.pendingStop);
	}
</script>

<AlertDialog.Root
	bind:open={
		() => store.pendingStop !== null,
		(open) => {
			if (open) return;
			store.pendingStop = null;
			dontAskAgain = false;
		}
	}
>
	<AlertDialog.Content size="sm">
		{#if server}
			<AlertDialog.Header>
				<AlertDialog.Title>Stop {server.name}?</AlertDialog.Title>
				<AlertDialog.Description>
					This ends {server.process} (PID {server.pid}) and frees :{server.port}.
					{#if server.protected}It is marked as protected.{/if}
				</AlertDialog.Description>
			</AlertDialog.Header>
			<Field.Field orientation="horizontal" class="justify-center">
				<Checkbox id="{id}-dont-ask" bind:checked={dontAskAgain} />
				<Field.Label for="{id}-dont-ask" class="font-normal text-muted-foreground">Don't ask again</Field.Label>
			</Field.Field>
			<AlertDialog.Footer>
				<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
				<AlertDialog.Action variant="destructive" onclick={confirm}>Stop</AlertDialog.Action>
			</AlertDialog.Footer>
		{/if}
	</AlertDialog.Content>
</AlertDialog.Root>
