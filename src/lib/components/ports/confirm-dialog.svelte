<script lang="ts">
	import CircleAlertIcon from "@lucide/svelte/icons/circle-alert";
	import * as Alert from "$lib/components/ui/alert";
	import * as AlertDialog from "$lib/components/ui/alert-dialog";
	import { Badge } from "$lib/components/ui/badge";
	import { Checkbox } from "$lib/components/ui/checkbox";
	import * as Field from "$lib/components/ui/field";
	import { Skeleton } from "$lib/components/ui/skeleton";
	import type { PendingAction, PortsStore } from "$lib/ports/ports-store.svelte";
	import { isHidden, mustConfirm } from "$lib/ports/ports-store.svelte";
	import { preferences } from "$lib/ports/preferences.svelte";
	import { processKey } from "$lib/ports/resources";
	import type { ControlPreview, Server, Supervisor } from "$lib/ports/types";
	import { settings } from "$lib/settings.svelte";
	import { cubicOut } from "svelte/easing";
	import { prefersReducedMotion } from "svelte/motion";
	import { fade, fly } from "svelte/transition";

	let { store }: { store: PortsStore } = $props();

	const id = $props.id();
	let dontAskAgain = $state(false);

	// Short, opacity and transform only; nothing moves when the user asks for less motion.
	const reduced = $derived(prefersReducedMotion.current);
	/** Most previews arrive within a frame or two; the skeleton waits so it doesn't flash. */
	const appear = $derived({ delay: 150, duration: reduced ? 0 : 150 });
	const leave = $derived({ duration: reduced ? 0 : 120 });
	const enter = $derived({ y: reduced ? 0 : 4, duration: reduced ? 0 : 200, easing: cubicOut });

	/** Long worker trees fold into a count past this many rows. */
	const SHOWN_PROCESSES = 6;

	// Keeps the text in place while the dialog animates out.
	let last: PendingAction | null = null;
	const pending = $derived.by(() => (last = store.pending ?? last));

	const titles: Record<PendingAction["action"], (p: PendingAction) => string> = {
		stop: (p) => `Stop ${p.server.name}?`,
		restart: (p) => `Restart ${p.server.name}?`,
		forceKill: (p) => `Force kill ${p.server.name}?`,
		stopParent: (p) => `Stop ${p.server.parent?.name ?? "the parent"}?`,
		stopAll: (p) => `Stop every ${p.server.process} server?`,
	};
	const buttons: Record<PendingAction["action"], string> = {
		stop: "Stop",
		restart: "Restart",
		forceKill: "Force Kill",
		stopParent: "Stop parent",
		stopAll: "Stop all",
	};

	const processes = $derived(pending?.previews?.flatMap((preview) => preview.processes) ?? []);
	const keys = $derived(new Set(processes.map((affected) => processKey(affected.process))));
	/** Ports the backend found under these processes, or the container's own rows. */
	const ports = $derived.by(() => {
		if (!pending?.previews) return [];
		const found = processes.flatMap((affected) => affected.ports);
		const containers = pending.servers.filter((server) => server.target?.kind === "container").map((s) => s.port);
		return [...new Set([...found, ...containers])].toSorted((a, b) => a - b);
	});
	/** Other servers these processes also run, including ones the list hides or protects. */
	const alsoAffected = $derived(
		store.servers.filter(
			(server) =>
				!pending?.servers.some((chosen) => chosen.id === server.id) &&
				server.resources.some((resource) => keys.has(processKey(resource.process))),
		),
	);
	function supervisorName(found: Supervisor) {
		return { homebrew: "Homebrew", pm2: "PM2", supervisord: "supervisord" }[found.kind];
	}

	function describe(action: PendingAction["action"], server: Server, preview?: ControlPreview) {
		const grace = settings.current.stopGraceSeconds;
		const managed = preview?.supervisor?.supported ? preview.supervisor : undefined;
		if (managed?.kind === "homebrew") {
			return action === "restart"
				? `Homebrew restarts ${managed.name} with brew services restart.`
				: `Homebrew stops ${managed.name} with brew services stop, which also keeps it from starting at login.`;
		}
		if (managed?.kind === "pm2") return `PM2 ${action === "restart" ? "restarts" : "stops"} ${managed.name}.`;
		switch (action) {
			case "stopAll":
				return "Each server stops the way its line says.";
			case "stop":
				return `These processes get ${grace === 1 ? "1 second" : `${grace} seconds`} to exit before Ports kills them.`;
			case "restart":
				return `These processes stop, then ${server.name} starts again on :${server.port}.`;
			case "forceKill":
				return "These processes end right away, without saving or cleaning up.";
			case "stopParent":
				return `It started ${server.name} and may start it again. It ends with everything it runs.`;
		}
	}

	function confirm() {
		if (dontAskAgain) preferences.confirmActions.current = false;
		dontAskAgain = false;
		store.confirm();
	}
</script>

