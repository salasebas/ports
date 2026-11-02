<script lang="ts">
	import { invoke } from "@tauri-apps/api/core";
	import { toast } from "svelte-sonner";
	import { Button } from "$lib/components/ui/button";
	import * as Dialog from "$lib/components/ui/dialog";
	import * as Field from "$lib/components/ui/field";
	import { Switch } from "$lib/components/ui/switch";
	import { Textarea } from "$lib/components/ui/textarea";

	let { open = $bindable(false) }: { open?: boolean } = $props();

	const id = $props.id();

	const choices = [
		{ key: "commands", label: "Commands", description: "With secrets and environment values removed." },
		{ key: "paths", label: "Folders", description: "Project and working folders." },
		{ key: "settings", label: "Settings", description: "App settings, and how many rules you have." },
	] as const;

	let include = $state({ commands: false, paths: false, settings: true });
	let contents = $state("");
	let error = $state<string | null>(null);
	let saving = $state(false);

	$effect(() => {
		if (!open) return;
		let current = true;
		contents = "";
		error = null;
		void invoke<string>("preview_diagnostics", { include: { ...include } })
			.then((preview) => {
				if (current) contents = preview;
			})
			.catch((reason: unknown) => {
				if (current) error = String(reason);
			});
		return () => {
			current = false;
		};
	});

	async function save() {
		saving = true;
		try {
			const path = await invoke<string | null>("save_diagnostics", { contents });
			if (path === null) return;
			toast.success("Diagnostics saved", { description: path });
			open = false;
		} catch (reason) {
			error = String(reason);
		} finally {
			saving = false;
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="sm:max-w-xl">
		<Dialog.Header>
			<Dialog.Title>Export diagnostics</Dialog.Title>
			<Dialog.Description>
				Review what goes in the file. Ports only writes it where you choose, and keeps no copy.
			</Dialog.Description>
		</Dialog.Header>
		<Field.Group class="gap-3">
			{#each choices as choice (choice.key)}
				<Field.Field orientation="horizontal">
					<Field.Content>
						<Field.Label for="{id}-{choice.key}">{choice.label}</Field.Label>
						<Field.Description>{choice.description}</Field.Description>
					</Field.Content>
					<Switch id="{id}-{choice.key}" bind:checked={include[choice.key]} />
				</Field.Field>
			{/each}
		</Field.Group>
		<Textarea readonly class="h-48 font-mono text-xs" aria-label="Preview of the file" value={contents} />
		{#if error}
			<Field.Error>{error}</Field.Error>
		{/if}
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (open = false)}>Cancel</Button>
			<Button disabled={!contents || saving} aria-busy={saving} onclick={save}>Save…</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
