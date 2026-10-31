<script lang="ts" generics="T extends string">
	import ChevronDownIcon from "@lucide/svelte/icons/chevron-down";
	import { Button } from "$lib/components/ui/button";
	import * as DropdownMenu from "$lib/components/ui/dropdown-menu";
	import * as NativeSelect from "$lib/components/ui/native-select";
	import { isMac } from "$lib/platform";
	import { cn } from "$lib/utils";

	let {
		value = $bindable(),
		options,
		id,
		class: className,
		disabled,
		"aria-label": ariaLabel,
	}: {
		value: T;
		options: readonly { value: T; label: string }[];
		id?: string;
		class?: string;
		disabled?: boolean;
		"aria-label"?: string;
	} = $props();
</script>

{#if isMac}
	<NativeSelect.Root {id} size="sm" class={className} {disabled} aria-label={ariaLabel} bind:value>
		{#each options as option (option.value)}
			<NativeSelect.Option value={option.value}>{option.label}</NativeSelect.Option>
		{/each}
	</NativeSelect.Root>
{:else}
	<DropdownMenu.Root>
		<DropdownMenu.Trigger {disabled}>
			{#snippet child({ props })}
				<Button
					{...props}
					{id}
					variant="secondary"
					size="sm"
					class={cn("justify-between font-normal", className)}
					aria-label={ariaLabel}
				>
					<span class="truncate">{options.find((option) => option.value === value)?.label}</span>
					<ChevronDownIcon data-icon="inline-end" class="text-muted-foreground" />
				</Button>
			{/snippet}
		</DropdownMenu.Trigger>
		<DropdownMenu.Content align="end" class="min-w-(--bits-dropdown-menu-anchor-width)">
			<DropdownMenu.RadioGroup {value} onValueChange={(next) => (value = next as T)}>
				{#each options as option (option.value)}
					<DropdownMenu.RadioItem value={option.value} closeOnSelect>{option.label}</DropdownMenu.RadioItem>
				{/each}
			</DropdownMenu.RadioGroup>
		</DropdownMenu.Content>
	</DropdownMenu.Root>
{/if}
