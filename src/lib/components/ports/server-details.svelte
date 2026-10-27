<script lang="ts">
	import ChevronLeftIcon from "@lucide/svelte/icons/chevron-left";
	import CodeIcon from "@lucide/svelte/icons/code";
	import CopyIcon from "@lucide/svelte/icons/copy";
	import FolderOpenIcon from "@lucide/svelte/icons/folder-open";
	import SquareTerminalIcon from "@lucide/svelte/icons/square-terminal";
	import RotateCwIcon from "@lucide/svelte/icons/rotate-cw";
	import ShieldIcon from "@lucide/svelte/icons/shield";
	import SquareIcon from "@lucide/svelte/icons/square";
	import { Badge } from "$lib/components/ui/badge";
	import { Button } from "$lib/components/ui/button";
	import { Separator } from "$lib/components/ui/separator";
	import { formatBytes, formatBytesText, formatDuration } from "$lib/ports/format";
	import { fileManagerName } from "$lib/platform";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";
	import { editorLabel, preferences } from "$lib/ports/preferences.svelte";
	import type { Server } from "$lib/ports/types";
	import { cn } from "$lib/utils";
	import CpuBars from "./cpu-bars.svelte";
	import PortLabel from "./port-label.svelte";
	import Sparkline from "./sparkline.svelte";

	const LEAK_THRESHOLD_BYTES = 2 * 1024 ** 3;

	let {
		server,
		store,
		onBack,
		class: className,
	}: { server: Server; store: PortsStore; onBack?: () => void; class?: string } = $props();

	const memory = $derived(formatBytes(server.memoryBytes));
	const attention = $derived(server.status === "attention");

	const facts = $derived(
		[
			["Process", `${server.process} · PID ${server.pid}`],
			["Framework", server.framework],
			["Branch", server.branch],
			["Folder", server.folder],
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
			<h2 class="truncate text-sm font-semibold">{server.name}</h2>
		</div>
		<div class="flex items-center justify-between gap-3">
			<PortLabel port={server.port} class="text-4xl" />
			<div class="flex items-center gap-2">
				<span class="text-sm text-muted-foreground">
					{server.status === "idle" ? "Idle for" : "Running for"}
					{formatDuration(Date.now() - (server.status === "idle" ? server.lastActiveAt : server.startedAt))}
				</span>
				<Button variant="secondary" size="icon-sm" aria-label="Restart" onclick={() => store.restart(server)}>
					<RotateCwIcon />
				</Button>
				<Button variant="destructive" size="icon-sm" aria-label="Stop" onclick={() => store.requestStop(server)}>
					<SquareIcon class="fill-current" />
				</Button>
			</div>
		</div>
	</header>

	<div class="flex flex-col gap-2">
		<div class="flex gap-2">
			<Button size="lg" class="flex-1" onclick={() => store.open(server)}>
				Open localhost:{server.port}
			</Button>
			<Button variant="secondary" size="icon-lg" aria-label="Copy URL" onclick={() => store.copyUrl(server)}>
				<CopyIcon />
			</Button>
		</div>
		{#if server.folder}
			<div class="grid grid-cols-3 gap-2">
				<Button variant="secondary" size="sm" onclick={() => store.openInEditor(server)}>
					<CodeIcon data-icon="inline-start" />
					{editorLabel(preferences.editor.current)}
				</Button>
				<Button variant="secondary" size="sm" onclick={() => store.openInTerminal(server)}>
					<SquareTerminalIcon data-icon="inline-start" />
					Terminal
				</Button>
				<Button variant="secondary" size="sm" onclick={() => store.revealFolder(server)}>
					<FolderOpenIcon data-icon="inline-start" />
					{fileManagerName}
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
		{#if server.protected}
			<dt class="text-muted-foreground">Clean up</dt>
			<dd><Badge variant="secondary"><ShieldIcon data-icon="inline-start" />Protected</Badge></dd>
		{/if}
	</dl>

	<Separator />

	<section class="flex flex-col gap-2" aria-label="Memory">
		<div class="flex items-baseline justify-between">
			<p class="text-sm text-muted-foreground">
				Memory
				<span class={cn("font-mono text-foreground tabular-nums", attention && "text-warning")}>
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
			threshold={LEAK_THRESHOLD_BYTES}
			class={cn("h-20 w-full", attention ? "text-warning" : "text-foreground")}
		/>
		<p class="text-xs text-muted-foreground">
			Warns above {formatBytesText(LEAK_THRESHOLD_BYTES)}
		</p>
	</section>

	<section class="flex flex-col gap-2" aria-label="CPU">
		<p class="text-sm text-muted-foreground">
			CPU <span class="font-mono text-foreground tabular-nums">{Math.round(server.cpuPercent)}%</span>
		</p>
		<CpuBars values={server.cpuHistory} class="h-14 w-full" />
	</section>
</article>
