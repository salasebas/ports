<script lang="ts">
	import type { Component, Snippet } from "svelte";
	import ChevronLeftIcon from "@lucide/svelte/icons/chevron-left";
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
	import { Badge } from "$lib/components/ui/badge";
	import { Button } from "$lib/components/ui/button";
	import * as Card from "$lib/components/ui/card";
	import * as Field from "$lib/components/ui/field";
	import * as NativeSelect from "$lib/components/ui/native-select";
	import { Switch } from "$lib/components/ui/switch";
	import * as ToggleGroup from "$lib/components/ui/toggle-group";
	import ShortcutRecorder from "$lib/components/ports/shortcut-recorder.svelte";
	import { cn } from "$lib/utils";
	import { hiddenLabel, setRefreshInterval, unhide } from "$lib/ports/ports-store.svelte";
	import { editors, preferences, refreshIntervals } from "$lib/ports/preferences.svelte";
	import { alertThresholds, settings, type AlertSettings } from "$lib/settings.svelte";

	type Mode = "light" | "dark" | "system";

	const id = $props.id();
	const desktop = isTauri();

	/** Owned by the OS, so it's read whenever the window comes forward instead of stored. */
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

	let scrollY = $state(0);

	function onkeydown(event: KeyboardEvent) {
		// The shortcut recorder and open menus handle Escape first and prevent default.
		if (event.key === "Escape" && !event.defaultPrevented) void goto(resolve("/"));
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

{#snippet section(title: string, fields: Snippet)}
	<!-- A legend isn't a flex item of its fieldset, so its margin spaces it, not the gap. -->
	<Field.Set>
		<Field.Legend variant="label" class="mb-3 px-1 text-muted-foreground">{title}</Field.Legend>
		<Card.Root size="sm" class="rounded-2xl">
			<Card.Content>
				<Field.Group class="gap-4">
					{@render fields()}
				</Field.Group>
			</Card.Content>
		</Card.Root>
	</Field.Set>
{/snippet}

<svelte:window onfocus={readOpenAtLogin} {onkeydown} bind:scrollY />

<div class="flex min-h-screen flex-col">
	<header
		class={cn(
			"sticky top-0 z-10 grid grid-cols-[1fr_auto_1fr] items-center border-b bg-background/80 px-4 py-3 backdrop-blur-xl backdrop-saturate-150 transition-colors duration-200",
			scrollY > 0 ? "border-border" : "border-transparent",
		)}
	>
		<Button href={resolve("/")} variant="ghost" size="sm" class="justify-self-start">
			<ChevronLeftIcon data-icon="inline-start" />
			Ports
		</Button>
		<h1 class="text-lg font-semibold tracking-tight">Settings</h1>
	</header>

	<main class="mx-auto flex w-full max-w-lg flex-col gap-6 px-6 py-6">
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
		{@render section("General", general)}

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
					<Field.Description>Ask before stopping or restarting a server.</Field.Description>
				</Field.Content>
				<Switch id="{id}-confirm" bind:checked={preferences.confirmActions.current} />
			</Field.Field>
		{/snippet}
		{@render section("Servers", servers)}

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
		{@render section("Memory alerts", memory)}

		{#snippet hidden()}
			<Field.Field>
				<Field.Content>
					<Field.Title>Hidden servers</Field.Title>
					<Field.Description>
						{preferences.hidden.current.length
							? "Left out of both lists. Remove one to show it again."
							: "Hide a server from its details to leave it out of both lists."}
					</Field.Description>
				</Field.Content>
				{#if preferences.hidden.current.length}
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
				{/if}
			</Field.Field>
		{/snippet}
		{@render section("Lists", hidden)}
	</main>
</div>
