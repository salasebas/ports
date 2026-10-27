<script lang="ts">
	import ArrowUpRightIcon from "@lucide/svelte/icons/arrow-up-right";
	import SquareIcon from "@lucide/svelte/icons/square";
	import { Button } from "$lib/components/ui/button";
	import { canStop, formatBytes, isMeasured, serverActivity } from "$lib/ports/format";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";
	import { preferences } from "$lib/ports/preferences.svelte";
	import type { Server } from "$lib/ports/types";
	import { cn } from "$lib/utils";
	import PortLabel from "./port-label.svelte";
	import Sparkline from "./sparkline.svelte";

	let { server, store }: { server: Server; store: PortsStore } = $props();

	const cpu = $derived(preferences.metric.current === "cpu");
	const reading = $derived.by(() => {
		if (!isMeasured(server)) return { value: "–", unit: "" };
		return cpu ? { value: String(Math.round(server.cpuPercent)), unit: "%" } : formatBytes(server.memoryBytes);
	});
	const attention = $derived(server.status === "attention");
	const detail = $derived(server.branch ?? server.framework ?? server.process);
</script>

<li
	class={cn(
		"group/row relative flex items-center gap-3 rounded-xl px-3 py-2.5 transition-[background-color,opacity] duration-150",
		"focus-within:bg-muted/70 hover:bg-muted/70 has-[>button:active]:bg-muted",
		server.status === "idle" && "opacity-60 focus-within:opacity-100 hover:opacity-100",
	)}
>
	<button
		type="button"
		class="absolute inset-0 rounded-xl outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
		aria-label="Details for {server.name} on port {server.port}"
		onclick={() => store.select(server.port)}
	></button>

	<PortLabel port={server.port} class="w-14 shrink-0 text-[15px]" />

	<div class="pointer-events-none flex min-w-0 flex-1 flex-col">
		<span class="truncate text-sm font-medium">{server.name}</span>
		<span class="truncate text-xs text-muted-foreground">
			{#if attention}
				<span class="text-warning">{serverActivity(server)}</span>
			{:else}
				{detail} · {serverActivity(server)}
			{/if}
		</span>
	</div>

	<div class="grid w-16 shrink-0 place-items-center *:col-start-1 *:row-start-1">
		<Sparkline
			values={cpu ? server.cpuHistory : server.memoryHistory}
			max={cpu ? 100 : undefined}
			class={cn(
				"pointer-events-none h-5 w-full transition-opacity duration-150 will-change-[opacity] group-focus-within/row:opacity-0 group-hover/row:opacity-0",
				attention ? "text-warning" : "text-muted-foreground",
			)}
		/>
		<div
			class="pointer-events-none relative flex gap-0.5 opacity-0 transition-opacity duration-150 will-change-[opacity] group-focus-within/row:pointer-events-auto group-focus-within/row:opacity-100 group-hover/row:pointer-events-auto group-hover/row:opacity-100"
		>
			<Button
				variant="ghost"
				size="icon-sm"
				aria-label="Open localhost:{server.port}"
				onclick={() => store.open(server)}
			>
				<ArrowUpRightIcon />
			</Button>
			{#if canStop(server)}
				<Button
					variant="ghost"
					size="icon-sm"
					aria-label="Stop {server.name}"
					disabled={store.isBusy(server, "control")}
					onclick={() => store.request("stop", server)}
				>
					<SquareIcon class="fill-current" />
				</Button>
			{/if}
		</div>
	</div>

	<p
		class={cn(
			"pointer-events-none w-16 shrink-0 text-right font-mono text-sm tabular-nums",
			attention && "text-warning",
		)}
	>
		{reading.value} <span class="text-xs text-muted-foreground">{reading.unit}</span>
	</p>
</li>
