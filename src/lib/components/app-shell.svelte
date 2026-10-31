<script lang="ts">
	import type { Snippet } from "svelte";
	import { preferences } from "$lib/ports/preferences.svelte";
	import { cn } from "$lib/utils";

	let { header, children }: { header: Snippet; children: Snippet } = $props();
	let headerHeight = $state(60);
	const keepVisible = $derived(preferences.keepHeaderVisible.current);
</script>

<div
	class="isolate grid h-dvh [scrollbar-gutter:stable_both-edges] grid-cols-[minmax(0,1fr)] grid-rows-[auto_1fr] overflow-y-auto overscroll-contain"
	style:scroll-padding-block-start={keepVisible ? `${headerHeight}px` : undefined}
>
	<header
		bind:offsetHeight={headerHeight}
		class={cn("z-10 border-b bg-background", keepVisible && "sticky inset-bs-0")}
	>
		{@render header()}
	</header>
	<main>
		{@render children()}
	</main>
</div>
