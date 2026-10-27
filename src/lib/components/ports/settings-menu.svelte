<script lang="ts">
	import type { Component } from "svelte";
	import MonitorIcon from "@lucide/svelte/icons/monitor";
	import MoonIcon from "@lucide/svelte/icons/moon";
	import SettingsIcon from "@lucide/svelte/icons/settings";
	import SunIcon from "@lucide/svelte/icons/sun";
	import { setMode, userPrefersMode } from "mode-watcher";
	import { Button } from "$lib/components/ui/button";
	import * as Popover from "$lib/components/ui/popover";
	import * as ToggleGroup from "$lib/components/ui/toggle-group";

	type Mode = "light" | "dark" | "system";

	let { side = "bottom" }: { side?: "top" | "bottom" } = $props();

	const modes: { value: Mode; label: string; icon: Component }[] = [
		{ value: "light", label: "Light", icon: SunIcon },
		{ value: "dark", label: "Dark", icon: MoonIcon },
		{ value: "system", label: "System", icon: MonitorIcon },
	];
</script>

<Popover.Root>
	<Popover.Trigger>
		{#snippet child({ props })}
			<Button {...props} variant="ghost" size="icon-sm" aria-label="Settings">
				<SettingsIcon />
			</Button>
		{/snippet}
	</Popover.Trigger>
	<Popover.Content {side} align="end" class="w-72">
		<Popover.Header>
			<Popover.Title>Appearance</Popover.Title>
			<Popover.Description>Choose a theme or follow your system.</Popover.Description>
		</Popover.Header>
		<ToggleGroup.Root
			type="single"
			variant="outline"
			size="sm"
			class="w-full"
			value={userPrefersMode.current}
			onValueChange={(value) => {
				if (value) setMode(value as Mode);
			}}
		>
			{#each modes as mode (mode.value)}
				<ToggleGroup.Item value={mode.value} class="flex-1" aria-label={mode.label}>
					<mode.icon data-icon="inline-start" />
					{mode.label}
				</ToggleGroup.Item>
			{/each}
		</ToggleGroup.Root>
	</Popover.Content>
</Popover.Root>
