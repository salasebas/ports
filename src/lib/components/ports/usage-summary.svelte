<script lang="ts">
	import * as ToggleGroup from "$lib/components/ui/toggle-group";
	import { formatBytes, formatBytesText, formatPercent, portColor } from "$lib/ports/format";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";
	import { preferences, type Metric } from "$lib/ports/preferences.svelte";

	import { cn } from "$lib/utils";

	let { store, class: className }: { store: PortsStore; class?: string } = $props();

	/** The biggest servers get their own color and legend entry; the rest share one. */
	const NAMED = 3;

	const metrics: { value: Metric; label: string }[] = [
		{ value: "memory", label: "Memory" },
		{ value: "cpu", label: "CPU" },
	];

	/** What the summary shows for the chosen metric. Shares are fractions of the whole machine. */
	const view = $derived.by(() => {
		const { system } = store;
		const { usage } = store;
		const cpu = preferences.metric.current === "cpu";
		// Empty until the first snapshot arrives.
		const ofMemory = (bytes: number) => (system.totalBytes ? bytes / system.totalBytes : 0);
		const amount = (fraction: number) =>
			cpu ? formatPercent(fraction * 100) : formatBytesText(fraction * system.totalBytes);

		const ranked = store.allocations
			.map(({ server, usage: allocated }) => ({
				server,
				share: cpu ? allocated.cpuPercent / 100 : ofMemory(allocated.memoryBytes),
			}))
			.filter((slice) => slice.share > 0)
			.toSorted((a, b) => b.share - a.share);
		const named = ranked.slice(0, NAMED).map(({ server, share }) => ({
			key: server.id,
			label: server.name,
			color: portColor(server.port),
			share,
			amount: amount(share),
		}));
		const rest = ranked.slice(NAMED);
		const restShare = rest.reduce((sum, slice) => sum + slice.share, 0);

		const otherAppsShare = cpu ? usage.otherAppsCpuPercent / 100 : ofMemory(usage.otherAppsBytes);
		const serversShare = cpu ? usage.serversCpuPercent / 100 : ofMemory(usage.serversBytes);
		return {
			label: cpu ? "CPU" : "Memory",
			total: cpu ? { value: String(Math.round(usage.serversCpuPercent)), unit: "%" } : formatBytes(usage.serversBytes),
			named,
			rest: rest.length ? { count: rest.length, share: restShare, amount: amount(restShare) } : null,
			otherApps: { share: otherAppsShare, amount: amount(otherAppsShare) },
			remaining: cpu
				? ["Idle", formatPercent(Math.max(1 - serversShare - otherAppsShare, 0) * 100)]
				: [
						"Free",
						`${formatBytesText(Math.max(system.totalBytes - usage.serversBytes - usage.otherAppsBytes, 0))} of ${formatBytesText(system.totalBytes)}`,
					],
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

	<div class="flex h-1.5 gap-0.5 overflow-hidden rounded-full bg-muted" aria-hidden="true">
		{#each view.named as slice (slice.key)}
			<div
				class="h-full rounded-full transition-[width] duration-300 ease-out"
				style:width="{slice.share * 100}%"
				style:background-color={slice.color}
			></div>
		{/each}
		{#if view.rest}
			<div
				class="h-full rounded-full bg-foreground/60 transition-[width] duration-300 ease-out"
				style:width="{view.rest.share * 100}%"
			></div>
		{/if}
		<div
			class="h-full rounded-full bg-muted-foreground/40 transition-[width] duration-300 ease-out"
			style:width="{view.otherApps.share * 100}%"
		></div>
	</div>

	<ul class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
		{#each view.named as slice (slice.key)}
			<li class="flex min-w-0 items-center gap-1.5">
				<span class="size-2 shrink-0 rounded-sm" style:background-color={slice.color}></span>
				<span class="max-w-32 truncate">{slice.label}</span>
				<span class="font-mono tabular-nums">{slice.amount}</span>
			</li>
		{/each}
		{#if view.rest}
			<li class="flex items-center gap-1.5">
				<span class="size-2 rounded-sm bg-foreground/60"></span>
				{view.rest.count} more <span class="font-mono tabular-nums">{view.rest.amount}</span>
			</li>
		{/if}
		<li class="flex items-center gap-1.5">
			<span class="size-2 rounded-sm bg-muted-foreground/40"></span>
			Other apps <span class="font-mono tabular-nums">{view.otherApps.amount}</span>
		</li>
		<li class="flex items-center gap-1.5">
			<span class="size-2 rounded-sm bg-muted"></span>
			{view.remaining[0]} <span class="font-mono tabular-nums">{view.remaining[1]}</span>
		</li>
	</ul>
</section>
