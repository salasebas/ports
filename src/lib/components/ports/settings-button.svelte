<script lang="ts">
	import SettingsIcon from "@lucide/svelte/icons/settings";
	import { Button } from "$lib/components/ui/button";
	import * as Tooltip from "$lib/components/ui/tooltip";
	import { openSettings } from "$lib/settings.svelte";
	import { isMac } from "$lib/platform";
</script>

<Tooltip.Root>
	<Tooltip.Trigger>
		{#snippet child({ props })}
			<Button {...props} variant="ghost" size="icon-sm" aria-label="Settings" onclick={openSettings}>
				<SettingsIcon />
			</Button>
		{/snippet}
	</Tooltip.Trigger>
	<Tooltip.Content>Settings {isMac ? "⌘," : "Ctrl+,"}</Tooltip.Content>
</Tooltip.Root>
