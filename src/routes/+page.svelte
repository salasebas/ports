<script lang="ts">
	import { onMount, type Component } from "svelte";
	import { flip } from "svelte/animate";
	import { cubicOut } from "svelte/easing";
	import { prefersReducedMotion } from "svelte/motion";
	import { fade } from "svelte/transition";
	import ArrowUpRightIcon from "@lucide/svelte/icons/arrow-up-right";
	import CodeIcon from "@lucide/svelte/icons/code";
	import CopyIcon from "@lucide/svelte/icons/copy";
	import CpuIcon from "@lucide/svelte/icons/cpu";
	import EllipsisIcon from "@lucide/svelte/icons/ellipsis";
	import FolderOpenIcon from "@lucide/svelte/icons/folder-open";
	import HashIcon from "@lucide/svelte/icons/hash";
	import InfoIcon from "@lucide/svelte/icons/info";
	import LayersIcon from "@lucide/svelte/icons/layers";
	import MemoryStickIcon from "@lucide/svelte/icons/memory-stick";
	import RefreshCwIcon from "@lucide/svelte/icons/refresh-cw";
	import ServerIcon from "@lucide/svelte/icons/server";
	import SquareIcon from "@lucide/svelte/icons/square";
	import SquareTerminalIcon from "@lucide/svelte/icons/square-terminal";
	import XIcon from "@lucide/svelte/icons/x";
	import { Badge } from "$lib/components/ui/badge";
	import { Button } from "$lib/components/ui/button";
	import * as Card from "$lib/components/ui/card";
	import * as Drawer from "$lib/components/ui/drawer";
	import * as DropdownMenu from "$lib/components/ui/dropdown-menu";
	import * as Table from "$lib/components/ui/table";
	import * as Tooltip from "$lib/components/ui/tooltip";
	import NoResults from "$lib/components/ports/no-results.svelte";
	import PortLabel from "$lib/components/ports/port-label.svelte";
	import ServerDetails from "$lib/components/ports/server-details.svelte";
	import ServerFilter from "$lib/components/ports/server-filter.svelte";
	import ServerSearch from "$lib/components/ports/server-search.svelte";
	import SettingsMenu from "$lib/components/ports/settings-menu.svelte";
	import Sparkline from "$lib/components/ports/sparkline.svelte";
	import ConfirmDialog from "$lib/components/ports/confirm-dialog.svelte";
	import UsageSummary from "$lib/components/ports/usage-summary.svelte";
	import { canStop, formatBytesText, isMeasured, serverActivity, serverProcessId } from "$lib/ports/format";
	import { fileManagerName } from "$lib/platform";
	import { PortsStore } from "$lib/ports/ports-store.svelte";
	import { editorLabel, preferences } from "$lib/ports/preferences.svelte";
	import type { Server, ServerStatus } from "$lib/ports/types";
	import { cn } from "$lib/utils";

	const store = new PortsStore();
	onMount(store.connect);

	let scrollY = $state(0);
	let tableHeight = $state(0);

	const motion = $derived(prefersReducedMotion.current ? 0 : 1);

	const statusDot: Record<ServerStatus, string> = {
		running: "bg-success",
		idle: "bg-muted-foreground/40",
		attention: "bg-warning",
	};

	// Keeps the drawer populated while it animates out after a stop.
	let lastSelected = $state.raw<Server | null>(null);
	const shown = $derived(store.selected ?? lastSelected);

	function showDetails(server: Server) {
		lastSelected = server;
		store.select(server.port);
	}
</script>

