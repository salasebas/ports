<script lang="ts">
	import { onMount } from "svelte";
	import * as Field from "$lib/components/ui/field";
	import * as NativeSelect from "$lib/components/ui/native-select";
	import { Switch } from "$lib/components/ui/switch";
	import { formatDuration } from "$lib/ports/format";
	import {
		autoIdleChoices,
		rules,
		type AutoCleanup,
		type AutoCleanupEntry,
		type AutoCleanupScope,
	} from "$lib/rules.svelte";

	let { id, disabled = false }: { id: string; disabled?: boolean } = $props();

	const auto = $derived(rules.current.rules.autoCleanup);
	let error = $state<string | null>(null);
	let entries = $state.raw<AutoCleanupEntry[]>([]);

	const defaultIdleMinutes = 120;
	const scopes: { value: AutoCleanupScope | "off"; label: string }[] = [
		{ value: "off", label: "Off" },
		{ value: "deleted", label: "Deleted folders" },
		{ value: "idle", label: "Every idle server" },
	];
	const outcomes: Record<AutoCleanupEntry["outcome"], string> = {
		stopped: "Stopped",
		skipped: "Left running",
		failed: "Couldn't stop",
	};

	function ago(at: number) {
		const elapsed = formatDuration(Date.now() - at);
		return elapsed === "now" ? "just now" : `${elapsed} ago`;
	}

	async function save(next: AutoCleanup) {
		try {
			await rules.change({ kind: "autoCleanup", ...next });
			error = null;
		} catch (reason) {
			error = String(reason);
		}
	}

	function setScope(value: string) {
		if (value === "off") return save({ policy: null, paused: false });
		const idleMinutes = auto.policy?.idleMinutes ?? defaultIdleMinutes;
		return save({ policy: { scope: value as AutoCleanupScope, idleMinutes }, paused: false });
	}

	async function readLog() {
		if (disabled) return;
		try {
			entries = await rules.autoCleanupLog();
		} catch (reason) {
			error = String(reason);
		}
	}

	onMount(() => void readLog());
</script>

<svelte:window onfocus={readLog} />

<Field.Field orientation="horizontal">
	<Field.Content>
		<Field.Label for="{id}-auto-scope">Stop forgotten servers</Field.Label>
		<Field.Description>
			Stops idle dev servers for you and notifies you. Never databases, containers, hidden or protected ones.
		</Field.Description>
	</Field.Content>
	<NativeSelect.Root
		id="{id}-auto-scope"
		size="sm"
		class="w-40"
		{disabled}
		bind:value={() => auto.policy?.scope ?? "off", setScope}
	>
		{#each scopes as scope (scope.value)}
			<NativeSelect.Option value={scope.value}>{scope.label}</NativeSelect.Option>
		{/each}
	</NativeSelect.Root>
</Field.Field>
{#if auto.policy}
	{@const policy = auto.policy}
	<Field.Separator />
	<Field.Field orientation="horizontal">
		<Field.Content>
			<Field.Label for="{id}-auto-idle">Idle for</Field.Label>
			<Field.Description>Without connections or CPU.</Field.Description>
		</Field.Content>
		<NativeSelect.Root
			id="{id}-auto-idle"
			size="sm"
			class="w-40"
			{disabled}
			bind:value={
				() => String(policy.idleMinutes),
				(value) => save({ policy: { ...policy, idleMinutes: Number(value) }, paused: auto.paused })
			}
		>
			{#each autoIdleChoices as choice (choice.value)}
				<NativeSelect.Option value={choice.value}>{choice.label}</NativeSelect.Option>
			{/each}
		</NativeSelect.Root>
	</Field.Field>
	<Field.Separator />
	<Field.Field orientation="horizontal">
		<Field.Content>
			<Field.Label for="{id}-auto-pause">Pause</Field.Label>
			<Field.Description>Stops nothing until you resume.</Field.Description>
		</Field.Content>
		<Switch id="{id}-auto-pause" {disabled} bind:checked={() => auto.paused, (paused) => save({ policy, paused })} />
	</Field.Field>
{/if}
{#if error}
	<Field.Error>{error}</Field.Error>
{/if}
{#if entries.length}
	<Field.Separator />
	<Field.Field>
		<Field.Title>Recent automatic stops</Field.Title>
		<ul class="flex max-h-48 flex-col gap-1.5 overflow-y-auto text-xs">
			{#each entries as entry, i (i)}
				<li class="flex items-baseline gap-2">
					<span class="w-12 shrink-0 font-mono text-muted-foreground tabular-nums">:{entry.port}</span>
					<span class="min-w-0 flex-1 truncate" title={entry.message ?? entry.folder}>
						<span class={entry.outcome === "stopped" ? "" : "text-warning"}>{outcomes[entry.outcome]}</span>
						{entry.name}{entry.cwdDeleted ? " · folder deleted" : ""} · idle {formatDuration(
							entry.idleMinutes * 60_000,
						)}
					</span>
					<span class="shrink-0 text-muted-foreground">{ago(entry.at)}</span>
				</li>
			{/each}
		</ul>
	</Field.Field>
{/if}
