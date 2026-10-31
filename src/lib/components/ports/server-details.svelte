<script lang="ts">
	import ChevronDownIcon from "@lucide/svelte/icons/chevron-down";
	import BookmarkIcon from "@lucide/svelte/icons/bookmark";
	import BookmarkMinusIcon from "@lucide/svelte/icons/bookmark-minus";
	import ChevronLeftIcon from "@lucide/svelte/icons/chevron-left";
	import CircleAlertIcon from "@lucide/svelte/icons/circle-alert";
	import CodeIcon from "@lucide/svelte/icons/code";
	import CopyIcon from "@lucide/svelte/icons/copy";
	import CornerLeftUpIcon from "@lucide/svelte/icons/corner-left-up";
	import EllipsisIcon from "@lucide/svelte/icons/ellipsis";
	import ListXIcon from "@lucide/svelte/icons/list-x";
	import FolderOpenIcon from "@lucide/svelte/icons/folder-open";
	import PinIcon from "@lucide/svelte/icons/pin";
	import RotateCwIcon from "@lucide/svelte/icons/rotate-cw";
	import ShieldIcon from "@lucide/svelte/icons/shield";
	import SquareIcon from "@lucide/svelte/icons/square";
	import SquareTerminalIcon from "@lucide/svelte/icons/square-terminal";
	import WifiIcon from "@lucide/svelte/icons/wifi";
	import ZapIcon from "@lucide/svelte/icons/zap";
	import * as Alert from "$lib/components/ui/alert";
	import { Badge } from "$lib/components/ui/badge";
	import { Button } from "$lib/components/ui/button";
	import * as DropdownMenu from "$lib/components/ui/dropdown-menu";
	import * as Field from "$lib/components/ui/field";
	import { Separator } from "$lib/components/ui/separator";
	import { Switch } from "$lib/components/ui/switch";
	import {
		canStop,
		describeEvidence,
		exposeFlag,
		formatBytes,
		formatBytesText,
		formatDuration,
		frameworkLabel,
		isMeasured,
		lanUrl,
		serverProcessId,
		serverUrl,
		verifiedPage,
	} from "$lib/ports/format";
	import { fileManagerName } from "$lib/platform";
	import { hiddenPort, hiddenProcess, serverKey, type PortsStore } from "$lib/ports/ports-store.svelte";
	import { processKey, processTree } from "$lib/ports/resources";
	import { editorLabel, preferences } from "$lib/ports/preferences.svelte";
	import { invoke, isTauri } from "@tauri-apps/api/core";
	import type { PullLookup } from "$lib/ports/projects";
	import type { Server } from "$lib/ports/types";
	import { settings } from "$lib/settings.svelte";
	import { cn } from "$lib/utils";
	import CpuBars from "./cpu-bars.svelte";
	import PortLabel from "./port-label.svelte";
	import QrCode from "./qr-code.svelte";
	import ServerIcon from "./server-icon.svelte";
	import Sparkline from "./sparkline.svelte";

	let {
		server,
		store,
		onBack,
		class: className,
	}: { server: Server; store: PortsStore; onBack?: () => void; class?: string } = $props();

	const id = $props.id();

	const memory = $derived(formatBytes(server.memoryBytes));
	const leaking = $derived(server.status === "attention" && server.responding);
	const page = $derived(verifiedPage(server));
	const lan = $derived(lanUrl(server));
	const idle = $derived(server.status === "idle");
	const alerts = $derived(settings.current.alerts);
	const alertNote = $derived.by(() => {
		if (server.kind !== "dev" && server.kind !== "database") return null;
		const above = alerts.thresholdBytes === null ? null : `above ${formatBytesText(alerts.thresholdBytes)}`;
		if (alerts.enabled && above) return `Alerts if it keeps growing or goes ${above}`;
		if (alerts.enabled) return "Alerts if it keeps growing";
		return above && `Alerts ${above}`;
	});
	const scope = $derived(
		serverKey(server).startsWith("project:") ? "every port of this project" : `:${server.port} only`,
	);

	const supervisorNames = { homebrew: "Homebrew", pm2: "PM2", supervisord: "supervisord" };
	const supervisor = $derived(server.supervisor);
	const managed = $derived(supervisor?.controllable === true);
	const others = $derived(server.target?.kind === "process" ? store.sameProgram(server).length - 1 : 0);
	const hasMore = $derived(!managed || Boolean(server.parent) || others > 0);
	const indents = ["ps-0", "ps-3", "ps-6", "ps-9", "ps-12"];

	const tree = $derived(processTree(server));
	const sharedWith = $derived.by(() => {
		const shared = new Map<string, string[]>();
		for (const other of store.servers) {
			if (other.id === server.id) continue;
			for (const resource of other.resources) {
				const key = processKey(resource.process);
				shared.set(key, [...(shared.get(key) ?? []), `:${other.port}`]);
			}
		}
		return shared;
	});

	const facts = $derived(
		[
			["Process", `${server.process} · ${serverProcessId(server)}`],
			["Parent", server.parent && `${server.parent.name} · PID ${server.parent.process.pid}`],
			["Managed by", supervisor && [supervisorNames[supervisor.kind], supervisor.name].filter(Boolean).join(" · ")],
			["Framework", frameworkLabel(server) || "Unknown"],
			["Page", page?.title],
			["Package", server.projectName !== server.name && server.projectName],
			["Branch", server.branch],
			["Worktree", server.worktree],
			[
				"Repository",
				server.repositoryName && server.repository
					? `${server.repositoryName} · ${server.repository}`
					: server.repositoryName,
			],
			["Folder", server.folder],
			[
				"Runs in",
				server.cwdDeleted
					? `${server.cwd ?? "Its folder"} (deleted)`
					: server.cwd !== server.folder && server.cwd !== "/" && server.cwd,
			],
			["Listens on", server.addresses.join(", ")],
			["Command", server.command],
		].filter((fact): fact is [string, string] => Boolean(fact[1])),
	);

	const saved = $derived(store.projectFor(server));
	let pull = $state<PullLookup | { state: "loading" } | null>(null);
	const folder = $derived(server.folder);
	const branch = $derived(server.branch);
	const port = $derived(server.port);

	$effect(() => {
		if (!folder || !branch || !isTauri()) {
			pull = null;
			return;
		}
		let current = true;
		pull = { state: "loading" };
		void invoke<PullLookup>("pull_request", { folder, port })
			.then((result) => {
				if (current) pull = result;
			})
			.catch((error: unknown) => {
				if (current) pull = { state: "offline", message: String(error) };
			});
		return () => {
			current = false;
		};
	});

	const confidence = $derived(server.attribution.confidence);
	const verdict = $derived.by(() => {
		if (confidence === "likely")
			return `Only ${server.attribution.evidence.some((e) => e.source === "wellKnownPort") ? "the port number" : "the project's dependencies"} suggest ${server.framework}; its command doesn't confirm it.`;
		if (confidence === "unknown") return "Nothing in its command, project or image names a framework.";
		return null;
	});
