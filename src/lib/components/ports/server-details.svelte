<script lang="ts">
	import ChevronDownIcon from "@lucide/svelte/icons/chevron-down";
	import ChevronLeftIcon from "@lucide/svelte/icons/chevron-left";
	import CircleAlertIcon from "@lucide/svelte/icons/circle-alert";
	import CodeIcon from "@lucide/svelte/icons/code";
	import CopyIcon from "@lucide/svelte/icons/copy";
	import FolderOpenIcon from "@lucide/svelte/icons/folder-open";
	import PinIcon from "@lucide/svelte/icons/pin";
	import RotateCwIcon from "@lucide/svelte/icons/rotate-cw";
	import ShieldIcon from "@lucide/svelte/icons/shield";
	import SquareIcon from "@lucide/svelte/icons/square";
	import SquareTerminalIcon from "@lucide/svelte/icons/square-terminal";
	import WifiIcon from "@lucide/svelte/icons/wifi";
	import * as Alert from "$lib/components/ui/alert";
	import { Badge } from "$lib/components/ui/badge";
	import { Button } from "$lib/components/ui/button";
	import * as DropdownMenu from "$lib/components/ui/dropdown-menu";
	import * as Field from "$lib/components/ui/field";
	import { Separator } from "$lib/components/ui/separator";
	import { Switch } from "$lib/components/ui/switch";
	import {
		canStop,
		exposeFlag,
		formatBytes,
		formatBytesText,
		formatDuration,
		isMeasured,
		serverProcessId,
	} from "$lib/ports/format";
	import { fileManagerName } from "$lib/platform";
	import { hiddenPort, hiddenProcess, type PortsStore } from "$lib/ports/ports-store.svelte";
	import { editorLabel, preferences } from "$lib/ports/preferences.svelte";
	import type { Server } from "$lib/ports/types";
	import { settings } from "$lib/settings.svelte";
	import { cn } from "$lib/utils";
	import CpuBars from "./cpu-bars.svelte";
	import PortLabel from "./port-label.svelte";
	import QrCode from "./qr-code.svelte";
	import Sparkline from "./sparkline.svelte";

	let {
		server,
		store,
		onBack,
		class: className,
	}: { server: Server; store: PortsStore; onBack?: () => void; class?: string } = $props();

	const id = $props.id();

	const memory = $derived(formatBytes(server.memoryBytes));
	const leaking = $derived(server.status === "attention" && server.responding);
	const lanUrl = $derived(server.lanUrl?.replace(/^http:\/\//, ""));
	const idle = $derived(server.status === "idle");
	const alerts = $derived(settings.current.alerts);
	/** What the memory alerts watch for on this server; desktop apps get none. */
	const alertNote = $derived.by(() => {
		if (server.kind !== "dev" && server.kind !== "database") return null;
		const above = alerts.thresholdBytes === null ? null : `above ${formatBytesText(alerts.thresholdBytes)}`;
		if (alerts.enabled && above) return `Alerts if it keeps growing or goes ${above}`;
		if (alerts.enabled) return "Alerts if it keeps growing";
		return above && `Alerts ${above}`;
	});
	// Pins and protection cover every port of a project.
	const scope = $derived(server.folder ? "every port of this project" : `:${server.port} only`);

	const facts = $derived(
		[
			["Process", `${server.process} · ${serverProcessId(server)}`],
			["Framework", server.framework],
			["Branch", server.branch],
			["Folder", server.folder],
			["Listens on", server.addresses.join(", ")],
			["Command", server.command],
		].filter((fact): fact is [string, string] => Boolean(fact[1])),
	);
</script>

<article class={cn("flex flex-col gap-5", className)} aria-label="{server.name} on port {server.port}">
	<header class="flex flex-col gap-1">
		<div class="relative flex min-h-8 items-center justify-center">
			{#if onBack}
				<Button variant="ghost" size="icon-sm" class="absolute left-0" aria-label="Back" onclick={onBack}>
					<ChevronLeftIcon />
				</Button>
			{/if}
			<h2 class="flex min-w-0 items-center gap-1.5 px-10 text-sm font-semibold">
				{#if store.isPinned(server)}
					<PinIcon class="size-3.5 shrink-0 text-muted-foreground" aria-label="Pinned" />
				{/if}
				<span class="truncate">{server.name}</span>
			</h2>
		</div>
		<PortLabel port={server.port} class="text-4xl" />
		<p class="text-sm text-muted-foreground">
			{idle ? "Idle for" : "Running for"}
			{formatDuration(Date.now() - (idle ? server.lastActiveAt : server.startedAt))}
		</p>
	</header>

	{#if !server.responding}
		<Alert.Root class="text-warning">
			<CircleAlertIcon />
			<Alert.Title>Not responding</Alert.Title>
			<Alert.Description>
				It holds :{server.port} but didn't answer the last check. It may be stuck in a loop{server.restartable
					? "; restarting it usually helps"
					: ""}.
			</Alert.Description>
		</Alert.Root>
	{/if}

	<div class="flex flex-col gap-2">
		<div class="flex gap-2">
			<Button size="lg" class="flex-1" onclick={() => store.open(server)}>
				Open port {server.port}
			</Button>
			<Button variant="secondary" size="icon-lg" aria-label="Copy URL" onclick={() => store.copyUrl(server)}>
				<CopyIcon />
			</Button>
		</div>
		{#if server.folder}
			<div class="grid grid-cols-3 gap-2">
				<Button
					variant="secondary"
					size="sm"
					disabled={store.isBusy(server, "editor")}
					onclick={() => store.openInEditor(server)}
				>
					<CodeIcon data-icon="inline-start" />
					{editorLabel(preferences.editor.current)}
				</Button>
				<Button
					variant="secondary"
					size="sm"
					disabled={store.isBusy(server, "terminal")}
					onclick={() => store.openInTerminal(server)}
				>
					<SquareTerminalIcon data-icon="inline-start" />
					Terminal
				</Button>
				<Button
					variant="secondary"
					size="sm"
					disabled={store.isBusy(server, "folder")}
					onclick={() => store.revealFolder(server)}
				>
					<FolderOpenIcon data-icon="inline-start" />
					{fileManagerName}
				</Button>
			</div>
		{/if}
		{#if canStop(server)}
			<div class={cn("grid gap-2", server.restartable && "grid-cols-2")}>
				{#if server.restartable}
					<Button
						variant="secondary"
						size="sm"
						disabled={store.isBusy(server, "control")}
						onclick={() => store.request("restart", server)}
					>
						<RotateCwIcon data-icon="inline-start" />
						Restart
					</Button>
				{/if}
				<Button
					variant="destructive"
					size="sm"
					disabled={store.isBusy(server, "control")}
					onclick={() => store.request("stop", server)}
				>
					<SquareIcon data-icon="inline-start" class="fill-current" />
					Stop
				</Button>
			</div>
		{/if}
	</div>

	<Separator />

	<dl class="grid grid-cols-[5.5rem_1fr] gap-x-4 gap-y-2.5 text-sm">
		{#each facts as [label, value] (label)}
			<dt class="text-muted-foreground">{label}</dt>
			<dd class={cn("min-w-0 truncate", label === "Command" && "font-mono text-xs leading-5")}>{value}</dd>
		{/each}
		{#if server.exposed}
			<dt class="text-muted-foreground">Network</dt>
			<dd><Badge variant="secondary"><WifiIcon data-icon="inline-start" />Reachable from other devices</Badge></dd>
		{/if}
		{#if server.protected}
			<dt class="text-muted-foreground">Clean up</dt>
			<dd><Badge variant="secondary"><ShieldIcon data-icon="inline-start" />Protected</Badge></dd>
		{/if}
	</dl>

	<!-- Only dev servers speak HTTP for sure; a database on 0.0.0.0 has no page to open. -->
	{#if server.kind === "dev" && server.lanUrl && lanUrl}
		<Separator />
		<section class="flex items-center gap-4" aria-label="Open on another device">
			<QrCode value={server.lanUrl} class="size-28 shrink-0" />
			<div class="flex min-w-0 flex-col gap-2">
				<p class="text-sm font-medium">Open on your phone</p>
				<p class="text-xs text-muted-foreground">Scan the code with a device on the same network.</p>
				<Button
					variant="secondary"
					size="sm"
					class="justify-start"
					aria-label="Copy {server.lanUrl}"
					onclick={() => store.copyUrl(server, server.lanUrl)}
				>
					<CopyIcon data-icon="inline-start" />
					<span class="truncate font-mono text-xs">{lanUrl}</span>
				</Button>
			</div>
		</section>
	{:else if server.kind === "dev" && server.exposed}
		<Separator />
		<p class="text-xs text-muted-foreground">
			Other devices can reach it, but this machine isn't on a network right now.
		</p>
	{:else if server.kind === "dev"}
		<Separator />
		<p class="text-xs text-muted-foreground">
			Only this machine can open it. To open it on your phone, start it with
			<code class="font-mono text-foreground">{exposeFlag(server) ?? "--host 0.0.0.0"}</code>
			so it listens on the network too.
		</p>
	{/if}

	{#if isMeasured(server)}
		<Separator />

		<section class="flex flex-col gap-2" aria-label="Memory">
			<div class="flex items-baseline justify-between">
				<p class="text-sm text-muted-foreground">
					Memory
					<span class={cn("font-mono text-foreground tabular-nums", leaking && "text-warning")}>
						{memory.value}
						{memory.unit}
					</span>
				</p>
				<span class="font-mono text-xs text-muted-foreground">10 min</span>
			</div>
			<Sparkline
				values={server.memoryHistory}
				fromZero
				area
				threshold={alerts.thresholdBytes ?? undefined}
				class={cn("h-20 w-full", leaking ? "text-warning" : "text-foreground")}
			/>
			{#if alertNote}
				<p class="text-xs text-muted-foreground">{alertNote}</p>
			{/if}
		</section>

		<section class="flex flex-col gap-2" aria-label="CPU">
			<p class="text-sm text-muted-foreground">
				CPU <span class="font-mono text-foreground tabular-nums">{Math.round(server.cpuPercent)}%</span>
			</p>
			<CpuBars values={server.cpuHistory} class="h-14 w-full" />
		</section>
	{/if}

	<Separator />

	<Field.Group class="gap-4" aria-label="In the list">
		<Field.Field orientation="horizontal">
			<Field.Content>
				<Field.Label for="{id}-pin">Pin to top</Field.Label>
				<Field.Description>Applies to {scope}.</Field.Description>
			</Field.Content>
			<Switch id="{id}-pin" bind:checked={() => store.isPinned(server), () => store.togglePin(server)} />
		</Field.Field>
		<Field.Field orientation="horizontal">
			<Field.Content>
				<Field.Label for="{id}-protect">Protect from clean up</Field.Label>
				<Field.Description>Applies to {scope}.</Field.Description>
			</Field.Content>
			<Switch id="{id}-protect" bind:checked={() => server.protected, (value) => store.setProtected(server, value)} />
		</Field.Field>
		<div class="grid grid-cols-2 gap-2">
			<Button variant="secondary" size="sm" onclick={() => (store.renaming = server)}>Rename…</Button>
			<DropdownMenu.Root>
				<DropdownMenu.Trigger>
					{#snippet child({ props })}
						<Button {...props} variant="secondary" size="sm">
							Hide
							<ChevronDownIcon data-icon="inline-end" />
						</Button>
					{/snippet}
				</DropdownMenu.Trigger>
				<DropdownMenu.Content align="end" class="w-56">
					<DropdownMenu.Item onSelect={() => store.hide(hiddenPort(server.port))}>
						Hide :{server.port}
					</DropdownMenu.Item>
					<DropdownMenu.Item onSelect={() => store.hide(hiddenProcess(server.process))}>
						Hide every port of {server.process}
					</DropdownMenu.Item>
				</DropdownMenu.Content>
			</DropdownMenu.Root>
		</div>
	</Field.Group>
</article>
