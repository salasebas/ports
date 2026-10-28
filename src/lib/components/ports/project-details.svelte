<script lang="ts">
	import { invoke } from "@tauri-apps/api/core";
	import CircleAlertIcon from "@lucide/svelte/icons/circle-alert";
	import PlayIcon from "@lucide/svelte/icons/play";
	import SquareIcon from "@lucide/svelte/icons/square";
	import StarIcon from "@lucide/svelte/icons/star";
	import Trash2Icon from "@lucide/svelte/icons/trash-2";
	import * as Alert from "$lib/components/ui/alert";
	import { Badge } from "$lib/components/ui/badge";
	import { Button } from "$lib/components/ui/button";
	import { Input } from "$lib/components/ui/input";
	import { Separator } from "$lib/components/ui/separator";
	import type { PortsStore } from "$lib/ports/ports-store.svelte";
	import { projectName, type ProjectLog, type SavedProject } from "$lib/ports/projects";
	import { cn } from "$lib/utils";

	let { project, store, class: className }: { project: SavedProject; store: PortsStore; class?: string } = $props();

	let log = $state<ProjectLog | null>(null);

	$effect(() => {
		const id = project.id;
		let current = true;
		log = null;
		void invoke<ProjectLog>("project_log", { id })
			.then((value) => {
				if (current) log = value;
			})
			.catch(() => {
				if (current) log = { text: "", truncated: false };
			});
		return () => {
			current = false;
		};
	});

	const title = $derived(projectName(project));
	const result = $derived(project.lastResult);
	const starting = $derived(Boolean(project.process) && !project.matchedServerId);

	function saveCommand(form: HTMLFormElement) {
		const command = String(new FormData(form).get("command") ?? "");
		void store.setSavedCommand(project, command);
	}

	function saveBinding(form: HTMLFormElement) {
		const data = new FormData(form);
		const expectedPort = Number(data.get("port"));
		const expectedAddress = String(data.get("address") ?? "");
		void store.setSavedBinding(project, expectedPort, expectedAddress);
	}
</script>

