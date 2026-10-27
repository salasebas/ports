import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "svelte-sonner";
import { serverUrl } from "./format";
import { mockServers, mockSystemUsage } from "./mock";
import { preferences } from "./preferences.svelte";
import type { Server, SystemUsage } from "./types";

/** Runs a system action and reports its failure, e.g. a missing folder or editor, as a toast. */
async function attempt(action: Promise<unknown>): Promise<boolean> {
	try {
		await action;
		return true;
	} catch (error) {
		toast.error(String(error));
		return false;
	}
}

/**
 * UI state for the server list. Data comes from mocks for now; the Rust
 * backend will replace `mockServers` and the stop/restart stubs.
 */
export class PortsStore {
	servers = $state.raw<Server[]>(mockServers());
	system = $state.raw<SystemUsage>(mockSystemUsage);
	query = $state("");
	selectedPort = $state<number | null>(null);
	/** Server waiting for the user to confirm the stop. */
	pendingStop = $state.raw<Server | null>(null);

	filtered = $derived.by(() => {
		const q = this.query.trim().toLowerCase();
		if (!q) return this.servers;
		return this.servers.filter((server) =>
			[server.port, server.name, server.process, server.framework, server.branch]
				.filter(Boolean)
				.some((field) => String(field).toLowerCase().includes(q)),
		);
	});

	selected = $derived(this.servers.find((server) => server.port === this.selectedPort) ?? null);

	serversBytes = $derived(this.servers.reduce((sum, server) => sum + server.memoryBytes, 0));
	cpuPercent = $derived(this.servers.reduce((sum, server) => sum + server.cpuPercent, 0));

	select = (port: number | null) => {
		this.selectedPort = port;
	};

	refresh = () => {
		this.servers = mockServers();
	};

	open = async (server: Server) => {
		await openUrl(serverUrl(server.port));
	};

	copyUrl = async (server: Server) => {
		const url = serverUrl(server.port);
		if (await attempt(navigator.clipboard.writeText(url))) {
			toast.success("Copied", { id: "copy-url", description: url });
		}
	};

	openInEditor = (server: Server) =>
		attempt(invoke("open_in_editor", { folder: server.folder, editor: preferences.editor.current }));

	openInTerminal = (server: Server) => attempt(invoke("open_in_terminal", { folder: server.folder }));

	revealFolder = (server: Server) => attempt(invoke("reveal_folder", { folder: server.folder }));

	restart = (server: Server) => {
		toast.success(`Restarted ${server.name}`, {
			id: `restart-${server.port}`,
			description: `:${server.port}`,
		});
	};

	/** Entry point for every stop button: asks first when the preference is on. */
	requestStop = (server: Server) => {
		if (preferences.confirmStop.current) this.pendingStop = server;
		else this.stop(server);
	};

	stop = (server: Server) => {
		this.pendingStop = null;
		this.servers = this.servers.filter((s) => s.port !== server.port);
		if (this.selectedPort === server.port) this.selectedPort = null;
		toast.success(`Stopped ${server.name}`, {
			id: `stop-${server.port}`,
			description: `:${server.port} is free`,
		});
	};
}
