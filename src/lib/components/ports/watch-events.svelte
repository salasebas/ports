<script lang="ts">
	import { toast } from "svelte-sonner";
	import * as ToggleGroup from "$lib/components/ui/toggle-group";
	import { rules, watchEvents, type WatchEvent } from "$lib/rules.svelte";

	let { watchKey, label, disabled = false }: { watchKey: string; label: string; disabled?: boolean } = $props();

	const events = $derived(rules.current.rules.watches[watchKey] ?? []);

	async function save(next: string[]) {
		try {
			await rules.change({ kind: "watch", key: watchKey, events: next as WatchEvent[] });
		} catch (error) {
			toast.error(String(error));
		}
	}
</script>

<ToggleGroup.Root
	type="multiple"
	variant="outline"
	size="sm"
	aria-label={label}
	{disabled}
	bind:value={() => events, save}
>
	{#each watchEvents as event (event.value)}
		<ToggleGroup.Item value={event.value} class="flex-1">{event.label}</ToggleGroup.Item>
	{/each}
</ToggleGroup.Root>
