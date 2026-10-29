<script lang="ts">
	import XIcon from "@lucide/svelte/icons/x";
	import { Button } from "$lib/components/ui/button";
	import * as Kbd from "$lib/components/ui/kbd";
	import { shortcutFor, shortcutKeys } from "$lib/platform";
	import { settings } from "$lib/settings.svelte";

	let { id, onerror }: { id?: string; onerror: (error: string | null) => void } = $props();

	let recording = $state(false);

	const keys = $derived(settings.current.shortcut ? shortcutKeys(settings.current.shortcut) : null);

	async function save(shortcut: string | null) {
		try {
			await settings.setShortcut(shortcut);
			onerror(null);
		} catch (error) {
			onerror(String(error));
		}
	}

	function onkeydown(event: KeyboardEvent) {
		if (!recording) return;
		const plain = !event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey;
		if (plain && event.key === "Tab") {
			recording = false;
			return;
		}
		event.preventDefault();
		event.stopPropagation();
		if (plain && event.key === "Escape") {
			recording = false;
			return;
		}
		const shortcut = shortcutFor(event);
		if (!shortcut) return;
		recording = false;
		void save(shortcut);
	}
</script>

<div class="flex items-center gap-1">
	<Button
		{id}
		variant="outline"
		size="sm"
		class="min-w-24"
		aria-pressed={recording}
		aria-label={recording
			? "Press the new shortcut, or Escape to cancel"
			: `Change the shortcut, now ${keys ? keys.join(" ") : "none"}`}
		onclick={() => {
			recording = !recording;
			onerror(null);
		}}
		onblur={() => (recording = false)}
		{onkeydown}
	>
		{#if recording}
			<span class="text-muted-foreground">Press keys…</span>
		{:else if keys}
			<Kbd.Group>
				{#each keys as key, i (i)}
					<Kbd.Root>{key}</Kbd.Root>
				{/each}
			</Kbd.Group>
		{:else}
			<span class="text-muted-foreground">None</span>
		{/if}
	</Button>
	{#if settings.current.shortcut && !recording}
		<Button variant="ghost" size="icon-sm" aria-label="Remove the shortcut" onclick={() => save(null)}>
			<XIcon />
		</Button>
	{/if}
</div>
