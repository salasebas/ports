<script lang="ts">
	import type { Snippet } from "svelte";

	let { header, children }: { header: Snippet; children: Snippet } = $props();
	let headerHeight = $state(60);
</script>

<div class="relative isolate flex h-dvh flex-col overflow-clip">
	<header
		bind:offsetHeight={headerHeight}
		class="absolute inset-x-0 inset-bs-0 z-10 border-b bg-surface-translucent backdrop-blur-xl backdrop-saturate-150"
	>
		{@render header()}
	</header>
	<main
		class="min-h-0 flex-1 overflow-y-auto overscroll-contain"
		style:padding-block-start={`${headerHeight}px`}
		style:scroll-padding-block-start={`${headerHeight}px`}
	>
		{@render children()}
	</main>
</div>

<style>
	@media (prefers-reduced-transparency: reduce) {
		header {
			backdrop-filter: none;
		}
	}
</style>
