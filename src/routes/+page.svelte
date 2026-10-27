<script lang="ts">
	import { isTauri } from "@tauri-apps/api/core";
	import { listen } from "@tauri-apps/api/event";
	import { onMount, type Component } from "svelte";
	import { flip } from "svelte/animate";
	import { cubicOut } from "svelte/easing";
	import { prefersReducedMotion } from "svelte/motion";
	import { fade } from "svelte/transition";
	import ArrowDownIcon from "@lucide/svelte/icons/arrow-down";
	import ArrowUpDownIcon from "@lucide/svelte/icons/arrow-up-down";
	import ArrowUpIcon from "@lucide/svelte/icons/arrow-up";
	import ArrowUpRightIcon from "@lucide/svelte/icons/arrow-up-right";
	import ClockIcon from "@lucide/svelte/icons/clock";
	import CpuIcon from "@lucide/svelte/icons/cpu";
	import FolderIcon from "@lucide/svelte/icons/folder";
	import HashIcon from "@lucide/svelte/icons/hash";
	import LayersIcon from "@lucide/svelte/icons/layers";
	import MemoryStickIcon from "@lucide/svelte/icons/memory-stick";
	import PinIcon from "@lucide/svelte/icons/pin";
	import RefreshCwIcon from "@lucide/svelte/icons/refresh-cw";
	import ServerIcon from "@lucide/svelte/icons/server";
	import SquareIcon from "@lucide/svelte/icons/square";
	import WifiIcon from "@lucide/svelte/icons/wifi";
	import XIcon from "@lucide/svelte/icons/x";
	import { Badge } from "$lib/components/ui/badge";
	import { Button } from "$lib/components/ui/button";
	import * as Card from "$lib/components/ui/card";
	import * as Drawer from "$lib/components/ui/drawer";
	import { ScrollArea } from "$lib/components/ui/scroll-area";
	import * as DropdownMenu from "$lib/components/ui/dropdown-menu";
	import * as Table from "$lib/components/ui/table";
	import * as Tooltip from "$lib/components/ui/tooltip";
	import CleanUpButton from "$lib/components/ports/clean-up-button.svelte";
	import CleanUpDialog from "$lib/components/ports/clean-up-dialog.svelte";
	import NoResults from "$lib/components/ports/no-results.svelte";
	import PortLabel from "$lib/components/ports/port-label.svelte";
	import ServerDetails from "$lib/components/ports/server-details.svelte";
	import RenameDialog from "$lib/components/ports/rename-dialog.svelte";
	import ServerFilter from "$lib/components/ports/server-filter.svelte";
	import ServerSearch from "$lib/components/ports/server-search.svelte";
	import SettingsMenu from "$lib/components/ports/settings-menu.svelte";
	import Sparkline from "$lib/components/ports/sparkline.svelte";
	import ConfirmDialog from "$lib/components/ports/confirm-dialog.svelte";
	import UsageSummary from "$lib/components/ports/usage-summary.svelte";
	import {
		canStop,
		formatBytesText,
		formatDuration,
		isMeasured,
		serverActivity,
		serverProcessId,
	} from "$lib/ports/format";
	import { PortsStore, type ServerGroup } from "$lib/ports/ports-store.svelte";
	import { preferences, sortKeys, type SortKey } from "$lib/ports/preferences.svelte";
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

	/** Must match `DETAILS_EVENT` in `src-tauri/src/ports/mod.rs`. */
	const DETAILS_EVENT = "ports://details";

	// A click on a memory alert brings this window up on the server's details.
	onMount(() => {
		if (!isTauri()) return;
		const unlisten = listen<number>(DETAILS_EVENT, ({ payload: port }) => {
			const server = store.servers.find((s) => s.port === port);
			if (server) showDetails(server);
		});
		return () => void unlisten.then((stop) => stop());
	});

	/** A new column sorts in its natural direction; the current one flips. */
	function sortBy(key: SortKey) {
		const sort = preferences.sort.current;
		const descending =
			sort.key === key ? !sort.descending : (sortKeys.find((k) => k.value === key)?.descending ?? false);
		preferences.sort.current = { key, descending };
	}

	type Row = { key: number; server: Server; group?: never } | { key: string; group: ServerGroup; server?: never };

	const rows = $derived<Row[]>(
		preferences.groupByProject.current
			? store.groups.flatMap((group) => [
					{ key: `group:${group.folder ?? ""}`, group },
					...group.servers.map((server) => ({ key: server.port, server })),
				])
			: store.filtered.map((server) => ({ key: server.port, server })),
	);
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

