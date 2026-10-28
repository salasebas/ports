<script lang="ts">
	import CircleAlertIcon from "@lucide/svelte/icons/circle-alert";
	import * as Alert from "$lib/components/ui/alert";
	import * as AlertDialog from "$lib/components/ui/alert-dialog";
	import { Badge } from "$lib/components/ui/badge";
	import { Checkbox } from "$lib/components/ui/checkbox";
	import * as Field from "$lib/components/ui/field";
	import { Spinner } from "$lib/components/ui/spinner";
	import type { PendingAction, PortsStore } from "$lib/ports/ports-store.svelte";
	import { isHidden } from "$lib/ports/ports-store.svelte";
	import { preferences } from "$lib/ports/preferences.svelte";
	import { processKey } from "$lib/ports/resources";
	import type { ControlPreview, Server, Supervisor } from "$lib/ports/types";
	import { settings } from "$lib/settings.svelte";

	let { store }: { store: PortsStore } = $props();

	const id = $props.id();
	let dontAskAgain = $state(false);

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
	const supervisor = $derived(pending?.previews?.find((preview) => preview.supervisor)?.supervisor);

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
			case "stop":
			case "stopAll":
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
				<AlertDialog.Description>{describe(action, server, previews?.[0])}</AlertDialog.Description>
			</AlertDialog.Header>

			{#if error}
				<Alert.Root variant="destructive">
					<CircleAlertIcon />
					<Alert.Title>Ports can't tell what this would stop</Alert.Title>
					<Alert.Description>{error}</Alert.Description>
				</Alert.Root>
			{:else if !previews}
				<p class="flex items-center justify-center gap-2 text-sm text-muted-foreground">
					<Spinner />
					Checking what this would stop…
				</p>
			{:else}
				<div class="flex flex-col gap-3 text-sm" aria-live="polite">
					{#if processes.length}
						<ul class="flex flex-col gap-1" aria-label="Processes">
							{#each processes.slice(0, SHOWN_PROCESSES) as affected (processKey(affected.process))}
								<li class="flex items-center gap-2">
									<span class="min-w-0 flex-1 truncate">{affected.name}</span>
									{#each affected.ports as port (port)}
										<Badge variant="secondary" class="font-mono">:{port}</Badge>
									{/each}
									<span class="shrink-0 font-mono text-xs text-muted-foreground">PID {affected.process.pid}</span>
								</li>
							{/each}
							{#if processes.length > SHOWN_PROCESSES}
								<li class="text-muted-foreground">and {processes.length - SHOWN_PROCESSES} more</li>
							{/if}
						</ul>
					{/if}
					{#if ports.length}
						<p class="text-muted-foreground">
							Frees {ports.map((port) => `:${port}`).join(", ")}
						</p>
					{/if}
					{#if alsoAffected.length}
						<p class="text-muted-foreground">
							Also stops
							{#each alsoAffected as other, i (other.id)}
								{i ? ", " : ""}<span class="text-foreground">{other.name} :{other.port}</span>{isHidden(other)
									? " (hidden)"
									: ""}{other.protected ? " (protected)" : ""}
							{/each}
						</p>
					{/if}
					{#if supervisor && !supervisor.supported}
						<p class="text-warning">
							{supervisorName(supervisor)} runs it and may start it again. Stop it with supervisorctl to keep it stopped.
						</p>
					{/if}
				</div>
			{/if}

			{#if action === "stop" || action === "restart"}
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
