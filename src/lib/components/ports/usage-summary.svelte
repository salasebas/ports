<script lang="ts">
	import * as ToggleGroup from "$lib/components/ui/toggle-group";
	import { formatBytes, formatBytesText, formatPercent, portColor } from "$lib/ports/format";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";
	import { preferences, type Metric } from "$lib/ports/preferences.svelte";
	import type { Server } from "$lib/ports/types";
	import { cn } from "$lib/utils";

	let { store, class: className }: { store: PortsStore; class?: string } = $props();

	const metrics: { value: Metric; label: string }[] = [
		{ value: "memory", label: "Memory" },
		{ value: "cpu", label: "CPU" },
	];

	/** What the summary shows for the chosen metric. Shares are fractions of the whole machine. */
	const view = $derived.by(() => {
		const { system } = store;
		if (preferences.metric.current === "cpu") {
			return {
				label: "CPU",
				total: { value: String(Math.round(store.cpuPercent)), unit: "%" },
				share: (server: Server) => server.cpuPercent / 100,
				otherAppsShare: system.otherAppsCpuPercent / 100,
				servers: formatPercent(store.cpuPercent),
				otherApps: formatPercent(system.otherAppsCpuPercent),
				rest: ["Idle", formatPercent(Math.max(100 - store.cpuPercent - system.otherAppsCpuPercent, 0))],
			};
		}
		const freeBytes = Math.max(system.totalBytes - system.otherAppsBytes - store.serversBytes, 0);
		return {
			label: "Memory",
			total: formatBytes(store.serversBytes),
			share: (server: Server) => server.memoryBytes / system.totalBytes,
			otherAppsShare: system.otherAppsBytes / system.totalBytes,
			servers: formatBytesText(store.serversBytes),
			otherApps: formatBytesText(system.otherAppsBytes),
			rest: ["Free", `${formatBytesText(freeBytes)} of ${formatBytesText(system.totalBytes)}`],
		};
	});
</script>

<section class={cn("flex flex-col gap-3", className)} aria-label="{view.label} used by servers">
	<div class="flex items-center justify-between gap-4">
		<p class="flex items-baseline gap-1.5">
			<span class="font-mono text-4xl font-medium tracking-tight tabular-nums">{view.total.value}</span>
			<span class="text-sm text-muted-foreground">{view.total.unit}</span>
		</p>
		<ToggleGroup.Root
			type="single"
			variant="outline"
			size="sm"
			aria-label="Metric"
			bind:value={
				() => preferences.metric.current,
				(value) => {
					if (value) preferences.metric.current = value as Metric;
				}
			}
		>
			{#each metrics as metric (metric.value)}
				<ToggleGroup.Item value={metric.value} class="px-2.5">{metric.label}</ToggleGroup.Item>
			{/each}
		</ToggleGroup.Root>
	</div>

	<div class="flex h-1.5 gap-0.5 overflow-hidden rounded-full bg-muted">
		{#each store.servers as server (server.port)}
			<div
				class="h-full rounded-full transition-[width] duration-300 ease-out"
				style:width="{view.share(server) * 100}%"
				style:background-color={portColor(server.port)}
			></div>
		{/each}
		<div
			class="h-full rounded-full bg-muted-foreground/40 transition-[width] duration-300 ease-out"
			style:width="{view.otherAppsShare * 100}%"
		></div>
	</div>

	<ul class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
		<li class="flex items-center gap-1.5">
			<span class="size-2 rounded-sm bg-foreground"></span>
			Servers <span class="font-mono tabular-nums">{view.servers}</span>
		</li>
		<li class="flex items-center gap-1.5">
			<span class="size-2 rounded-sm bg-muted-foreground/40"></span>
			Other apps <span class="font-mono tabular-nums">{view.otherApps}</span>
		</li>
		<li class="flex items-center gap-1.5">
			<span class="size-2 rounded-sm bg-muted"></span>
			{view.rest[0]} <span class="font-mono tabular-nums">{view.rest[1]}</span>
		</li>
	</ul>
</section>