{#snippet head(label: string, Icon: Component, className = "", key?: SortKey)}
	{@const sort = preferences.sort.current}
	{@const active = key !== undefined && sort.key === key}
	{@const Arrow = sort.descending ? ArrowDownIcon : ArrowUpIcon}
	<Table.Head
		class={cn("h-10 content-center text-xs font-normal text-muted-foreground", className)}
		aria-sort={active ? (sort.descending ? "descending" : "ascending") : undefined}
	>
		{#if key}
			<Button
				variant="ghost"
				size="xs"
				class={cn("-mx-2 gap-1.5 font-normal whitespace-nowrap text-muted-foreground", active && "text-foreground")}
				onclick={() => sortBy(key)}
			>
				<Icon class="size-3.5 opacity-70" />
				{label}
				<Arrow class={cn("size-3 transition-opacity", !active && "opacity-0")} />
			</Button>
		{:else}
			<span class="inline-flex items-center gap-1.5">
				<Icon class="size-3.5 opacity-70" />
				{label}
			</span>
		{/if}
	</Table.Head>
{/snippet}

{#snippet sortMenu()}
	<DropdownMenu.Root>
		<DropdownMenu.Trigger>
			{#snippet child({ props })}
				<Button {...props} variant="ghost" size="icon-sm" aria-label="Sort and group">
					<ArrowUpDownIcon />
				</Button>
			{/snippet}
		</DropdownMenu.Trigger>
		<DropdownMenu.Content align="end" class="w-52">
			<DropdownMenu.Group>
				<DropdownMenu.GroupHeading>Sort by</DropdownMenu.GroupHeading>
				<DropdownMenu.RadioGroup bind:value={() => preferences.sort.current.key, (key) => sortBy(key as SortKey)}>
					{#each sortKeys as key (key.value)}
						<DropdownMenu.RadioItem value={key.value} closeOnSelect={false}>{key.label}</DropdownMenu.RadioItem>
					{/each}
				</DropdownMenu.RadioGroup>
			</DropdownMenu.Group>
			<DropdownMenu.Separator />
			<DropdownMenu.RadioGroup
				bind:value={
					() => (preferences.sort.current.descending ? "descending" : "ascending"),
					(order) => (preferences.sort.current = { ...preferences.sort.current, descending: order === "descending" })
				}
			>
				<DropdownMenu.RadioItem value="ascending" closeOnSelect={false}>Ascending</DropdownMenu.RadioItem>
				<DropdownMenu.RadioItem value="descending" closeOnSelect={false}>Descending</DropdownMenu.RadioItem>
			</DropdownMenu.RadioGroup>
			<DropdownMenu.Separator />
			<DropdownMenu.CheckboxItem bind:checked={preferences.groupByProject.current}>
				Group by project
			</DropdownMenu.CheckboxItem>
		</DropdownMenu.Content>
	</DropdownMenu.Root>
{/snippet}

{#snippet groupCell(group: ServerGroup)}
	<td class="col-span-full flex min-w-0 items-center gap-2 px-5 py-2 text-xs text-muted-foreground">
		<FolderIcon class="size-3.5 shrink-0 opacity-70" />
		{#if group.folder}
			<span class="shrink-0 font-medium text-foreground">{group.folder.split("/").at(-1)}</span>
			<span class="truncate">{group.folder}</span>
		{:else}
			<span class="font-medium text-foreground">Outside a project</span>
		{/if}
		<span class="ml-auto font-mono tabular-nums">{group.servers.length}</span>
	</td>
{/snippet}

{#snippet serverCells(server: Server)}
	{@const attention = server.status === "attention"}
	{@const leaking = attention && server.responding}
	<Table.Cell class="py-3.5 pl-5">
		<PortLabel port={server.port} class="text-[15px]" />
	</Table.Cell>
	<Table.Cell class="min-w-0">
		<div class="flex items-center gap-3">
			<span class={cn("size-2 shrink-0 rounded-full", statusDot[server.status])} title={server.status}></span>
			<div class="flex min-w-0 flex-col gap-0.5">
				<span class="flex items-center gap-1.5 font-medium">
					{#if store.isPinned(server)}
						<PinIcon class="size-3.5 shrink-0 text-muted-foreground" aria-label="Pinned" />
					{/if}
					<span class="truncate">{server.name}</span>
					{#if server.exposed}
						<Tooltip.Root>
							<Tooltip.Trigger>
								{#snippet child({ props })}
									<WifiIcon
										{...props}
										class="size-3.5 shrink-0 text-muted-foreground"
										aria-label="Reachable from the network"
									/>
								{/snippet}
							</Tooltip.Trigger>
							<Tooltip.Content>Reachable from other devices on the network</Tooltip.Content>
						</Tooltip.Root>
					{/if}
				</span>
				<span
					class={cn("truncate text-xs", attention || !server.responding ? "text-warning" : "text-muted-foreground")}
				>
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
	<Table.Cell class="text-right font-mono text-muted-foreground tabular-nums">
		{formatDuration(Date.now() - server.startedAt)}
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
					class={cn("h-4 w-14", leaking ? "text-warning" : "text-muted-foreground")}
				/>
				<span class={cn("w-16 text-right font-mono tabular-nums", leaking && "text-warning")}>
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
			{@render sortMenu()}
			<CleanUpButton {store} />
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
					<Table.Root class="grid [--cols:6rem_minmax(0,1fr)_8rem_7rem_5.5rem_10rem_5.5rem]">
						<Table.Header class="grid bg-muted/60">
							<Table.Row class="grid grid-cols-(--cols) items-center hover:bg-transparent">
								{@render head("Port", HashIcon, "pl-5", "port")}
								{@render head("Server", ServerIcon, "", "name")}
								{@render head("Framework", LayersIcon)}
								{@render head("Uptime", ClockIcon, "text-right", "uptime")}
								{@render head("CPU", CpuIcon, "text-right", "cpu")}
								{@render head("Memory", MemoryStickIcon, "text-right", "memory")}
								<Table.Head class="pr-5"><span class="sr-only">Actions</span></Table.Head>
							</Table.Row>
						</Table.Header>
						<Table.Body class="grid">
							{#each rows as row (row.key)}
								{@const server = row.server}
								<!-- A plain <tr> because animate: and transitions need an element, not a component. -->
								<tr
									data-slot="table-row"
									class={server
										? "group/row grid cursor-default grid-cols-(--cols) items-center border-b transition-colors hover:bg-muted/60 data-[state=selected]:bg-muted"
										: "grid grid-cols-(--cols) border-b bg-muted/30"}
									data-state={server && store.selectedPort === server.port ? "selected" : undefined}
									onclick={server ? () => showDetails(server) : undefined}
									animate:flip={{ duration: 220 * motion, easing: cubicOut }}
									in:fade={{ duration: 180 * motion, delay: 60 * motion }}
									out:fade={{ duration: 120 * motion }}
								>
									{#if server}
										{@render serverCells(server)}
									{:else if row.group}
										{@render groupCell(row.group)}
									{/if}
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
			<ScrollArea class="min-h-0 flex-1">
				<ServerDetails server={shown} {store} class="p-3" />
			</ScrollArea>
		{/if}
	</Drawer.Content>
</Drawer.Root>

<ConfirmDialog {store} />
<CleanUpDialog {store} />
<RenameDialog {store} />
