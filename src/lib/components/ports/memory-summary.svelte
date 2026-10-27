<script lang="ts">
	import { formatBytes, formatBytesText, portColor } from "$lib/ports/format";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";
	import { cn } from "$lib/utils";

	let { store, class: className }: { store: PortsStore; class?: string } = $props();

	const total = $derived(formatBytes(store.serversBytes));
	const freeBytes = $derived(
		Math.max(store.system.totalBytes - store.system.otherAppsBytes - store.serversBytes, 0),
	);
	const share = (bytes: number) => `${(bytes / store.system.totalBytes) * 100}%`;
</script>

<section class={cn("flex flex-col gap-3", className)} aria-label="Memory used by servers">
	<div class="flex items-baseline justify-between gap-4">
		<p class="flex items-baseline gap-1.5">
			<span class="font-mono text-4xl font-medium tracking-tight tabular-nums">{total.value}</span>
			<span class="text-muted-foreground text-sm">{total.unit}</span>
		</p>
		<p class="text-muted-foreground text-sm">
			CPU (servers) <span class="text-foreground font-mono tabular-nums">{Math.round(store.cpuPercent)}%</span>
		</p>
	</div>

	<div class="bg-muted flex h-1.5 gap-0.5 overflow-hidden rounded-full">
		{#each store.servers as server (server.port)}
			<div
				class="h-full rounded-full transition-[width] duration-300 ease-out"
				style:width={share(server.memoryBytes)}
				style:background-color={portColor(server.port)}
			></div>
		{/each}
		<div
			class="bg-muted-foreground/40 h-full rounded-full"
			style:width={share(store.system.otherAppsBytes)}
		></div>
	</div>

	<ul class="text-muted-foreground flex flex-wrap gap-x-4 gap-y-1 text-xs">
		<li class="flex items-center gap-1.5">
			<span class="bg-foreground size-2 rounded-sm"></span>
			Servers <span class="font-mono tabular-nums">{formatBytesText(store.serversBytes)}</span>
		</li>
		<li class="flex items-center gap-1.5">
			<span class="bg-muted-foreground/40 size-2 rounded-sm"></span>
			Other apps <span class="font-mono tabular-nums">{formatBytesText(store.system.otherAppsBytes)}</span>
		</li>
		<li class="flex items-center gap-1.5">
			<span class="bg-muted size-2 rounded-sm"></span>
			Free
			<span class="font-mono tabular-nums">
				{formatBytesText(freeBytes)} of {formatBytesText(store.system.totalBytes)}
			</span>
		</li>
	</ul>
</section>