{#snippet respawn(preview: ControlPreview)}
	{#if preview.supervisor && !preview.supervisor.supported}
		<p class="text-warning">
			{supervisorName(preview.supervisor)} runs it and may start it again. Stop it through {supervisorName(
				preview.supervisor,
			)} to keep it stopped.
		</p>
	{/if}
{/snippet}

<AlertDialog.Root
	bind:open={
		() => store.pending !== null,
		(open) => {
			if (open) return;
			store.pending = null;
			dontAskAgain = false;
		}
	}
>
	<AlertDialog.Content size="sm">
		{#if pending}
			{@const { action, server, previews, error } = pending}
			<AlertDialog.Header>
				<AlertDialog.Title>{titles[action](pending)}</AlertDialog.Title>
				<AlertDialog.Description>
					{describe(action, server, action === "stopAll" ? undefined : previews?.[0])}
				</AlertDialog.Description>
			</AlertDialog.Header>

			<!-- Loading, error and preview share one cell, so they crossfade in place instead of jumping. -->
			<div class="grid min-h-16 text-sm *:col-start-1 *:row-start-1" aria-live="polite" aria-busy={!previews && !error}>
				{#if error}
					<div in:fly={enter}>
						<Alert.Root variant="destructive">
							<CircleAlertIcon />
							<Alert.Title>Ports can't tell what this would stop</Alert.Title>
							<Alert.Description>{error}</Alert.Description>
						</Alert.Root>
					</div>
				{:else if !previews}
					<!-- Shows only when checking takes a moment, in the shape of what it will show. -->
					<div class="flex flex-col gap-3" in:fade|global={appear} out:fade={leave}>
						<div class="flex items-center gap-3 rounded-xl bg-muted/50 px-3 py-2.5">
							<Skeleton class="h-4 w-20" />
							<Skeleton class="ms-auto h-5 w-12 rounded-full" />
							<Skeleton class="h-4 w-16" />
						</div>
						<span class="sr-only">Checking what this would stop</span>
						<Skeleton class="h-3.5 w-24" />
					</div>
				{:else}
					<div class="flex flex-col gap-3" in:fly={enter}>
						{#if processes.length}
							<ul class="flex flex-col divide-y divide-border/60 rounded-xl bg-muted/50" aria-label="Processes">
								{#each processes.slice(0, SHOWN_PROCESSES) as affected (processKey(affected.process))}
									<li class="flex items-center gap-2 px-3 py-2">
										<span class="min-w-0 flex-1 truncate font-medium">{affected.name}</span>
										{#each affected.ports as port (port)}
											<Badge variant="secondary" class="font-mono">:{port}</Badge>
										{/each}
										<span class="w-20 shrink-0 text-right font-mono text-xs text-muted-foreground tabular-nums">
											{affected.process.pid}
										</span>
									</li>
								{/each}
								{#if processes.length > SHOWN_PROCESSES}
									<li class="px-3 py-2 text-muted-foreground">
										and {processes.length - SHOWN_PROCESSES} more
									</li>
								{/if}
							</ul>
						{/if}
						{#if ports.length || alsoAffected.length}
							<p class="text-muted-foreground">
								{#if ports.length}
									Frees <span class="font-mono text-foreground">{ports.map((port) => `:${port}`).join(" ")}</span>
								{/if}
								{#if alsoAffected.length}
									{ports.length ? "· " : ""}Also stops
									{#each alsoAffected as other, i (other.id)}
										{i ? ", " : ""}<span class="text-foreground">{other.name} :{other.port}</span>{isHidden(other)
											? " (hidden)"
											: ""}{other.protected ? " (protected)" : ""}
									{/each}
								{/if}
							</p>
						{/if}
						{#if action === "stopAll"}
							<ul class="flex flex-col gap-1" aria-label="Servers">
								{#each pending.servers as row, i (row.id)}
									<li class="text-muted-foreground">
										<span class="text-foreground">{row.name} :{row.port}</span> · {describe("stop", row, previews[i])}
										{@render respawn(previews[i])}
									</li>
								{/each}
							</ul>
						{:else}
							{@render respawn(previews[0])}
						{/if}
					</div>
				{/if}
			</div>

			{#if !mustConfirm(action, server)}
				<Field.Field orientation="horizontal" class="justify-center">
					<Checkbox id="{id}-dont-ask" bind:checked={dontAskAgain} />
					<Field.Label for="{id}-dont-ask" class="font-normal text-muted-foreground">Don't ask again</Field.Label>
				</Field.Field>
			{/if}
			<AlertDialog.Footer>
				<AlertDialog.Cancel>Cancel</AlertDialog.Cancel>
				<AlertDialog.Action
					variant={action === "restart" ? "default" : "destructive"}
					disabled={!previews}
					onclick={confirm}
				>
					{buttons[action]}
				</AlertDialog.Action>
			</AlertDialog.Footer>
		{/if}
	</AlertDialog.Content>
</AlertDialog.Root>
