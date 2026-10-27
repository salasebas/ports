<script lang="ts">
	import type { Component } from "svelte";
	import MonitorIcon from "@lucide/svelte/icons/monitor";
	import MoonIcon from "@lucide/svelte/icons/moon";
	import SettingsIcon from "@lucide/svelte/icons/settings";
	import SunIcon from "@lucide/svelte/icons/sun";
	import XIcon from "@lucide/svelte/icons/x";
	import { isTauri } from "@tauri-apps/api/core";
	import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
	import { setMode, userPrefersMode } from "mode-watcher";
	import { toast } from "svelte-sonner";
	import { Badge } from "$lib/components/ui/badge";
	import { Button } from "$lib/components/ui/button";
	import * as Field from "$lib/components/ui/field";
	import * as NativeSelect from "$lib/components/ui/native-select";
	import * as Popover from "$lib/components/ui/popover";
	import { Switch } from "$lib/components/ui/switch";
	import * as ToggleGroup from "$lib/components/ui/toggle-group";
	import { hiddenLabel, setRefreshInterval, unhide } from "$lib/ports/ports-store.svelte";
	import { editors, preferences, refreshIntervals } from "$lib/ports/preferences.svelte";
	import { settings, type AlertSettings } from "$lib/settings.svelte";
	import ShortcutRecorder from "./shortcut-recorder.svelte";

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

	let shortcutError = $state<string | null>(null);

	const alerts = $derived(settings.current.alerts);

	async function setAlerts(change: Partial<AlertSettings>) {
		try {
			await settings.setAlerts({ ...alerts, ...change });
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

<Popover.Root
	onOpenChange={(open) => {
		if (open) void readOpenAtLogin();
		else shortcutError = null;
	}}
>
	<Popover.Trigger>
		{#snippet child({ props })}
			<Button {...props} variant="ghost" size="icon-sm" aria-label="Settings">
				<SettingsIcon />
			</Button>
		{/snippet}
	</Popover.Trigger>
	<Popover.Content {side} align="end" class="max-h-(--bits-popover-content-available-height) w-72 overflow-y-auto">
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
					<Field.Label for="{id}-alerts">Memory alerts</Field.Label>
					<Field.Description>When a server's memory keeps growing.</Field.Description>
				</Field.Content>
				<Switch
					id="{id}-alerts"
					disabled={!desktop}
					bind:checked={() => alerts.enabled, (enabled) => setAlerts({ enabled })}
				/>
			</Field.Field>
			<Field.Separator />
			<Field.Field orientation="horizontal" data-invalid={shortcutError ? true : undefined}>
				<Field.Content>
					<Field.Label for="{id}-shortcut">Show Ports</Field.Label>
					<Field.Description>From anywhere.</Field.Description>
				</Field.Content>
				<ShortcutRecorder id="{id}-shortcut" onerror={(error) => (shortcutError = error)} />
			</Field.Field>
			{#if shortcutError}
				<Field.Error>{shortcutError}</Field.Error>
			{/if}
			{#if preferences.hidden.current.length}
				<Field.Separator />
				<Field.Field>
					<Field.Content>
						<Field.Title>Hidden</Field.Title>
						<Field.Description>Left out of both lists.</Field.Description>
					</Field.Content>
					<div class="flex flex-wrap gap-1.5">
						{#each preferences.hidden.current as entry (entry)}
							<Badge variant="secondary" class="gap-0.5 pr-0.5">
								{hiddenLabel(entry)}
								<Button
									variant="ghost"
									size="icon-xs"
									class="size-4 rounded-full"
									aria-label="Show {hiddenLabel(entry)} again"
									onclick={() => unhide(entry)}
								>
									<XIcon />
								</Button>
							</Badge>
						{/each}
					</div>
				</Field.Field>
			{/if}
		</Field.Group>
	</Popover.Content>
</Popover.Root>