</script>

<article class={cn("flex flex-col gap-5", className)} aria-label="{server.name} on port {server.port}">
	<header class="flex flex-col gap-1">
		<div class="relative flex min-h-8 items-center justify-center">
			{#if onBack}
				<Button variant="ghost" size="icon-sm" class="absolute left-0" aria-label="Back" onclick={onBack}>
					<ChevronLeftIcon />
				</Button>
			{/if}
			<h2 class="flex min-w-0 items-center gap-1.5 px-10 text-sm font-semibold">
				<ServerIcon {server} class="size-4" />
				{#if store.isPinned(server)}
					<PinIcon class="size-3.5 shrink-0 text-muted-foreground" aria-label="Pinned" />
				{/if}
				<span class="truncate">{server.name}</span>
			</h2>
		</div>
		<PortLabel port={server.port} class="text-4xl" />
		<p class="text-sm text-muted-foreground">
			{idle ? "Idle for" : "Running for"}
			{formatDuration(Date.now() - (idle ? server.lastActiveAt : server.startedAt))}
		</p>
	</header>

	{#if !server.responding}
		<Alert.Root class="text-warning">
			<CircleAlertIcon />
			<Alert.Title>Not responding</Alert.Title>
			<Alert.Description>
				It holds :{server.port} but didn't answer the last check. It may be stuck in a loop{server.restartable
					? "; restarting it usually helps"
					: ""}.
			</Alert.Description>
		</Alert.Root>
	{/if}

	<div class="flex flex-col gap-2">
		<div class="flex gap-2">
			<Button size="lg" class="flex-1" onclick={() => store.open(server)}>
				Open port {server.port}
			</Button>
			<Button variant="secondary" size="icon-lg" aria-label="Copy URL" onclick={() => store.copyUrl(server)}>
				<CopyIcon />
			</Button>
		</div>
		{#if server.folder}
			<div class="grid grid-cols-3 gap-2">
				<Button
					variant="secondary"
					size="sm"
					disabled={store.isBusy(server, "editor")}
					onclick={() => store.openInEditor(server)}
				>
					<CodeIcon data-icon="inline-start" />
					{editorLabel(preferences.editor.current)}
				</Button>
				<Button
					variant="secondary"
					size="sm"
					disabled={store.isBusy(server, "terminal")}
					onclick={() => store.openInTerminal(server)}
				>
					<SquareTerminalIcon data-icon="inline-start" />
					Terminal
				</Button>
				<Button
					variant="secondary"
					size="sm"
					disabled={store.isBusy(server, "folder")}
					onclick={() => store.revealFolder(server)}
				>
					<FolderOpenIcon data-icon="inline-start" />
					{fileManagerName}
				</Button>
			</div>
			{#if server.kind !== "container"}
				{#if saved}
					<div class="grid grid-cols-2 gap-2">
						<Button variant="secondary" size="sm" onclick={() => store.saveProject(server)}>
							<BookmarkIcon data-icon="inline-start" />
							Update saved
						</Button>
						<Button variant="secondary" size="sm" onclick={() => store.forgetSaved(saved)}>
							<BookmarkMinusIcon data-icon="inline-start" />
							Unsave
						</Button>
					</div>
				{:else}
					<Button variant="secondary" size="sm" onclick={() => store.saveProject(server)}>
						<BookmarkIcon data-icon="inline-start" />
						Save project
					</Button>
				{/if}
				{#if saved}
					<p class="text-xs text-muted-foreground">
						Saved to start {saved.command} at {saved.expectedAddress}:{saved.expectedPort}. Restart above still signals
						this process.
					</p>
				{/if}
			{/if}
		{/if}
		{#if canStop(server)}
			<div
				class={cn(
					"grid gap-2",
					server.restartable ? "grid-cols-2" : "grid-cols-1",
					hasMore && (server.restartable ? "grid-cols-[1fr_1fr_auto]" : "grid-cols-[1fr_auto]"),
				)}
			>
				{#if server.restartable}
					<Button
						variant="secondary"
						size="sm"
						disabled={store.isBusy(server, "control")}
						onclick={() => store.request("restart", server)}
					>
						<RotateCwIcon data-icon="inline-start" />
						Restart
					</Button>
				{/if}
				<Button
					variant="destructive"
					size="sm"
					disabled={store.isBusy(server, "control")}
					onclick={() => store.request("stop", server)}
				>
					<SquareIcon data-icon="inline-start" class="fill-current" />
					Stop
				</Button>
				{#if hasMore}
					<DropdownMenu.Root>
						<DropdownMenu.Trigger disabled={store.isBusy(server, "control")}>
							{#snippet child({ props })}
								<Button {...props} variant="secondary" size="icon-sm" aria-label="More ways to stop">
									<EllipsisIcon />
								</Button>
							{/snippet}
						</DropdownMenu.Trigger>
						<DropdownMenu.Content align="end" class="w-60">
							{#if !managed}
								<DropdownMenu.Item onSelect={() => store.request("forceKill", server)}>
									<ZapIcon />
									Force Kill
								</DropdownMenu.Item>
							{/if}
							{#if server.parent}
								<DropdownMenu.Item onSelect={() => store.request("stopParent", server)}>
									<CornerLeftUpIcon />
									Stop parent {server.parent.name}
								</DropdownMenu.Item>
							{/if}
							{#if others > 0}
								<DropdownMenu.Item onSelect={() => store.request("stopAll", server)}>
									<ListXIcon />
									Stop every {server.process} server
								</DropdownMenu.Item>
							{/if}
						</DropdownMenu.Content>
					</DropdownMenu.Root>
				{/if}
			</div>
			{#if supervisor && !supervisor.controllable}
				<p class="text-xs text-muted-foreground">
					{supervisorNames[supervisor.kind]} may start it again after Stop; Ports can't control it yet.
				</p>
			{:else if managed}
				<p class="text-xs text-muted-foreground">
					Stop and Restart go through {supervisorNames[supervisor!.kind]}, so it doesn't start again on its own.{supervisor!
						.kind === "homebrew"
						? " Stop also keeps it from starting at login."
						: ""}
				</p>
			{/if}
		{/if}
	</div>

	<Separator />

	<dl class="grid grid-cols-[5.5rem_1fr] gap-x-4 gap-y-2.5 text-sm">
		{#each facts as [label, value] (label)}
			<dt class="text-muted-foreground">{label}</dt>
			<dd class={cn("flex min-w-0 items-center gap-1.5", label === "Command" && "font-mono text-xs leading-5")}>
				<span class="truncate" title={value}>{value}</span>
				{#if label === "Framework" && confidence === "likely"}
					<Badge variant="outline">Likely</Badge>
				{/if}
			</dd>
		{/each}
		{#if server.exposed}
			<dt class="text-muted-foreground">Network</dt>
			<dd><Badge variant="secondary"><WifiIcon data-icon="inline-start" />Reachable from other devices</Badge></dd>
		{/if}
		{#if server.protected}
			<dt class="text-muted-foreground">Clean up</dt>
			<dd><Badge variant="secondary"><ShieldIcon data-icon="inline-start" />Protected</Badge></dd>
		{/if}
	</dl>

	<section class="flex flex-col gap-1.5" aria-label="Identified by">
		<p class="text-sm text-muted-foreground">Identified by</p>
		<ul class="flex flex-col gap-1 text-xs">
			{#each server.attribution.evidence as evidence, i (i)}
				<li>{describeEvidence(evidence)}</li>
			{/each}
			{#if page}
				<li>Answers {page.scheme.toUpperCase()} at /</li>
			{:else if server.web.state === "none"}
				<li>Answers neither HTTPS nor HTTP at /</li>
			{/if}
		</ul>
		{#if verdict}
			<p class="text-xs text-muted-foreground">{verdict}</p>
		{/if}
	</section>

	<section class="flex flex-col gap-1.5" aria-label="Session">
		<p class="text-sm text-muted-foreground">Session</p>
		{#if server.session}
			<p class="text-sm">{server.session.label}</p>
			<ul class="flex flex-col gap-1 text-xs">
				{#each server.session.evidence as evidence, i (i)}
					<li>
						{evidence.source === "ancestor" ? `Parent process ${evidence.name}` : `Environment ${evidence.name}`}
					</li>
				{/each}
			</ul>
			{#if server.session.resume.state === "available"}
				<Button
					variant="secondary"
					size="sm"
					class="self-start"
					onclick={() =>
						store.revealSession(server.session?.resume.state === "available" ? server.session.resume.path : "")}
				>
					{server.session.resume.label}
				</Button>
			{:else}
				<p class="text-xs text-muted-foreground">{server.session.resume.message}</p>
			{/if}
		{:else}
			<p class="text-xs text-muted-foreground">
				Session unknown. Nothing in the process tree or its environment names a terminal or coding agent.
			</p>
		{/if}
	</section>

	{#if server.branch}
		<Separator />
		<section class="flex flex-col gap-2" aria-label="Remote">
			<p class="text-sm text-muted-foreground">Remote</p>
			{#if !pull || pull.state === "loading"}
				<p class="text-xs text-muted-foreground">Looking up the pull request…</p>
			{:else if pull.state === "found"}
				<Button
					class="justify-start"
					variant="secondary"
					size="sm"
					onclick={() => store.openRemote(pull && pull.state === "found" ? pull.pull.url : "")}
				>
					PR #{pull.pull.number}: {pull.pull.title}
				</Button>
				{#if pull.preview}
					<Button
						class="justify-start"
						variant="secondary"
						size="sm"
						onclick={() => store.openRemote(pull && pull.state === "found" && pull.preview ? pull.preview.url : "")}
					>
						{pull.preview.label}
					</Button>
					<p class="text-xs text-muted-foreground">Remote preview. This is not the local server on :{server.port}.</p>
				{/if}
			{:else if pull.state === "ambiguous"}
				<p class="text-xs text-muted-foreground">{pull.message}</p>
				{#each pull.pulls as item (item.url)}
					<Button class="justify-start" variant="ghost" size="sm" onclick={() => store.openRemote(item.url)}>
						PR #{item.number}: {item.title}
					</Button>
				{/each}
			{:else}
				<p class="text-xs text-muted-foreground">{pull.message}</p>
			{/if}
		</section>
	{/if}

	{#if page && lan}
		<Separator />
		<section class="flex items-center gap-4" aria-label="Open on another device">
			<QrCode value={lan} class="size-28 shrink-0" />
			<div class="flex min-w-0 flex-col gap-2">
				<p class="text-sm font-medium">Open on your phone</p>
				<p class="text-xs text-muted-foreground">Scan the code with a device on the same network.</p>
				<Button
					variant="secondary"
					size="sm"
					class="justify-start"
					aria-label="Copy {lan}"
					onclick={() => store.copyUrl(server, lan)}
				>
					<CopyIcon data-icon="inline-start" />
					<span class="truncate font-mono text-xs">{server.lanHost}</span>
				</Button>
			</div>
		</section>
	{:else if page && server.exposed}
		<Separator />
		<p class="text-xs text-muted-foreground">
			Other devices can reach it, but this machine isn't on a network right now.
		</p>
	{:else if page && server.kind === "dev"}
		<Separator />
		<p class="text-xs text-muted-foreground">
			Only this machine can open it. To open it on your phone, start it with
			<code class="font-mono text-foreground">{exposeFlag(server) ?? "--host 0.0.0.0"}</code>
			so it listens on the network too.
		</p>
	{:else if server.web.state === "none"}
		<Separator />
		<p class="text-xs text-muted-foreground">
			No web page answered on :{server.port}, so it may speak another protocol. Open tries
			<code class="font-mono text-foreground">{serverUrl(server)}</code>.
		</p>
	{/if}

	{#if isMeasured(server)}
		<Separator />

		<section class="flex flex-col gap-2" aria-label="Memory">
			<div class="flex items-baseline justify-between">
				<p class="text-sm text-muted-foreground">
					Memory
					<span class={cn("font-mono text-foreground tabular-nums", leaking && "text-warning")}>
						{memory.value}
						{memory.unit}
					</span>
				</p>
				<span class="font-mono text-xs text-muted-foreground">10 min</span>
			</div>
			<Sparkline
				values={server.memoryHistory}
				fromZero
				area
				threshold={alerts.thresholdBytes ?? undefined}
				class={cn("h-20 w-full", leaking ? "text-warning" : "text-foreground")}
			/>
			{#if alertNote}
				<p class="text-xs text-muted-foreground">{alertNote}</p>
			{/if}
		</section>

		<section class="flex flex-col gap-2" aria-label="CPU">
			<p class="text-sm text-muted-foreground">
				CPU <span class="font-mono text-foreground tabular-nums">{Math.round(server.cpuPercent)}%</span>
			</p>
			<CpuBars values={server.cpuHistory} class="h-14 w-full" />
		</section>

		{#if tree.length > 1}
			<section class="flex flex-col gap-2" aria-label="Processes">
				<p class="text-sm text-muted-foreground">Processes <span class="text-foreground">{tree.length}</span></p>
				<ul class="flex flex-col gap-1.5 text-sm">
					{#each tree as { usage, depth } (processKey(usage.process))}
						{@const shared = sharedWith.get(processKey(usage.process))}
						<li class={cn("flex items-center gap-2", indents[Math.min(depth, indents.length - 1)])}>
							<span class="flex min-w-0 flex-1 items-center gap-1.5">
								<span class="truncate">{usage.name}</span>
								<span class="shrink-0 font-mono text-xs text-muted-foreground">{usage.process.pid}</span>
								{#each usage.ports as port (port)}
									<Badge variant="secondary" class="font-mono">:{port}</Badge>
								{/each}
								{#if shared}
									<span class="shrink-0 text-xs text-muted-foreground">also {shared.join(", ")}</span>
								{/if}
							</span>
							<span class="w-16 shrink-0 text-right font-mono text-xs tabular-nums">
								{formatBytesText(usage.memoryBytes)}
							</span>
							<span class="w-9 shrink-0 text-right font-mono text-xs text-muted-foreground tabular-nums">
								{Math.round(usage.cpuPercent)}%
							</span>
						</li>
					{/each}
				</ul>
			</section>
		{/if}
	{/if}

	<Separator />

	<Field.Group class="gap-4" aria-label="In the list">
		<Field.Field orientation="horizontal">
			<Field.Content>
				<Field.Label for="{id}-pin">Pin to top</Field.Label>
				<Field.Description>Applies to {scope}.</Field.Description>
			</Field.Content>
			<Switch id="{id}-pin" bind:checked={() => store.isPinned(server), () => store.togglePin(server)} />
		</Field.Field>
		<Field.Field orientation="horizontal">
			<Field.Content>
				<Field.Label for="{id}-protect">Protect from clean up</Field.Label>
				<Field.Description>Applies to {scope}.</Field.Description>
			</Field.Content>
			<Switch id="{id}-protect" bind:checked={() => server.protected, (value) => store.setProtected(server, value)} />
		</Field.Field>
		<div class="grid grid-cols-2 gap-2">
			<Button variant="secondary" size="sm" onclick={() => (store.renaming = server)}>Rename…</Button>
			<DropdownMenu.Root>
				<DropdownMenu.Trigger>
					{#snippet child({ props })}
						<Button {...props} variant="secondary" size="sm">
							Hide
							<ChevronDownIcon data-icon="inline-end" />
						</Button>
					{/snippet}
				</DropdownMenu.Trigger>
				<DropdownMenu.Content align="end" class="w-56">
					<DropdownMenu.Item onSelect={() => store.hide(hiddenPort(server.port))}>
						Hide :{server.port}
					</DropdownMenu.Item>
					<DropdownMenu.Item onSelect={() => store.hide(hiddenProcess(server.process))}>
						Hide every port of {server.process}
					</DropdownMenu.Item>
				</DropdownMenu.Content>
			</DropdownMenu.Root>
		</div>
	</Field.Group>
</article>
