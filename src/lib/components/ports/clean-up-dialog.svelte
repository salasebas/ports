<script lang="ts">
	import * as AlertDialog from "$lib/components/ui/alert-dialog";
	import { LastShown } from "$lib/hooks/last-shown.svelte";
	import { serverActivity } from "$lib/ports/format";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";
	import PortLabel from "./port-label.svelte";

	let { store }: { store: PortsStore } = $props();

	const cleaning = new LastShown(() => store.cleaning);
	const servers = $derived(cleaning.current ?? []);
	const count = $derived(servers.length === 1 ? "1 idle server" : `${servers.length} idle servers`);
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
			<AlertDialog.Title>Stop {count}?</AlertDialog.Title>
			<AlertDialog.Description>Protected servers keep running.</AlertDialog.Description>
		</AlertDialog.Header>
		<ul class="flex max-h-60 flex-col gap-2 overflow-y-auto text-sm">
			{#each servers as server (server.id)}
				<li class="flex items-center gap-3">
					<PortLabel port={server.port} class="w-14 shrink-0" />
					<span class="min-w-0 flex-1 truncate font-medium">{server.name}</span>
					<span class="shrink-0 text-xs text-muted-foreground">{serverActivity(server)}</span>
				</li>
			{/each}
		</ul>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
			<AlertDialog.Action variant="destructive" onclick={store.confirmCleanUp}>
				Stop {servers.length}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
