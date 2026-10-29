<script lang="ts">
	import { encode } from "uqr";
	import { cn } from "$lib/utils";

	let { value, class: className }: { value: string; class?: string } = $props();

	const qr = $derived(encode(value, { border: 0 }));
	const path = $derived(qr.data.flatMap((row, y) => row.map((dark, x) => (dark ? `M${x} ${y}h1v1h-1z` : ""))).join(""));
</script>

<div class={cn("rounded-xl bg-qr-background p-2.5 text-qr", className)}>
	<svg viewBox="0 0 {qr.size} {qr.size}" class="block size-full fill-current" shape-rendering="crispEdges" role="img">
		<title>QR code for {value}</title>
		<path d={path} />
	</svg>
</div>
