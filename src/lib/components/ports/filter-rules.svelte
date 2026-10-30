<script lang="ts">
	import XIcon from "@lucide/svelte/icons/x";
	import { Badge } from "$lib/components/ui/badge";
	import { Button } from "$lib/components/ui/button";
	import * as Field from "$lib/components/ui/field";
	import { Input } from "$lib/components/ui/input";
	import * as NativeSelect from "$lib/components/ui/native-select";
	import { hiddenLabel } from "$lib/ports/ports-store.svelte";
	import { rules, type Rule, type RuleField, type RulesChange } from "$lib/rules.svelte";

	let { id, disabled = false }: { id: string; disabled?: boolean } = $props();

	const current = $derived(rules.current.rules);
	const fieldLabels: Record<RuleField, string> = { process: "process", command: "command" };
	type List = "include" | "exclude";

	let start = $derived(current.portRange ? String(current.portRange.start) : "");
	let end = $derived(current.portRange ? String(current.portRange.end) : "");
	let rangeError = $state<string | null>(null);

	let list = $state<List>("exclude");
	let field = $state<RuleField>("process");
	let pattern = $state("");
	let ruleError = $state<string | null>(null);
	let hiddenError = $state<string | null>(null);

	const validPort = (port: number) => Number.isInteger(port) && port >= 1 && port <= 65_535;

	async function save(change: RulesChange, report: (error: string | null) => void) {
		try {
			await rules.change(change);
			report(null);
			return true;
		} catch (error) {
			report(String(error));
			return false;
		}
	}

	function saveRange(event: SubmitEvent) {
		event.preventDefault();
		if (!start.trim() && !end.trim()) {
			void save({ kind: "portRange", range: null }, (error) => (rangeError = error));
			return;
		}
		const range = { start: Number(start), end: Number(end) };
		if (!validPort(range.start) || !validPort(range.end)) {
			rangeError = "Use whole port numbers from 1 to 65535";
			return;
		}
		void save({ kind: "portRange", range }, (error) => (rangeError = error));
	}

	const withRules = (next: Record<List, Rule[]>): RulesChange => ({ kind: "filters", ...next });

	async function addRule(event: SubmitEvent) {
		event.preventDefault();
		const next = { include: current.include, exclude: current.exclude };
		next[list] = [...next[list], { field, pattern: pattern.trim() }];
		if (await save(withRules(next), (error) => (ruleError = error))) pattern = "";
	}

	function removeRule(from: List, index: number) {
		const next = { include: current.include, exclude: current.exclude };
		next[from] = next[from].filter((_, i) => i !== index);
		void save(withRules(next), (error) => (ruleError = error));
	}

	const ruleLists = $derived([
		...current.include.map((rule, index) => ({ list: "include" as const, rule, index })),
		...current.exclude.map((rule, index) => ({ list: "exclude" as const, rule, index })),
	]);
</script>

<Field.Field data-invalid={rangeError ? true : undefined}>
	<Field.Content>
		<Field.Label for="{id}-range-start">Port range</Field.Label>
		<Field.Description>Leave empty to list every port.</Field.Description>
	</Field.Content>
	<form class="flex items-center gap-2" onsubmit={saveRange}>
		<Input
			id="{id}-range-start"
			class="h-8 w-24"
			inputmode="numeric"
			placeholder="From"
			aria-label="First port"
			{disabled}
			bind:value={start}
		/>
		<span class="text-muted-foreground">–</span>
		<Input class="h-8 w-24" inputmode="numeric" placeholder="To" aria-label="Last port" {disabled} bind:value={end} />
		<Button type="submit" variant="secondary" size="sm" {disabled}>Save</Button>
	</form>
	{#if rangeError}
		<Field.Error>{rangeError}</Field.Error>
	{/if}
</Field.Field>
<Field.Separator />
<Field.Field data-invalid={ruleError ? true : undefined}>
	<Field.Content>
		<Field.Label for="{id}-pattern">Filters</Field.Label>
		<Field.Description>Matches text in the process name or command. Hide wins over Show only.</Field.Description>
	</Field.Content>
	{#if ruleLists.length}
		<div class="flex flex-wrap gap-1.5">
			{#each ruleLists as item (`${item.list}-${item.index}`)}
				<Badge variant={item.list === "exclude" ? "secondary" : "outline"} class="gap-1 pr-0.5">
					{item.list === "exclude" ? "Hide" : "Show only"}
					{fieldLabels[item.rule.field]}
					<span class="font-mono">{item.rule.pattern}</span>
					<Button
						variant="ghost"
						size="icon-xs"
						class="size-4 rounded-full"
						aria-label="Remove the rule {item.rule.pattern}"
						{disabled}
						onclick={() => removeRule(item.list, item.index)}
					>
						<XIcon />
					</Button>
				</Badge>
			{/each}
		</div>
	{/if}
	<form class="flex flex-wrap items-center gap-2" onsubmit={addRule}>
		<NativeSelect.Root size="sm" class="w-28" aria-label="What the rule does" {disabled} bind:value={list}>
			<NativeSelect.Option value="exclude">Hide</NativeSelect.Option>
			<NativeSelect.Option value="include">Show only</NativeSelect.Option>
		</NativeSelect.Root>
		<NativeSelect.Root size="sm" class="w-28" aria-label="What the rule matches" {disabled} bind:value={field}>
			<NativeSelect.Option value="process">Process</NativeSelect.Option>
			<NativeSelect.Option value="command">Command</NativeSelect.Option>
		</NativeSelect.Root>
		<Input
			id="{id}-pattern"
			class="h-8 min-w-0 flex-1"
			placeholder={field === "process" ? "ControlCenter" : "storybook"}
			{disabled}
			bind:value={pattern}
		/>
		<Button type="submit" variant="secondary" size="sm" disabled={disabled || !pattern.trim()}>Add</Button>
	</form>
	{#if ruleError}
		<Field.Error>{ruleError}</Field.Error>
	{/if}
</Field.Field>
<Field.Separator />
<Field.Field data-invalid={hiddenError ? true : undefined}>
	<Field.Content>
		<Field.Title>Hidden servers</Field.Title>
		<Field.Description>
			{current.hidden.length ? "Remove one to show it again." : "None. Hide one from its details."}
		</Field.Description>
	</Field.Content>
	{#if current.hidden.length}
		<div class="flex flex-wrap gap-1.5">
			{#each current.hidden as entry (entry)}
				<Badge variant="secondary" class="gap-0.5 pr-0.5">
					{hiddenLabel(entry)}
					<Button
						variant="ghost"
						size="icon-xs"
						class="size-4 rounded-full"
						aria-label="Show {hiddenLabel(entry)} again"
						{disabled}
						onclick={() => save({ kind: "unhide", entry }, (error) => (hiddenError = error))}
					>
						<XIcon />
					</Button>
				</Badge>
			{/each}
		</div>
	{/if}
	{#if hiddenError}
		<Field.Error>{hiddenError}</Field.Error>
	{/if}
</Field.Field>
