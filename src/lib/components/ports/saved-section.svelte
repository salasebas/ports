<script lang="ts">
	import PlayIcon from "@lucide/svelte/icons/play";
	import SquareIcon from "@lucide/svelte/icons/square";
	import { Badge } from "$lib/components/ui/badge";
	import { Button } from "$lib/components/ui/button";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";
	import { projectName, type SavedProject } from "$lib/ports/projects";

	let { store, onOpen }: { store: PortsStore; onOpen: (project: SavedProject) => void } = $props();

	const projects = $derived(store.savedProjects);

	function status(project: SavedProject): string {
		if (project.folderMissing) return "Folder missing";
		if (project.pid) return "Starting";
		if (project.lastResult?.outcome === "conflict") return "Port in use";
		if (project.lastResult?.outcome === "failed") return "Failed";
		return "Stopped";
	}
</script>

{#if projects.length}
	<section class="flex flex-col gap-2" aria-label="Saved projects">
		<h2 class="px-1 text-xs font-medium text-muted-foreground">Saved</h2>
		<ul class="overflow-hidden rounded-2xl border bg-card">
			{#each projects as project (project.id)}
				<li class="flex items-center gap-3 border-b px-4 py-3 last:border-b-0">
					<button
						type="button"
						class="flex min-w-0 flex-1 flex-col items-start gap-0.5 text-start"
						onclick={() => onOpen(project)}
					>
						<span class="flex items-center gap-2 font-medium">
							{projectName(project)}
							<Badge variant="outline">:{project.expectedPort}</Badge>
						</span>
						<span class="w-full truncate text-xs text-muted-foreground">{status(project)} · {project.command}</span>
					</button>
					{#if project.pid}
						<Button
							variant="ghost"
							size="icon-sm"
							aria-label="Stop {projectName(project)}"
							onclick={() => store.stopSaved(project)}
						>
							<SquareIcon />
						</Button>
					{:else}
						<Button
							variant="ghost"
							size="icon-sm"
							aria-label="Start {projectName(project)}"
							disabled={project.folderMissing}
							onclick={() => store.startSaved(project)}
						>
							<PlayIcon />
						</Button>
					{/if}
				</li>
			{/each}
		</ul>
	</section>
{/if}
