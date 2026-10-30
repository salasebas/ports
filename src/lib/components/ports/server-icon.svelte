<script lang="ts">
	import AppWindowIcon from "@lucide/svelte/icons/app-window";
	import CodeXmlIcon from "@lucide/svelte/icons/code-xml";
	import ContainerIcon from "@lucide/svelte/icons/container";
	import DatabaseIcon from "@lucide/svelte/icons/database";
	import { iconUrl } from "$lib/ports/format";
	import type { Server, ServerKind } from "$lib/ports/types";
	import { cn } from "$lib/utils";

	let { server, class: className }: { server: Pick<Server, "icon" | "kind">; class?: string } = $props();

	const fallbacks: Record<ServerKind, typeof AppWindowIcon> = {
		dev: CodeXmlIcon,
		database: DatabaseIcon,
		container: ContainerIcon,
		other: AppWindowIcon,
	};

	const src = $derived(iconUrl(server.icon));
	let brokenIcon = $state<string | undefined>();
	const Fallback = $derived(fallbacks[server.kind]);
</script>

{#if src && brokenIcon !== server.icon}
	<img
		{src}
		alt=""
		class={cn("shrink-0 rounded-[3px] object-contain", className)}
		onerror={() => (brokenIcon = server.icon)}
	/>
{:else}
	<Fallback class={cn("shrink-0 text-muted-foreground", className)} aria-hidden="true" />
{/if}
