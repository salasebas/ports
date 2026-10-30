<script lang="ts">
	import * as AlertDialog from "$lib/components/ui/alert-dialog";
	import { Checkbox } from "$lib/components/ui/checkbox";
	import { LastShown } from "$lib/hooks/last-shown.svelte";
	import { serverActivity } from "$lib/ports/format";
	import { cleanUpReason, type PortsStore } from "$lib/ports/ports-store.svelte";
	import { settings } from "$lib/settings.svelte";
	import PortLabel from "./port-label.svelte";

	let { store }: { store: PortsStore } = $props();

	const id = $props.id();

	const cleaning = new LastShown(() => store.cleaning);
	const servers = $derived(cleaning.current ?? []);
	const chosen = $derived(servers.filter((server) => store.cleanSelection.has(server.id)).length);
	const idleAfter = $derived(settings.current.idleAfterMinutes);
</script>

<AlertDialog.Root
	bind:open={
		() => store.cleaning !== null,
		(open) => {
			if (!open) store.cleaning = null;
		}
	}
>
	<AlertDialog.Content size="sm">
		<AlertDialog.Header>
			<AlertDialog.Title>Clean up servers</AlertDialog.Title>
			<AlertDialog.Description>
				Idle means {idleAfter} minutes without connections or CPU.
			</AlertDialog.Description>
		</AlertDialog.Header>
		<ul class="flex max-h-60 flex-col gap-1 overflow-y-auto text-sm">
			{#each servers as server (server.id)}
				<li class="flex items-center gap-3 rounded-md py-1">
					<Checkbox
						id="{id}-{server.id}"
						checked={store.cleanSelection.has(server.id)}
						onCheckedChange={(checked) => store.chooseForCleanUp(server, checked === true)}
					/>
					<label for="{id}-{server.id}" class="flex min-w-0 flex-1 items-center gap-3">
						<PortLabel port={server.port} class="w-14 shrink-0" />
						<span class="min-w-0 flex-1 truncate font-medium" title={server.folder ?? server.cwd}>{server.name}</span>
						<span class="shrink-0 text-xs {server.cwdDeleted ? 'text-warning' : 'text-muted-foreground'}">
							{server.cwdDeleted ? cleanUpReason(server) : serverActivity(server)}
						</span>
					</label>
				</li>
			{/each}
		</ul>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
			<AlertDialog.Action variant="destructive" disabled={chosen === 0} onclick={store.confirmCleanUp}>
				Stop {chosen}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
