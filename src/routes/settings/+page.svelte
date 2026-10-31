<script lang="ts">
	import type { Component, Snippet } from "svelte";
	import MonitorIcon from "@lucide/svelte/icons/monitor";
	import MoonIcon from "@lucide/svelte/icons/moon";
	import SunIcon from "@lucide/svelte/icons/sun";
	import XIcon from "@lucide/svelte/icons/x";
	import { isTauri } from "@tauri-apps/api/core";
	import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
	import { setMode, userPrefersMode } from "mode-watcher";
	import { onMount } from "svelte";
	import { toast } from "svelte-sonner";
	import { goto } from "$app/navigation";
	import { resolve } from "$app/paths";
	import { Button } from "$lib/components/ui/button";
	import AppShell from "$lib/components/app-shell.svelte";
	import * as AlertDialog from "$lib/components/ui/alert-dialog";
	import * as Card from "$lib/components/ui/card";
	import * as Field from "$lib/components/ui/field";
	import { Switch } from "$lib/components/ui/switch";
	import * as ToggleGroup from "$lib/components/ui/toggle-group";
	import * as Tooltip from "$lib/components/ui/tooltip";
	import AutoCleanupSettings from "$lib/components/ports/auto-cleanup-settings.svelte";
	import FilterRules from "$lib/components/ports/filter-rules.svelte";
	import PlatformSelect from "$lib/components/ports/platform-select.svelte";
	import ShortcutRecorder from "$lib/components/ports/shortcut-recorder.svelte";
	import { setRefreshInterval } from "$lib/ports/ports-store.svelte";
	import { editors, groupHeaderSizes, preferences, refreshIntervals } from "$lib/ports/preferences.svelte";
	import { alertThresholds, idleThresholds, settings, stopGraces, type AlertSettings } from "$lib/settings.svelte";
	import { resetSections, resetSettings, type SettingsSection, type ResetScope } from "$lib/reset-settings";

	type Mode = "light" | "dark" | "system";

	const id = $props.id();
	const desktop = isTauri();

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

	onMount(() => void readOpenAtLogin());

	function onkeydown(event: KeyboardEvent) {
		if (event.key === "Escape" && !event.defaultPrevented) void goto(resolve("/"));
	}

	let shortcutError = $state<string | null>(null);
	let resetting = $state<ResetScope | null>(null);
	let resetOpen = $state(false);
	let resetScope = $state<ResetScope>("all");
	const resetTitle = $derived(
		resetScope === "all" ? "Reset all settings?" : `Reset ${resetSections[resetScope]} settings?`,
	);
	const resetDescriptions: Record<ResetScope, string> = {
		appearance: "Restores the system theme and keeps the header visible.",
		general: "Restores the default shortcut and turns off Open at login.",
		servers:
			"Restores the default editor, refresh interval, confirmation prompts, and time before force killing a server.",
		cleanUp: "Turns off automatic clean up and restores the default idle threshold. No servers will be stopped.",
		memoryAlerts: "Restores the default leak notifications and high memory threshold.",
		lists:
			"Restores list layout, sorting, grouping and visibility, clears filters, and shows hidden servers. Pins and protection stay.",
		all: "Restores app preferences and filters, shows hidden servers, and turns off automatic clean up. Saved projects, server names, pins and protection stay.",
	};

	function confirmReset(scope: ResetScope) {
		resetScope = scope;
		resetOpen = true;
	}

	async function restoreDefaults(scope: ResetScope) {
		if (resetting) return;
		resetting = scope;
		try {
			await resetSettings(scope);
			await readOpenAtLogin();
			shortcutError = null;
			resetOpen = false;
			toast.success(scope === "all" ? "All settings restored" : `${resetSections[scope]} settings restored`);
		} catch (error) {
			toast.error(`Could not finish resetting settings: ${String(error)}`);
		} finally {
			resetting = null;
		}
	}

	const alerts = $derived(settings.current.alerts);

	async function setAlerts(change: Partial<AlertSettings>) {
		try {
			await settings.setAlerts(change);
		} catch (error) {
			toast.error(String(error));
		}
	}

	async function setStopGrace(seconds: number) {
		try {
			await settings.setStopGrace(seconds);
		} catch (error) {
			toast.error(String(error));
		}
	}

	async function setIdleAfter(minutes: number) {
		try {
			await settings.setIdleAfter(minutes);
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

{#snippet section(title: string, fields: Snippet, scope: SettingsSection)}
	<!-- A legend isn't a flex item of its fieldset, so its margin spaces it, not the gap. -->
	<Field.Set>
		<Field.Legend variant="label" class="mb-3 px-1 text-muted-foreground">{title}</Field.Legend>
		<Card.Root size="sm" class="rounded-2xl">
			<Card.Content>
				<Field.Group class="gap-4">
					{@render fields()}
				</Field.Group>
			</Card.Content>
			<Card.Footer class="justify-end border-t">
				<Button
					variant="ghost"
					size="sm"
					disabled={!desktop || resetting !== null}
					aria-label="Reset {title} to defaults"
					onclick={() => confirmReset(scope)}
				>
					Reset to defaults
				</Button>
			</Card.Footer>
		</Card.Root>
	</Field.Set>
{/snippet}

<svelte:window onfocus={readOpenAtLogin} {onkeydown} />

<AppShell>
	{#snippet header()}
		<div class="mx-auto flex min-h-15 w-full max-w-5xl items-center gap-3 px-6 py-3">
			<h1 class="text-lg font-semibold tracking-tight">Settings</h1>
			<Tooltip.Root>
				<Tooltip.Trigger>
					{#snippet child({ props })}
						<Button {...props} href={resolve("/")} variant="ghost" size="icon-sm" class="ml-auto" aria-label="Close">
							<XIcon />
						</Button>
					{/snippet}
				</Tooltip.Trigger>
				<Tooltip.Content>Close Esc</Tooltip.Content>
			</Tooltip.Root>
		</div>
	{/snippet}

	<div class="mx-auto flex w-full max-w-2xl flex-col gap-6 p-6">
		{#snippet appearance()}
			<Field.Field>
				<Field.Title id="{id}-theme">Theme</Field.Title>
				<ToggleGroup.Root
					type="single"
					variant="outline"
					size="sm"
					class="w-full"
					aria-labelledby="{id}-theme"
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
				<Field.Content>
					<Field.Label for="{id}-keep-header">Keep header visible</Field.Label>
					<Field.Description>Stays at the top of the window while you scroll.</Field.Description>
				</Field.Content>
				<Switch id="{id}-keep-header" bind:checked={preferences.keepHeaderVisible.current} />
			</Field.Field>
		{/snippet}
		{@render section("Appearance", appearance, "appearance")}

		{#snippet general()}
			<Field.Field orientation="horizontal">
				<Field.Label for="{id}-open-at-login">Open at login</Field.Label>
				<Switch id="{id}-open-at-login" disabled={!desktop} bind:checked={() => openAtLogin, setOpenAtLogin} />
			</Field.Field>
			<Field.Separator />
			<Field.Field orientation="horizontal" data-invalid={shortcutError ? true : undefined}>
				<Field.Content>
					<Field.Label for="{id}-shortcut">Show Ports</Field.Label>
					<Field.Description>Opens the menu bar popover from any app.</Field.Description>
				</Field.Content>
				<ShortcutRecorder id="{id}-shortcut" onerror={(error) => (shortcutError = error)} />
			</Field.Field>
			{#if shortcutError}
				<Field.Error>{shortcutError}</Field.Error>
			{/if}
		{/snippet}
		{@render section("General", general, "general")}

		{#snippet servers()}
			<Field.Field orientation="horizontal">
				<Field.Label for="{id}-editor">Editor</Field.Label>
				<PlatformSelect id="{id}-editor" class="w-36" options={editors} bind:value={preferences.editor.current} />
			</Field.Field>
			<Field.Separator />
			<Field.Field orientation="horizontal">
				<Field.Label for="{id}-refresh">Refresh</Field.Label>
				<PlatformSelect
					id="{id}-refresh"
					class="w-36"
					options={refreshIntervals.map(({ value, label }) => ({ value: String(value), label }))}
					bind:value={() => String(preferences.refreshInterval.current), (value) => setRefreshInterval(Number(value))}
				/>
			</Field.Field>
			<Field.Separator />
			<Field.Field orientation="horizontal">
				<Field.Content>
					<Field.Label for="{id}-confirm">Confirm actions</Field.Label>
					<Field.Description
						>Ask before stopping or restarting a server. Force Kill and broader stops always ask.</Field.Description
					>
				</Field.Content>
				<Switch id="{id}-confirm" bind:checked={preferences.confirmActions.current} />
			</Field.Field>
			<Field.Separator />
			<Field.Field orientation="horizontal">
				<Field.Content>
					<Field.Label for="{id}-grace">Wait before killing</Field.Label>
					<Field.Description
						>How long Stop gives a server to exit on its own. Force Kill doesn't wait.</Field.Description
					>
				</Field.Content>
				<PlatformSelect
					id="{id}-grace"
					class="w-36"
					disabled={!desktop}
					options={stopGraces}
					bind:value={() => String(settings.current.stopGraceSeconds), (value) => setStopGrace(Number(value))}
				/>
			</Field.Field>
		{/snippet}
		{@render section("Servers", servers, "servers")}

		{#snippet cleanUp()}
			<Field.Field orientation="horizontal">
				<Field.Content>
					<Field.Label for="{id}-idle">Idle after</Field.Label>
					<Field.Description>Without connections or CPU.</Field.Description>
				</Field.Content>
				<PlatformSelect
					id="{id}-idle"
					class="w-36"
					disabled={!desktop}
					options={idleThresholds}
					bind:value={() => String(settings.current.idleAfterMinutes), (value) => setIdleAfter(Number(value))}
				/>
			</Field.Field>
			<Field.Separator />
			<AutoCleanupSettings {id} disabled={!desktop} />
		{/snippet}
		{@render section("Clean up", cleanUp, "cleanUp")}

		{#snippet memory()}
			<Field.Field orientation="horizontal">
				<Field.Content>
					<Field.Label for="{id}-leaks">Leaks</Field.Label>
					<Field.Description>When a server's memory keeps growing for ten minutes.</Field.Description>
				</Field.Content>
				<Switch
					id="{id}-leaks"
					disabled={!desktop}
					bind:checked={() => alerts.enabled, (enabled) => setAlerts({ enabled })}
				/>
			</Field.Field>
			<Field.Separator />
			<Field.Field orientation="horizontal">
				<Field.Content>
					<Field.Label for="{id}-threshold">High memory</Field.Label>
					<Field.Description>When a server uses more than this.</Field.Description>
				</Field.Content>
				<PlatformSelect
					id="{id}-threshold"
					class="w-36"
					disabled={!desktop}
					options={alertThresholds}
					bind:value={
						() => (alerts.thresholdBytes === null ? "off" : String(alerts.thresholdBytes)),
						(value) => setAlerts({ thresholdBytes: value === "off" ? null : Number(value) })
					}
				/>
			</Field.Field>
		{/snippet}
		{@render section("Memory alerts", memory, "memoryAlerts")}

		{#snippet lists()}
			<Field.Field orientation="horizontal">
				<Field.Label for="{id}-group-headers">Group headers</Field.Label>
				<PlatformSelect
					id="{id}-group-headers"
					class="w-36"
					options={groupHeaderSizes}
					bind:value={preferences.groupHeaders.current}
				/>
			</Field.Field>
			<Field.Separator />
			<FilterRules {id} disabled={!desktop} />
		{/snippet}
		{@render section("Lists", lists, "lists")}
		<div class="flex justify-end">
			<Button variant="outline" disabled={!desktop || resetting !== null} onclick={() => confirmReset("all")}
				>Reset all settings…</Button
			>
		</div>
	</div>
</AppShell>

<AlertDialog.Root bind:open={resetOpen}>
	<AlertDialog.Content
		onEscapeKeydown={(event) => {
			if (resetting) event.preventDefault();
		}}
	>
		<AlertDialog.Header>
			<AlertDialog.Title>{resetTitle}</AlertDialog.Title>
			<AlertDialog.Description>
				{resetDescriptions[resetScope]}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={resetting !== null}>Cancel</AlertDialog.Cancel>
			<Button disabled={resetting !== null} aria-busy={resetting !== null} onclick={() => restoreDefaults(resetScope)}>
				{resetScope === "all" ? "Reset all settings" : "Reset to defaults"}
			</Button>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
