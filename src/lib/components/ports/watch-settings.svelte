<script lang="ts">
	import XIcon from "@lucide/svelte/icons/x";
	import { Button } from "$lib/components/ui/button";
	import * as Field from "$lib/components/ui/field";
	import { Input } from "$lib/components/ui/input";
	import { rules, watchEvents, watchLabel, type WatchEvent } from "$lib/rules.svelte";
	import WatchEvents from "./watch-events.svelte";

	let { id, disabled = false }: { id: string; disabled?: boolean } = $props();

	const watched = $derived(Object.keys(rules.current.rules.watches));
	let port = $state("");
	let error = $state<string | null>(null);

	async function save(key: string, events: WatchEvent[]) {
		try {
			await rules.change({ kind: "watch", key, events });
			error = null;
			return true;
		} catch (reason) {
			error = String(reason);
			return false;
		}
	}

	async function add(event: SubmitEvent) {
		event.preventDefault();
		const number = Number(port);
		if (!Number.isInteger(number) || number < 1 || number > 65_535) {
			error = "Use a whole port number from 1 to 65535";
			return;
		}
		const everyEvent = watchEvents.map(({ value }) => value);
		if (await save(`port:${number}`, everyEvent)) port = "";
	}
</script>

<Field.Field data-invalid={error ? true : undefined}>
	<Field.Content>
		<Field.Label for="{id}-watch-port">Notify me about a port</Field.Label>
		<Field.Description>
			Whatever runs on it, even with no window open. Add a project from its server's details. Nothing is noticed while
			Refresh is set to Never.
		</Field.Description>
	</Field.Content>
	<form class="flex items-center gap-2" onsubmit={add}>
		<Input id="{id}-watch-port" class="h-8 w-24" inputmode="numeric" placeholder="3000" {disabled} bind:value={port} />
		<Button type="submit" variant="secondary" size="sm" disabled={disabled || !port.trim()}>Watch</Button>
	</form>
	{#if error}
		<Field.Error>{error}</Field.Error>
	{/if}
</Field.Field>
{#each watched as key (key)}
	{@const label = watchLabel(key)}
	<Field.Separator />
	<Field.Field orientation="horizontal">
		<Field.Title class="min-w-0 truncate font-mono">{label}</Field.Title>
		<div class="ms-auto flex items-center gap-1">
			<WatchEvents watchKey={key} label="Notify me about {label}" {disabled} />
			<Button
				variant="ghost"
				size="icon-sm"
				aria-label="Stop watching {label}"
				{disabled}
				onclick={() => save(key, [])}
			>
				<XIcon />
			</Button>
		</div>
	</Field.Field>
{/each}
