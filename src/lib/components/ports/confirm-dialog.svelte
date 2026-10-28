<script lang="ts">
	import CircleAlertIcon from "@lucide/svelte/icons/circle-alert";
	import * as Alert from "$lib/components/ui/alert";
	import * as AlertDialog from "$lib/components/ui/alert-dialog";
	import { Checkbox } from "$lib/components/ui/checkbox";
	import * as Field from "$lib/components/ui/field";
	import { Spinner } from "$lib/components/ui/spinner";
	import type { PendingAction, PortsStore } from "$lib/ports/ports-store.svelte";
	import { isHidden, mustConfirm } from "$lib/ports/ports-store.svelte";
	import { preferences } from "$lib/ports/preferences.svelte";
	import { processKey } from "$lib/ports/resources";
	import type { ControlPreview, Server, Supervisor } from "$lib/ports/types";
	import { settings } from "$lib/settings.svelte";
	import { cubicOut } from "svelte/easing";
	import { prefersReducedMotion } from "svelte/motion";
	import { fade, fly } from "svelte/transition";
	import PortLabel from "./port-label.svelte";

	let { store }: { store: PortsStore } = $props();

	const id = $props.id();
	let dontAskAgain = $state(false);

	// Short, opacity and transform only; nothing moves when the user asks for less motion.
	const reduced = $derived(prefersReducedMotion.current);
	/** Most previews arrive within a frame or two; the skeleton waits so it doesn't flash. */
	const appear = $derived({ delay: 150, duration: reduced ? 0 : 150 });
	const leave = $derived({ duration: reduced ? 0 : 120 });
	const enter = $derived({ y: reduced ? 0 : 4, duration: reduced ? 0 : 200, easing: cubicOut });

	/** Long lists fold into a count past this many rows. */
	const SHOWN_ROWS = 6;

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

	/** One line of the preview: a process, or a whole server for Stop all. */
	interface Row {
		key: string;
		name: string;
		ports: number[];
		pids: number[];
		/** How a supervisor stops it, when Ports goes through one. */
		note?: string;
		/** A supervisor Ports can't control may start it again. */
		warning?: string;
	}

	const unique = (ports: number[]) => [...new Set(ports)].toSorted((a, b) => a - b);

	const rows = $derived.by((): Row[] => {
		if (!pending?.previews) return [];
		const { action, server, servers, previews } = pending;
		if (action === "stopAll") {
			return servers.map((row, i) => ({
				key: row.id,
				name: row.name,
				ports: unique(previews[i].processes.flatMap((affected) => affected.ports)),
				pids: previews[i].processes.map((affected) => affected.process.pid),
				note: managedNote(previews[i]),
				warning: respawnWarning(previews[i]),
			}));
		}
		const [preview] = previews;
		// A container has no processes of its own here; Docker stops it as a whole.
		if (!preview.processes.length) return [{ key: server.id, name: server.name, ports: [server.port], pids: [] }];
		return preview.processes.map((affected) => ({
			key: processKey(affected.process),
			name: affected.name,
			ports: affected.ports,
			pids: [affected.process.pid],
		}));
	});

	const keys = $derived(
		new Set(pending?.previews?.flatMap((preview) => preview.processes.map((p) => processKey(p.process))) ?? []),
	);
	/** Other servers these processes also run, including ones the list hides or protects. */
	const alsoAffected = $derived(
		store.servers.filter(
			(server) =>
				!pending?.servers.some((chosen) => chosen.id === server.id) &&
				server.resources.some((resource) => keys.has(processKey(resource.process))),
		),
	);
	const warning = $derived(
		pending?.action !== "stopAll" && pending?.previews ? respawnWarning(pending.previews[0]) : undefined,
	);

	function supervisorName(found: Supervisor) {
		return { homebrew: "Homebrew", pm2: "PM2", supervisord: "supervisord" }[found.kind];
	}

	function managedNote(preview: ControlPreview) {
		const managed = preview.supervisor?.supported ? preview.supervisor : undefined;
		if (managed?.kind === "homebrew") return "Through brew services, which also keeps it off at login";
		if (managed?.kind === "pm2") return "Through PM2";
	}

	function respawnWarning(preview: ControlPreview) {
		const found = preview.supervisor;
		if (!found || found.supported) return;
		return `${supervisorName(found)} may start it again; stop it there to keep it stopped`;
	}

	const seconds = (n: number) => (n === 1 ? "1 second" : `${n} seconds`);

	function describe(shown: PendingAction) {
		const { action, server, previews } = shown;
		const grace = seconds(settings.current.stopGraceSeconds);
		const managed = action === "stopAll" ? undefined : previews?.[0].supervisor;
		if (managed?.supported && managed.kind === "homebrew") {
			return action === "restart"
				? `Homebrew restarts ${managed.name} with brew services restart.`
				: `Homebrew stops ${managed.name} with brew services stop, which also keeps it from starting at login.`;
		}
		if (managed?.supported && managed.kind === "pm2") {
			return `PM2 ${action === "restart" ? "restarts" : "stops"} ${managed.name}.`;
		}
		switch (action) {
			case "stopAll":
				return previews?.some((preview) => preview.supervisor?.supported)
					? `Each gets ${grace} to exit before Ports kills it, unless its line says otherwise.`
					: `Each gets ${grace} to exit before Ports kills it.`;
			case "stop":
				return `Each gets ${grace} to exit before Ports kills it.`;
			case "restart":
				return `It stops, then ${server.name} starts again on :${server.port}.`;
			case "forceKill":
				return "Ends right away, without saving or cleaning up.";
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
				<AlertDialog.Description>{describe(pending)}</AlertDialog.Description>
			</AlertDialog.Header>

			<!-- Loading, error and preview share one cell, so they crossfade in place instead of jumping. -->
			<div class="grid text-sm *:col-start-1 *:row-start-1" aria-live="polite" aria-busy={!previews && !error}>
				{#if error}
					<div in:fly={enter}>
						<Alert.Root variant="destructive">
							<CircleAlertIcon />
							<Alert.Title>Ports can't tell what this would stop</Alert.Title>
							<Alert.Description>{error}</Alert.Description>
						</Alert.Root>
					</div>
				{:else if !previews}
					<!-- Shows only when checking takes a moment, so a fast preview doesn't flash it. -->
					<div class="flex h-10 items-center justify-center" in:fade|global={appear} out:fade={leave}>
						<Spinner class="size-5 text-muted-foreground" aria-label="Checking what this would stop" />
					</div>
				{:else}
					<div class="flex flex-col gap-3" in:fly={enter}>
						<ul
							class="flex flex-col divide-y divide-border/60 rounded-xl bg-muted/50"
							aria-label={action === "stopAll" ? "Servers" : "Processes"}
						>
							{#each rows.slice(0, SHOWN_ROWS) as row (row.key)}
								<li class="flex min-h-10 flex-col justify-center gap-0.5 px-3 py-2">
									<div class="flex items-baseline gap-2">
										<span class="min-w-0 truncate font-medium">{row.name}</span>
										{#each row.ports as port (port)}
											<PortLabel {port} class="text-[13px]" />
										{/each}
										{#if row.pids.length}
											<span class="ms-auto shrink-0 font-mono text-xs text-muted-foreground tabular-nums">
												{row.pids[0]}{row.pids.length > 1 ? ` +${row.pids.length - 1}` : ""}
											</span>
										{/if}
									</div>
									{#if row.note}
										<p class="text-xs text-muted-foreground">{row.note}</p>
									{/if}
									{#if row.warning}
										<p class="text-xs text-warning">{row.warning}</p>
									{/if}
								</li>
							{/each}
							{#if rows.length > SHOWN_ROWS}
								<li class="flex h-10 items-center px-3 text-muted-foreground">
									and {rows.length - SHOWN_ROWS} more
								</li>
							{/if}
						</ul>
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
						{#if warning}
							<p class="text-warning">{warning}.</p>
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