<article class={cn("flex flex-col gap-5", className)} aria-label="{title} saved project">
	<header class="flex flex-col gap-1">
		<h2 class="truncate text-sm font-semibold">{title}</h2>
		<p class="font-mono text-4xl tabular-nums">:{project.expectedPort}</p>
		<p class="text-sm text-muted-foreground">
			{starting ? "Starting" : "Stopped"} · {project.expectedAddress}:{project.expectedPort}
		</p>
	</header>

	{#if project.folderMissing}
		<Alert.Root class="text-warning">
			<CircleAlertIcon />
			<Alert.Title>Folder missing</Alert.Title>
			<Alert.Description>The project folder {project.folder} is missing.</Alert.Description>
		</Alert.Root>
	{:else if result?.outcome === "conflict" && result.owner}
		<Alert.Root class="text-warning">
			<CircleAlertIcon />
			<Alert.Title>Port in use</Alert.Title>
			<Alert.Description>{result.message}</Alert.Description>
		</Alert.Root>
		{#if result.owner.serverId}
			<Button variant="secondary" size="sm" onclick={() => store.select(result.owner?.serverId ?? null)}>
				Show {result.owner.name} on :{result.owner.port}
			</Button>
		{/if}
	{:else if result && result.outcome !== "started"}
		<Alert.Root class={result.outcome === "failed" || result.outcome === "missingFolder" ? "text-warning" : undefined}>
			<CircleAlertIcon />
			<Alert.Title>
				{result.outcome === "stopped" ? "Stopped" : result.outcome === "exited" ? "Exited" : "Didn't start"}
			</Alert.Title>
			<Alert.Description>{result.message}</Alert.Description>
		</Alert.Root>
	{/if}

	<div class="flex gap-2">
		{#if project.matchedServerId}
			<Button class="flex-1" onclick={() => store.select(project.matchedServerId ?? null)}>
				Show the running server
			</Button>
		{:else if project.process}
			<Button class="flex-1" variant="destructive" onclick={() => store.stopSaved(project)}>
				<SquareIcon data-icon="inline-start" class="fill-current" />
				Stop
			</Button>
		{:else}
			<Button class="flex-1" disabled={project.folderMissing} onclick={() => store.startSaved(project)}>
				<PlayIcon data-icon="inline-start" />
				Start
			</Button>
		{/if}
		<Button
			variant="secondary"
			size="icon-lg"
			aria-label="Remove saved project"
			onclick={() => store.forgetSaved(project)}
		>
			<Trash2Icon />
		</Button>
	</div>
	<p class="text-xs text-muted-foreground">
		Start runs the saved command in its folder. It is not a restart of a process that is already listening. Recovery is
		manual, so a failure stays here and is not tried again.
	</p>

	<Separator />

	<dl class="grid grid-cols-[5.5rem_1fr] gap-x-4 gap-y-2.5 text-sm">
		<dt class="text-muted-foreground">Folder</dt>
		<dd class="truncate" title={project.folder}>{project.folder}</dd>
		<dt class="text-muted-foreground">Command</dt>
		<dd class="truncate font-mono text-xs" title={project.command}>{project.command}</dd>
		<dt class="text-muted-foreground">Binding</dt>
		<dd class="font-mono text-xs">{project.expectedAddress}:{project.expectedPort}</dd>
	</dl>

	<form
		class="flex flex-col gap-2"
		onsubmit={(event) => {
			event.preventDefault();
			saveBinding(event.currentTarget);
		}}
	>
		<div class="grid grid-cols-[7rem_1fr] gap-2">
			<Input name="address" aria-label="Expected address" value={project.expectedAddress} />
			<Input name="port" aria-label="Expected port" type="number" min="1" max="65535" value={project.expectedPort} />
		</div>
		<Button type="submit" variant="secondary" size="sm">Save expected binding</Button>
	</form>

	{#if project.scripts.length}
		<section class="flex flex-col gap-2" aria-label="Scripts">
			<p class="text-sm text-muted-foreground">Scripts · {project.manager}</p>
			<ul class="flex flex-col gap-1">
				{#each project.scripts as script (script.name)}
					<li class="flex items-center gap-1">
						<Button
							class="min-w-0 flex-1 justify-start"
							variant={script.run === project.command ? "secondary" : "ghost"}
							size="sm"
							disabled={script.missing}
							onclick={() => store.setSavedCommand(project, script.run)}
						>
							<span class="truncate">{script.missing ? `${script.name} (missing)` : script.name}</span>
						</Button>
						<Button
							variant="ghost"
							size="icon-sm"
							aria-label={script.favorite ? `Unfavorite ${script.name}` : `Favorite ${script.name}`}
							aria-pressed={script.favorite}
							onclick={() => store.setFavorite(project, script.name, !script.favorite)}
						>
							<StarIcon class={cn(script.favorite && "fill-current")} />
						</Button>
					</li>
				{/each}
			</ul>
		</section>
	{/if}

	<form
		class="flex flex-col gap-2"
		onsubmit={(event) => {
			event.preventDefault();
			saveCommand(event.currentTarget);
		}}
	>
		<Input name="command" aria-label="Custom command" value={project.command} />
		<Button type="submit" variant="secondary" size="sm">Save command</Button>
	</form>

	<section class="flex flex-col gap-2" aria-label="Log">
		<div class="flex items-center gap-2">
			<p class="text-sm text-muted-foreground">Log</p>
			{#if log?.truncated}
				<Badge variant="outline">Last 64 KB</Badge>
			{/if}
		</div>
		<pre
			class="max-h-48 overflow-y-auto rounded-lg bg-muted p-3 font-mono text-xs leading-5 whitespace-pre-wrap">{log?.text ||
				"No output yet from a run Ports started."}</pre>
	</section>
</article>
