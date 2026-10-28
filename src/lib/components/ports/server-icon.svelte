<script lang="ts">
	import AppWindowIcon from "@lucide/svelte/icons/app-window";
	import CodeXmlIcon from "@lucide/svelte/icons/code-xml";
	import ContainerIcon from "@lucide/svelte/icons/container";
	import DatabaseIcon from "@lucide/svelte/icons/database";
	import { iconUrl } from "$lib/ports/format";
	import type { Server, ServerKind } from "$lib/ports/types";
	import { cn } from "$lib/utils";

	let { server, class: className }: { server: Pick<Server, "icon" | "kind">; class?: string } = $props();

	/** Stands in for a page or project without an icon, so each kind always looks the same. */
	const fallbacks: Record<ServerKind, typeof AppWindowIcon> = {
		dev: CodeXmlIcon,
		database: DatabaseIcon,
		container: ContainerIcon,
		other: AppWindowIcon,
	};

	const src = $derived(iconUrl(server.icon));
	/** The key of an icon the window couldn't show; a new key gets a new try. */
	let failed = $state<string | undefined>();
	const Fallback = $derived(fallbacks[server.kind]);
</script>

{#if src && failed !== server.icon}
	<img
		{src}
		alt=""
		class={cn("shrink-0 rounded-[3px] object-contain", className)}
		onerror={() => (failed = server.icon)}
	/>
{:else}
	<Fallback class={cn("shrink-0 text-muted-foreground", className)} aria-hidden="true" />
{/if}