{#snippet action(label: string, Icon: Component, onclick: () => void, disabled = false)}
	<Tooltip.Root>
		<Tooltip.Trigger>
			{#snippet child({ props })}
				<Button
					{...props}
					variant="ghost"
					size="icon-sm"
					aria-label={label}
					{disabled}
					onclick={(event: MouseEvent) => {
						event.stopPropagation();
						onclick();
					}}
				>
					<Icon />
				</Button>
			{/snippet}
		</Tooltip.Trigger>
		<Tooltip.Content>{label}</Tooltip.Content>
	</Tooltip.Root>
{/snippet}

{#snippet more(server: Server)}
	<DropdownMenu.Root>
		<DropdownMenu.Trigger>
			{#snippet child({ props })}
				<Button
					{...props}
					variant="ghost"
					size="icon-sm"
					aria-label="More actions for {server.name}"
					onclick={(event: MouseEvent) => event.stopPropagation()}
				>
					<EllipsisIcon />
				</Button>
			{/snippet}
		</DropdownMenu.Trigger>
		<DropdownMenu.Content align="end" class="w-48">
			{#if server.folder}
				<DropdownMenu.Item disabled={store.isBusy(server, "editor")} onSelect={() => store.openInEditor(server)}>
					<CodeIcon />
					Open in {editorLabel(preferences.editor.current)}
				</DropdownMenu.Item>
				<DropdownMenu.Item disabled={store.isBusy(server, "terminal")} onSelect={() => store.openInTerminal(server)}>
					<SquareTerminalIcon />
					Open in Terminal
				</DropdownMenu.Item>
				<DropdownMenu.Item disabled={store.isBusy(server, "folder")} onSelect={() => store.revealFolder(server)}>
					<FolderOpenIcon />
					Show in {fileManagerName}
				</DropdownMenu.Item>
				<DropdownMenu.Separator />
			{/if}
			<DropdownMenu.Item onSelect={() => store.copyUrl(server)}>
				<CopyIcon />
				Copy URL
			</DropdownMenu.Item>
			<DropdownMenu.Item onSelect={() => showDetails(server)}>
				<InfoIcon />
				Details
			</DropdownMenu.Item>
		</DropdownMenu.Content>
	</DropdownMenu.Root>
{/snippet}

{#snippet head(label: string, Icon: Component, className = "")}
	<Table.Head class={cn("h-10 content-center text-xs font-normal text-muted-foreground", className)}>
		<span class={cn("inline-flex items-center gap-1.5", className.includes("text-right") && "justify-end")}>
			<Icon class="size-3.5 opacity-70" />
			{label}
		</span>
	</Table.Head>
{/snippet}

<svelte:window bind:scrollY />

<div class="flex min-h-screen flex-col">
	<header
		class={cn(
			"sticky top-0 z-10 flex flex-wrap items-center gap-3 border-b bg-background/80 px-6 py-3 backdrop-blur-xl backdrop-saturate-150 transition-colors duration-200",
			scrollY > 0 ? "border-border" : "border-transparent",
		)}
	>
		<div class="flex items-center gap-2">
			<h1 class="text-lg font-semibold tracking-tight">Ports</h1>
			<Badge variant="secondary">{store.shown.length}</Badge>
		</div>
		<div class="ml-auto flex items-center gap-2">
			<ServerSearch bind:value={store.query} class="w-64" />
			<ServerFilter {store} />
			{@render action("Refresh", RefreshCwIcon, store.refresh)}
			<SettingsMenu />
		</div>
	</header>

	<main class="mx-auto flex w-full max-w-5xl flex-1 flex-col gap-6 p-6">
		<Card.Root>
			<Card.Content>
				<UsageSummary {store} />
			</Card.Content>
		</Card.Root>

		{#if store.servers.length}
			<!-- Height follows the content with a transition so rows sliding into place are not clipped. -->
			<div
				class="overflow-hidden rounded-2xl border bg-card transition-[height] duration-200 ease-out motion-reduce:transition-none"
				style:height={tableHeight ? `${tableHeight + 2}px` : undefined}
			>
				<div bind:clientHeight={tableHeight} class="[&>[data-slot=table-container]]:overflow-visible">
					<!-- Rows are grids on shared tracks so a row leaving the flow mid-animation keeps its columns. -->
					<Table.Root class="grid [--cols:7rem_minmax(0,1fr)_9rem_5rem_11rem_8rem]">
						<Table.Header class="grid bg-muted/60">
							<Table.Row class="grid grid-cols-(--cols) items-center hover:bg-transparent">
								{@render head("Port", HashIcon, "pl-5")}
								{@render head("Server", ServerIcon)}
								{@render head("Framework", LayersIcon)}
								{@render head("CPU", CpuIcon, "text-right")}
								{@render head("Memory", MemoryStickIcon, "text-right")}
								<Table.Head class="pr-5"><span class="sr-only">Actions</span></Table.Head>
							</Table.Row>
						</Table.Header>
						<Table.Body class="grid">
							{#each store.filtered as server (server.port)}
								{@const attention = server.status === "attention"}
								<!-- A plain <tr> because animate: and transitions need an element, not a component. -->
								<tr
									data-slot="table-row"
									class="group/row grid cursor-default grid-cols-(--cols) items-center border-b transition-colors hover:bg-muted/60 data-[state=selected]:bg-muted"
									data-state={store.selectedPort === server.port ? "selected" : undefined}
									onclick={() => showDetails(server)}
									animate:flip={{ duration: 220 * motion, easing: cubicOut }}
									in:fade={{ duration: 180 * motion, delay: 60 * motion }}
									out:fade={{ duration: 120 * motion }}
								>
									<Table.Cell class="py-3.5 pl-5">
										<PortLabel port={server.port} class="text-[15px]" />
									</Table.Cell>
									<Table.Cell class="min-w-0">
										<div class="flex items-center gap-3">
											<span class={cn("size-2 shrink-0 rounded-full", statusDot[server.status])} title={server.status}
											></span>
											<div class="flex min-w-0 flex-col gap-0.5">
												<span class="truncate font-medium">{server.name}</span>
												<span class={cn("truncate text-xs", attention ? "text-warning" : "text-muted-foreground")}>
													{server.branch ? `${server.branch} · ` : ""}{serverActivity(server)}
												</span>
											</div>
										</div>
									</Table.Cell>
									<Table.Cell>
										<div class="flex flex-col gap-0.5">
											<span>{server.framework ?? server.process}</span>
											<span class="font-mono text-xs text-muted-foreground">{serverProcessId(server)}</span>
										</div>
									</Table.Cell>
									{#if !isMeasured(server)}
										<Table.Cell class="text-right text-muted-foreground">–</Table.Cell>
										<Table.Cell class="text-right text-muted-foreground">–</Table.Cell>
									{:else}
										<Table.Cell class="text-right font-mono tabular-nums">
											{Math.round(server.cpuPercent)}%
										</Table.Cell>
										<Table.Cell>
											<div class="flex items-center justify-end gap-3">
												<Sparkline
													values={server.memoryHistory}
													class={cn("h-4 w-14", attention ? "text-warning" : "text-muted-foreground")}
												/>
												<span class={cn("w-16 text-right font-mono tabular-nums", attention && "text-warning")}>
													{formatBytesText(server.memoryBytes)}
												</span>
											</div>
										</Table.Cell>
									{/if}
									<Table.Cell class="pr-5">
										<div
											class="flex justify-end gap-0.5 opacity-0 transition-opacity duration-150 will-change-[opacity] group-hover/row:opacity-100 focus-within:opacity-100 has-data-[state=open]:opacity-100"
										>
											{@render action(`Open localhost:${server.port}`, ArrowUpRightIcon, () => store.open(server))}
											{@render more(server)}
											{#if canStop(server)}
												{@render action(
													`Stop ${server.name}`,
													SquareIcon,
													() => store.request("stop", server),
													store.isBusy(server, "control"),
												)}
											{/if}
										</div>
									</Table.Cell>
								</tr>
							{/each}
						</Table.Body>
					</Table.Root>
					{#if !store.filtered.length}
						<div in:fade={{ duration: 180 * motion, delay: 120 * motion }}>
							<NoResults {store} class="py-10" />
						</div>
					{/if}
				</div>
			</div>
		{:else}
			<NoResults {store} class="flex-1" />
		{/if}
	</main>
</div>

<Drawer.Root
	direction="right"
	bind:open={
		() => store.selected !== null,
		(open) => {
			if (!open) store.select(null);
		}
	}
>
	<Drawer.Content class="w-[26rem] sm:max-w-[26rem]">
		<Drawer.Title class="sr-only">Server details</Drawer.Title>
		<Drawer.Close>
			{#snippet child({ props })}
				<Button {...props} variant="ghost" size="icon-sm" class="absolute top-6 right-6" aria-label="Close">
					<XIcon />
				</Button>
			{/snippet}
		</Drawer.Close>
		{#if shown}
			<ServerDetails server={shown} {store} class="min-h-0 flex-1 overflow-y-auto p-3" />
		{/if}
	</Drawer.Content>
</Drawer.Root>

<ConfirmDialog {store} />
