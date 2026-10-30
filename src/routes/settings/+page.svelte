<script lang="ts">
	import type { Component, Snippet } from "svelte";
	import MonitorIcon from "@lucide/svelte/icons/monitor";
	import MoonIcon from "@lucide/svelte/icons/moon";
	import SunIcon from "@lucide/svelte/icons/sun";
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
	import * as NativeSelect from "$lib/components/ui/native-select";
	import { Switch } from "$lib/components/ui/switch";
	import * as ToggleGroup from "$lib/components/ui/toggle-group";
	import AutoCleanupSettings from "$lib/components/ports/auto-cleanup-settings.svelte";
	import FilterRules from "$lib/components/ports/filter-rules.svelte";
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
		general: "Restores system appearance and the default shortcut, and turns off Open at login.",
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
			<Button href={resolve("/")} variant="secondary" size="sm" class="ml-auto">Done</Button>
		</div>
	{/snippet}

	<div class="mx-auto flex w-full max-w-2xl flex-col gap-6 p-6">
		{#snippet general()}
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
				<NativeSelect.Root id="{id}-editor" size="sm" class="w-36" bind:value={preferences.editor.current}>
					{#each editors as editor (editor.value)}
						<NativeSelect.Option value={editor.value}>{editor.label}</NativeSelect.Option>
					{/each}
				</NativeSelect.Root>
			</Field.Field>
			<Field.Separator />
			<Field.Field orientation="horizontal">
				<Field.Label for="{id}-refresh">Refresh</Field.Label>
				<NativeSelect.Root
					id="{id}-refresh"
					size="sm"
					class="w-36"
					bind:value={() => String(preferences.refreshInterval.current), (value) => setRefreshInterval(Number(value))}
				>
					{#each refreshIntervals as interval (interval.value)}
						<NativeSelect.Option value={String(interval.value)}>{interval.label}</NativeSelect.Option>
					{/each}
				</NativeSelect.Root>
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
				<NativeSelect.Root
					id="{id}-grace"
					size="sm"
					class="w-36"
					disabled={!desktop}
					bind:value={() => String(settings.current.stopGraceSeconds), (value) => setStopGrace(Number(value))}
				>
					{#each stopGraces as grace (grace.value)}
						<NativeSelect.Option value={grace.value}>{grace.label}</NativeSelect.Option>
					{/each}
				</NativeSelect.Root>
			</Field.Field>
		{/snippet}
		{@render section("Servers", servers, "servers")}

		{#snippet cleanUp()}
			<Field.Field orientation="horizontal">
				<Field.Content>
					<Field.Label for="{id}-idle">Idle after</Field.Label>
					<Field.Description>Without connections or CPU.</Field.Description>
				</Field.Content>
				<NativeSelect.Root
					id="{id}-idle"
					size="sm"
					class="w-36"
					disabled={!desktop}
					bind:value={() => String(settings.current.idleAfterMinutes), (value) => setIdleAfter(Number(value))}
				>
					{#each idleThresholds as threshold (threshold.value)}
						<NativeSelect.Option value={threshold.value}>{threshold.label}</NativeSelect.Option>
					{/each}
				</NativeSelect.Root>
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
				<NativeSelect.Root
					id="{id}-threshold"
					size="sm"
					class="w-36"
					disabled={!desktop}
					bind:value={
						() => (alerts.thresholdBytes === null ? "off" : String(alerts.thresholdBytes)),
						(value) => setAlerts({ thresholdBytes: value === "off" ? null : Number(value) })
					}
				>
					{#each alertThresholds as threshold (threshold.value)}
						<NativeSelect.Option value={threshold.value}>{threshold.label}</NativeSelect.Option>
					{/each}
				</NativeSelect.Root>
			</Field.Field>
		{/snippet}
		{@render section("Memory alerts", memory, "memoryAlerts")}

		{#snippet lists()}
			<Field.Field orientation="horizontal">
				<Field.Label for="{id}-group-headers">Group headers</Field.Label>
				<NativeSelect.Root id="{id}-group-headers" size="sm" class="w-36" bind:value={preferences.groupHeaders.current}>
					{#each groupHeaderSizes as size (size.value)}
						<NativeSelect.Option value={size.value}>{size.label}</NativeSelect.Option>
					{/each}
				</NativeSelect.Root>
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
