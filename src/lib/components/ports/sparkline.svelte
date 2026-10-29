<script lang="ts">
	import { cn } from "$lib/utils";

	let {
		values,
		fromZero = false,
		max,
		threshold,
		area = false,
		class: className,
	}: {
		values: number[];
		fromZero?: boolean;
		max?: number;
		threshold?: number;
		area?: boolean;
		class?: string;
	} = $props();

	const W = 100;
	const H = 32;

	const domain = $derived.by(() => {
		if (max !== undefined) return { lo: 0, hi: max };
		const hi = Math.max(...values, threshold ?? 0);
		if (fromZero) return { lo: 0, hi: hi * 1.15 || 1 };
		const lo = Math.min(...values);
		const span = Math.max(hi - lo, hi * 0.25) || 1;
		const pad = (span - (hi - lo)) / 2;
		return { lo: lo - pad, hi: hi + pad };
	});

	const y = (value: number) => H - ((value - domain.lo) / (domain.hi - domain.lo)) * H;

	const points = $derived(
		values.map((value, i) => `${(i / Math.max(values.length - 1, 1)) * W},${y(value)}`).join(" "),
	);
</script>

<svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" class={cn("overflow-visible", className)} aria-hidden="true">
	{#if threshold !== undefined}
		<line
			x1="0"
			x2={W}
			y1={y(threshold)}
			y2={y(threshold)}
			class="stroke-warning"
			stroke-width="1"
			stroke-dasharray="3 3"
			vector-effect="non-scaling-stroke"
		/>
	{/if}
	{#if area}
		<polygon points="0,{H} {points} {W},{H}" class="fill-current opacity-10" />
	{/if}
	<polyline
		{points}
		fill="none"
		stroke="currentColor"
		stroke-width="1.5"
		stroke-linejoin="round"
		stroke-linecap="round"
		vector-effect="non-scaling-stroke"
	/>
</svg>
