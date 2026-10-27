<script lang="ts">
	import type { Component } from "svelte";
	import MonitorIcon from "@lucide/svelte/icons/monitor";
	import MoonIcon from "@lucide/svelte/icons/moon";
	import SettingsIcon from "@lucide/svelte/icons/settings";
	import SunIcon from "@lucide/svelte/icons/sun";
	import { isTauri } from "@tauri-apps/api/core";
	import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
	import { setMode, userPrefersMode } from "mode-watcher";
	import { toast } from "svelte-sonner";
	import { Button } from "$lib/components/ui/button";
	import * as Field from "$lib/components/ui/field";
	import * as Kbd from "$lib/components/ui/kbd";
	import * as NativeSelect from "$lib/components/ui/native-select";
	import * as Popover from "$lib/components/ui/popover";
	import { Switch } from "$lib/components/ui/switch";
	import * as ToggleGroup from "$lib/components/ui/toggle-group";
	import { popoverShortcut } from "$lib/platform";
	import { setRefreshInterval } from "$lib/ports/ports-store.svelte";
	import { editors, preferences, refreshIntervals } from "$lib/ports/preferences.svelte";

	type Mode = "light" | "dark" | "system";

	let { side = "bottom" }: { side?: "top" | "bottom" } = $props();

	const id = $props.id();
	const desktop = isTauri();

	/** Owned by the OS, so it's read on every open instead of stored. */
	let openAtLogin = $state(false);

	async function readOpenAtLogin() {
		if (desktop) openAtLogin = await isEnabled();
	}

	async function setOpenAtLogin(value: boolean) {
		try {
			await (value ? enable() : disable());
			openAtLogin = value;
		} catch (error) {
			toast.error(String(error));
		}
	}

	const modes: { value: Mode; label: string; icon: Component }[] = [
		{ value: "light", label: "Light", icon: SunIcon },
		{ value: "dark", label: "Dark", icon: MoonIcon },
		{ value: "system", label: "System", icon: MonitorIcon },
	];
</script>

<Popover.Root onOpenChange={(open) => open && readOpenAtLogin()}>
	<Popover.Trigger>
		{#snippet child({ props })}
			<Button {...props} variant="ghost" size="icon-sm" aria-label="Settings">
				<SettingsIcon />
			</Button>
		{/snippet}
	</Popover.Trigger>
	<Popover.Content {side} align="end" class="w-72">
		<Field.Group class="gap-4">
			<Field.Field>
				<Field.Title id="{id}-appearance">Appearance</Field.Title>
				<ToggleGroup.Root
					type="single"
					variant="outline"
					size="sm"
					class="w-full"
					aria-labelledby="{id}-appearance"
					bind:value={
						() => userPrefersMode.current,
						(value) => {
							if (value) setMode(value as Mode);
						}
					}
				>
					{#each modes as mode (mode.value)}
						<ToggleGroup.Item value={mode.value} class="flex-1" aria-label={mode.label}>
							<mode.icon data-icon="inline-start" />
							{mode.label}
						</ToggleGroup.Item>
					{/each}
				</ToggleGroup.Root>
			</Field.Field>
			<Field.Separator />
			<Field.Field orientation="horizontal">
				<Field.Label for="{id}-editor">Editor</Field.Label>
				<NativeSelect.Root id="{id}-editor" size="sm" class="w-32" bind:value={preferences.editor.current}>
					{#each editors as editor (editor.value)}
						<NativeSelect.Option value={editor.value}>{editor.label}</NativeSelect.Option>
					{/each}
				</NativeSelect.Root>
			</Field.Field>
			<Field.Field orientation="horizontal">
				<Field.Label for="{id}-refresh">Refresh</Field.Label>
				<NativeSelect.Root
					id="{id}-refresh"
					size="sm"
					class="w-32"
					bind:value={() => String(preferences.refreshInterval.current), (value) => setRefreshInterval(Number(value))}
				>
					{#each refreshIntervals as interval (interval.value)}
						<NativeSelect.Option value={String(interval.value)}>{interval.label}</NativeSelect.Option>
					{/each}
				</NativeSelect.Root>
			</Field.Field>
			<Field.Field orientation="horizontal">
				<Field.Content>
					<Field.Label for="{id}-confirm">Confirm actions</Field.Label>
					<Field.Description>Stop and restart.</Field.Description>
				</Field.Content>
				<Switch id="{id}-confirm" bind:checked={preferences.confirmActions.current} />
			</Field.Field>
			<Field.Field orientation="horizontal">
				<Field.Label for="{id}-open-at-login">Open at login</Field.Label>
				<Switch id="{id}-open-at-login" disabled={!desktop} bind:checked={() => openAtLogin, setOpenAtLogin} />
			</Field.Field>
			<Field.Separator />
			<Field.Field orientation="horizontal">
				<Field.Content>
					<Field.Title>Show Ports</Field.Title>
					<Field.Description>From anywhere.</Field.Description>
				</Field.Content>
				<Kbd.Group>
					{#each popoverShortcut as key (key)}
						<Kbd.Root>{key}</Kbd.Root>
					{/each}
				</Kbd.Group>
			</Field.Field>
		</Field.Group>
	</Popover.Content>
</Popover.Root>
