<script lang="ts">
	import ListFilterIcon from "@lucide/svelte/icons/list-filter";
	import { Button } from "$lib/components/ui/button";
	import * as DropdownMenu from "$lib/components/ui/dropdown-menu";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";
	import { preferences, serverKinds } from "$lib/ports/preferences.svelte";
	import type { ServerKind } from "$lib/ports/types";

	let { store }: { store: PortsStore } = $props();

	const count = (kind: ServerKind) => store.servers.filter((server) => server.kind === kind).length;
</script>

<DropdownMenu.Root>
	<DropdownMenu.Trigger>
		{#snippet child({ props })}
			<Button {...props} variant="ghost" size="icon-sm" aria-label="Choose which servers to show">
				<ListFilterIcon />
			</Button>
		{/snippet}
	</DropdownMenu.Trigger>
	<DropdownMenu.Content align="end" class="w-56">
		<DropdownMenu.Group>
			<DropdownMenu.GroupHeading>Show</DropdownMenu.GroupHeading>
			<DropdownMenu.CheckboxGroup
				bind:value={
					() => preferences.shownKinds.current, (kinds) => (preferences.shownKinds.current = kinds as ServerKind[])
				}
			>
				{#each serverKinds as kind (kind.value)}
					<DropdownMenu.CheckboxItem value={kind.value}>
						{kind.label}
						<span class="ml-auto font-mono text-xs text-muted-foreground tabular-nums">{count(kind.value)}</span>
					</DropdownMenu.CheckboxItem>
				{/each}
			</DropdownMenu.CheckboxGroup>
		</DropdownMenu.Group>
	</DropdownMenu.Content>
</DropdownMenu.Root>
