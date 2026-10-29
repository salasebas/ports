<script lang="ts">
	import AppWindowIcon from "@lucide/svelte/icons/app-window";
	import PowerIcon from "@lucide/svelte/icons/power";
	import { invoke } from "@tauri-apps/api/core";
	import { getCurrentWindow } from "@tauri-apps/api/window";
	import { prefersReducedMotion } from "svelte/motion";
	import { onMount } from "svelte";
	import { fly } from "svelte/transition";
	import { cubicOut } from "svelte/easing";
	import { Button } from "$lib/components/ui/button";
	import { ScrollArea } from "$lib/components/ui/scroll-area";
	import { Separator } from "$lib/components/ui/separator";
	import CleanUpButton from "$lib/components/ports/clean-up-button.svelte";
	import CleanUpDialog from "$lib/components/ports/clean-up-dialog.svelte";
	import NoResults from "$lib/components/ports/no-results.svelte";
	import RenameDialog from "$lib/components/ports/rename-dialog.svelte";
	import ServerDetails from "$lib/components/ports/server-details.svelte";
	import ServerFilter from "$lib/components/ports/server-filter.svelte";
	import ServerRow from "$lib/components/ports/server-row.svelte";
	import ServerSearch from "$lib/components/ports/server-search.svelte";
	import SettingsButton from "$lib/components/ports/settings-button.svelte";
	import ConfirmDialog from "$lib/components/ports/confirm-dialog.svelte";
	import UsageSummary from "$lib/components/ports/usage-summary.svelte";
	import { PortsStore } from "$lib/ports/ports-store.svelte";

	const store = new PortsStore();
	onMount(store.connect);

	const slide = (direction: 1 | -1) => ({
		x: prefersReducedMotion.current ? 0 : 24 * direction,
		duration: 200,
		easing: cubicOut,
	});

	function onkeydown(event: KeyboardEvent) {
		if (event.key !== "Escape" || event.defaultPrevented) return;
		if (store.selected) store.select(null);
		else getCurrentWindow().hide();
	}
</script>

<svelte:window {onkeydown} />

<div
	data-surface="tray"
	class="grid h-screen overflow-hidden rounded-[14px] border bg-surface-translucent text-foreground *:col-start-1 *:row-start-1"
>
	{#if store.selected}
		<div class="flex min-h-0 flex-col" in:fly={slide(1)} out:fly={slide(1)}>
			<ScrollArea class="min-h-0 flex-1">
				<ServerDetails server={store.selected} {store} onBack={() => store.select(null)} class="p-4" />
			</ScrollArea>
		</div>
	{:else}
		<div class="flex min-h-0 flex-col" in:fly={slide(-1)} out:fly={slide(-1)}>
			<header class="flex flex-col gap-4 px-4 pt-3 pb-4">
				<h1 class="text-center text-sm font-semibold">Ports</h1>
				<UsageSummary {store} />
				<div class="flex items-center gap-1">
					<ServerSearch bind:value={store.query} class="flex-1" />
					<ServerFilter {store} />
				</div>
			</header>

			<Separator />

			<ScrollArea class="min-h-0 flex-1">
				{#if store.filtered.length}
					<ul class="flex flex-col p-2">
						{#each store.filtered as server (server.id)}
							<ServerRow {server} {store} />
						{/each}
					</ul>
				{:else}
					<NoResults {store} class="p-8" />
				{/if}
			</ScrollArea>

			<Separator />

			<footer class="flex items-center justify-between px-3 py-2.5">
				<Button variant="secondary" size="sm" onclick={() => invoke("open_main_window")}>
					<AppWindowIcon data-icon="inline-start" />
					Open Ports
				</Button>
				<div class="flex items-center gap-0.5">
					<CleanUpButton {store} />
					<SettingsButton />
					<Button variant="ghost" size="icon-sm" aria-label="Quit Ports" onclick={() => invoke("quit_app")}>
						<PowerIcon />
					</Button>
				</div>
			</footer>
		</div>
	{/if}
</div>

<ConfirmDialog {store} />
<CleanUpDialog {store} />
<RenameDialog {store} />
