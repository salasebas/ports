import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "svelte-sonner";
import { serverUrl } from "./format";
import { mockServers, mockSystemMemory } from "./mock";
import type { Server, SystemMemory } from "./types";

/**
 * UI state for the server list. Data comes from mocks for now; the Rust
 * backend will replace `mockServers` and the stop/restart stubs.
 */
export class PortsStore {
	servers = $state.raw<Server[]>(mockServers());
	system = $state.raw<SystemMemory>(mockSystemMemory);
	query = $state("");
	selectedPort = $state<number | null>(null);

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

	restart = (server: Server) => {
		toast.success(`Restarted ${server.name}`, {
			id: `restart-${server.port}`,
			description: `:${server.port}`,
		});
	};

	stop = (server: Server) => {
		this.servers = this.servers.filter((s) => s.port !== server.port);
		if (this.selectedPort === server.port) this.selectedPort = null;
		toast.success(`Stopped ${server.name}`, {
			id: `stop-${server.port}`,
			description: `:${server.port} is free`,
			action: {
				label: "Undo",
				onClick: () => {
					this.servers = [...this.servers, server].sort((a, b) => a.port - b.port);
				},
			},
		});
	};
}
