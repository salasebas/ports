<script lang="ts">
	import { onMount, type Component } from "svelte";
	import { goto } from "$app/navigation";
	import { resolve } from "$app/paths";
	import { page } from "$app/state";
	import { flip } from "svelte/animate";
	import { cubicOut } from "svelte/easing";
	import { prefersReducedMotion } from "svelte/motion";
	import { fade } from "svelte/transition";
	import ArrowDownIcon from "@lucide/svelte/icons/arrow-down";
	import ArrowUpDownIcon from "@lucide/svelte/icons/arrow-up-down";
	import ArrowUpIcon from "@lucide/svelte/icons/arrow-up";
	import ArrowUpRightIcon from "@lucide/svelte/icons/arrow-up-right";
	import ChevronDownIcon from "@lucide/svelte/icons/chevron-down";
	import ChevronsDownUpIcon from "@lucide/svelte/icons/chevrons-down-up";
	import ChevronsUpDownIcon from "@lucide/svelte/icons/chevrons-up-down";
	import ChevronRightIcon from "@lucide/svelte/icons/chevron-right";
	import ClockIcon from "@lucide/svelte/icons/clock";
	import CpuIcon from "@lucide/svelte/icons/cpu";
	import FolderIcon from "@lucide/svelte/icons/folder";
	import HashIcon from "@lucide/svelte/icons/hash";
	import LayersIcon from "@lucide/svelte/icons/layers";
	import MemoryStickIcon from "@lucide/svelte/icons/memory-stick";
	import PinIcon from "@lucide/svelte/icons/pin";
	import RefreshCwIcon from "@lucide/svelte/icons/refresh-cw";
	import ServerColumnIcon from "@lucide/svelte/icons/server";
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
	import ServerIcon from "$lib/components/ports/server-icon.svelte";
	import RenameDialog from "$lib/components/ports/rename-dialog.svelte";
	import ServerFilter from "$lib/components/ports/server-filter.svelte";
	import ServerSearch from "$lib/components/ports/server-search.svelte";
	import SavedSection from "$lib/components/ports/saved-section.svelte";
	import ProjectDetails from "$lib/components/ports/project-details.svelte";
	import SettingsButton from "$lib/components/ports/settings-button.svelte";
	import Sparkline from "$lib/components/ports/sparkline.svelte";
	import ConfirmDialog from "$lib/components/ports/confirm-dialog.svelte";
	import UsageSummary from "$lib/components/ports/usage-summary.svelte";
	import {
		placeLabel,
		canStop,
		confirmedFramework,
		formatBytesText,
		formatDuration,
		isMeasured,
		serverActivity,
		serverProcessId,
	} from "$lib/ports/format";
	import { groupKeys, listRows, toggleCollapsed, type GroupRow } from "$lib/ports/grouping";
	import { ListKeyboard } from "$lib/ports/list-keyboard.svelte";
	import { PortsStore } from "$lib/ports/ports-store.svelte";
	import { preferences, sortKeys, type SortKey } from "$lib/ports/preferences.svelte";
	import type { Server, ServerStatus } from "$lib/ports/types";
	import type { SavedProject } from "$lib/ports/projects";
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

	let lastSelected = $state.raw<Server | null>(null);
	let lastProject = $state.raw<SavedProject | null>(null);
	let lastKind = $state<"server" | "project">("server");
	const shown = $derived(store.selected ?? lastSelected);
	const shownProject = $derived(store.selectedProject ?? lastProject);

	function showDetails(server: Server) {
		lastSelected = server;
		lastKind = "server";
		keyboard.choose(server.id);
		store.select(server.id);
	}

	function keepOpenForToasts(event: Event) {
		if (event.target instanceof Element && event.target.closest("[data-sonner-toaster]")) event.preventDefault();
	}

	function showProject(project: SavedProject) {
		lastProject = project;
		lastKind = "project";
		store.selectProject(project.id);
	}

	$effect(() => {
		const id = page.url.searchParams.get("details");
		const server = store.servers.find((s) => s.id === id);
		if (!server) return;
		showDetails(server);
		void goto(resolve("/"), { replaceState: true, noScroll: true, keepFocus: true });
	});

	function sortBy(key: SortKey) {
		const sort = preferences.sort.current;
		const descending =
			sort.key === key ? !sort.descending : (sortKeys.find((k) => k.value === key)?.descending ?? false);
		preferences.sort.current = { key, descending };
	}

	const rows = $derived(listRows(store.filtered, preferences.groupBy.current, preferences.collapsedGroups.current));

	let search = $state<HTMLInputElement | null>(null);
	const keyboard = new ListKeyboard({
		store,
		rows: () => rows.flatMap((row) => (row.kind === "server" ? [row.server] : [])),
		search: () => search,
		details: showDetails,
	});

	function restoreFocus(event: Event) {
		event.preventDefault();
		keyboard.restore(lastKind === "server" ? (lastSelected?.id ?? null) : null);
	}

	function toggleGroup(key: string) {
		preferences.collapsedGroups.current = toggleCollapsed(preferences.collapsedGroups.current, key);
	}

	const groups = $derived(groupKeys(store.filtered, preferences.groupBy.current));
	const allCollapsed = $derived(groups.every((key) => preferences.collapsedGroups.current.includes(key)));

	function toggleAllGroups() {
		const others = preferences.collapsedGroups.current.filter((key) => !groups.includes(key));
		preferences.collapsedGroups.current = allCollapsed ? others : [...others, ...groups];
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
			<DropdownMenu.Group>
				<DropdownMenu.GroupHeading>Group by</DropdownMenu.GroupHeading>
				<DropdownMenu.RadioGroup bind:value={preferences.groupBy.current}>
					<DropdownMenu.RadioItem value="flat" closeOnSelect={false}>Flat list</DropdownMenu.RadioItem>
					<DropdownMenu.RadioItem value="project" closeOnSelect={false}>Project</DropdownMenu.RadioItem>
					<DropdownMenu.RadioItem value="repository" closeOnSelect={false}>Repository</DropdownMenu.RadioItem>
				</DropdownMenu.RadioGroup>
			</DropdownMenu.Group>
		</DropdownMenu.Content>
	</DropdownMenu.Root>
{/snippet}

{#snippet groupCell(row: GroupRow)}
	{@const collapsed = preferences.collapsedGroups.current.includes(row.key)}
	{@const Chevron = collapsed ? ChevronRightIcon : ChevronDownIcon}
	{@const place = [row.detail, row.path].filter(Boolean).join(" · ")}
	{@const tall = preferences.groupHeaders.current === "tall"}
	<td class={cn("col-span-full flex min-w-0 items-center px-5 text-xs text-muted-foreground", tall ? "py-3" : "py-1")}>
		<button
			type="button"
			class={cn(
				"flex min-w-0 flex-1 items-center rounded-md text-start",
				tall ? "gap-3" : "gap-2 py-1",
				row.depth === 1 && "ps-6",
			)}
			aria-expanded={!collapsed}
			onclick={(event: MouseEvent) => {
				event.stopPropagation();
				toggleGroup(row.key);
			}}
		>
			<Chevron class={cn("shrink-0 opacity-70", tall ? "size-4" : "size-3.5")} />
			<FolderIcon class={cn("shrink-0 opacity-70", tall ? "size-4" : "size-3.5")} />
			{#if tall}
				<span class="flex min-h-9.5 min-w-0 flex-col justify-center gap-0.5">
					<span class="truncate text-sm font-medium text-foreground">{row.name}</span>
					{#if place}
						<span class="truncate">{place}</span>
					{/if}
				</span>
			{:else}
				<span class="shrink-0 font-medium text-foreground">{row.name}</span>
				{#if place}
					<span class="truncate">{place}</span>
				{/if}
			{/if}
			<span class="ms-auto shrink-0 font-mono tabular-nums">
				{row.count} · {formatBytesText(row.memoryBytes)} · {Math.round(row.cpuPercent)}%
			</span>
		</button>
	</td>
{/snippet}

{#snippet serverCells(server: Server)}
	{@const attention = server.status === "attention"}
	{@const leaking = attention && server.responding}
	{@const place = placeLabel(server)}
	<Table.Cell class="py-3.5 pl-5">
		<PortLabel port={server.port} class="text-[15px]" />
	</Table.Cell>
	<Table.Cell class="min-w-0">
		<div class="flex items-center gap-3">
			<span class={cn("size-2 shrink-0 rounded-full", statusDot[server.status])} title={server.status}></span>
			<div class="flex min-w-0 flex-col gap-0.5">
				<span class="flex items-center gap-1.5 font-medium">
					<ServerIcon {server} class="size-4" />
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
					{place ? `${place} · ` : ""}{serverActivity(server)}
				</span>
			</div>
		</div>
	</Table.Cell>
	<Table.Cell class="min-w-0">
		{@const runs = confirmedFramework(server) ?? server.process}
		<div class="flex min-w-0 flex-col gap-0.5" title={runs}>
			<span class="truncate">{runs}</span>
			<span class="truncate font-mono text-xs text-muted-foreground">{serverProcessId(server)}</span>
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
			{@render action(`Open port ${server.port}`, ArrowUpRightIcon, () => store.open(server))}
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

<svelte:window bind:scrollY onkeydown={keyboard.onkeydown} />

<div class="flex min-h-screen flex-col">
	<header
		class={cn(
			"sticky top-0 z-10 border-b bg-background/80 backdrop-blur-xl backdrop-saturate-150 transition-colors duration-200",
			scrollY > 0 ? "border-border" : "border-transparent",
		)}
	>
		<div class="mx-auto flex w-full max-w-5xl flex-wrap items-center gap-3 px-6 py-3">
			<div class="flex items-center gap-2">
				<h1 class="text-lg font-semibold tracking-tight">Ports</h1>
				<Badge variant="secondary">{store.shown.length}</Badge>
			</div>
			<div class="ml-auto flex items-center gap-2">
				<ServerSearch bind:value={store.query} bind:ref={search} class="w-64" />
				<ServerFilter {store} />
				{@render sortMenu()}
				{#if groups.length}
					{@render action(
						allCollapsed ? "Expand all groups" : "Collapse all groups",
						allCollapsed ? ChevronsUpDownIcon : ChevronsDownUpIcon,
						toggleAllGroups,
					)}
				{/if}
				<CleanUpButton {store} />
				{@render action("Refresh", RefreshCwIcon, store.refresh)}
				<SettingsButton />
			</div>
		</div>
	</header>

	<main class="mx-auto flex w-full max-w-5xl flex-1 flex-col gap-6 p-6">
		<Card.Root>
			<Card.Content>
				<UsageSummary {store} />
			</Card.Content>
		</Card.Root>

		{#if store.servers.length}
			<div
				class="overflow-hidden rounded-2xl border bg-card transition-[height] duration-200 ease-out motion-reduce:transition-none"
				style:height={tableHeight ? `${tableHeight + 2}px` : undefined}
			>
				<div bind:clientHeight={tableHeight} class="[&>[data-slot=table-container]]:overflow-visible">
					<Table.Root class="grid [--cols:6rem_minmax(0,1fr)_8rem_7rem_5.5rem_10rem_5.5rem]">
						<Table.Header class="grid bg-muted/60">
							<Table.Row class="grid grid-cols-(--cols) items-center hover:bg-transparent">
								{@render head("Port", HashIcon, "pl-5", "port")}
								{@render head("Server", ServerColumnIcon, "", "name")}
								{@render head("Framework", LayersIcon)}
								{@render head("Uptime", ClockIcon, "text-right", "uptime")}
								{@render head("CPU", CpuIcon, "text-right", "cpu")}
								{@render head("Memory", MemoryStickIcon, "text-right", "memory")}
								<Table.Head class="pr-5"><span class="sr-only">Actions</span></Table.Head>
							</Table.Row>
						</Table.Header>
						<Table.Body class="grid">
							{#each rows as row (row.key)}
								{@const server = row.kind === "server" ? row.server : undefined}
								<!-- A plain <tr> because animate: and transitions need an element, not a component. -->
								<tr
									data-slot="table-row"
									class={server
										? "group/row grid cursor-default grid-cols-(--cols) items-center border-b transition-colors outline-none hover:bg-muted/60 focus-visible:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-inset data-[state=selected]:bg-muted"
										: "grid grid-cols-(--cols) border-b bg-muted/30"}
									data-state={server && store.selectedId === server.id ? "selected" : undefined}
									data-row-id={server?.id}
									tabindex={server ? (keyboard.activeId === server.id ? 0 : -1) : undefined}
									aria-label={server ? `${server.name} on port ${server.port}` : undefined}
									onclick={server ? () => showDetails(server) : undefined}
									onfocus={server ? () => keyboard.choose(server.id) : undefined}
									animate:flip={{ duration: 220 * motion, easing: cubicOut }}
									in:fade={{ duration: 180 * motion, delay: 60 * motion }}
									out:fade={{ duration: 120 * motion }}
								>
									{#if server}
										{@render serverCells(server)}
									{:else if row.kind !== "server"}
										{@render groupCell(row)}
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
		<SavedSection {store} onOpen={showProject} />
	</main>
</div>

<Drawer.Root
	direction="right"
	bind:open={
		() => store.selected !== null || store.selectedProject !== null,
		(open) => {
			if (!open) {
				store.select(null);
				store.selectProject(null);
			}
		}
	}
>
	<Drawer.Content
		class="w-[26rem] sm:max-w-[26rem]"
		onInteractOutside={keepOpenForToasts}
		onCloseAutoFocus={restoreFocus}
	>
		<Drawer.Title class="sr-only">Server details</Drawer.Title>
		<Drawer.Close>
			{#snippet child({ props })}
				<Button {...props} variant="ghost" size="icon-sm" class="absolute top-6 right-6" aria-label="Close">
					<XIcon />
				</Button>
			{/snippet}
		</Drawer.Close>
		{#if lastKind === "project" && shownProject && !store.selected}
			<ScrollArea class="min-h-0 flex-1">
				<ProjectDetails project={shownProject} {store} class="p-3" />
			</ScrollArea>
		{:else if shown}
			<ScrollArea class="min-h-0 flex-1">
				<ServerDetails server={shown} {store} class="p-3" />
			</ScrollArea>
		{/if}
	</Drawer.Content>
</Drawer.Root>

<ConfirmDialog {store} />
<CleanUpDialog {store} />
<RenameDialog {store} />
