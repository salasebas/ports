import { afterEach, expect, it, vi } from "vitest";
import { isMac } from "$lib/platform";
import { ListKeyboard } from "./list-keyboard.svelte";
import { mockSnapshot } from "./mock";
import { PortsStore } from "./ports-store.svelte";

let cleanup = () => {};
afterEach(() => {
	cleanup();
	document.body.replaceChildren();
});

/** A list with a search field and one focusable row per server, driven by a `ListKeyboard`. */
function list() {
	const store = new PortsStore();
	store.servers = mockSnapshot().servers.filter((server) => server.kind === "dev");
	const request = vi.spyOn(store, "request").mockResolvedValue();
	const search = document.createElement("input");
	document.body.append(search);
	for (const server of store.filtered) {
		const row = Object.assign(document.createElement("button"), { tabIndex: 0 });
		row.dataset.rowId = server.id;
		document.body.append(row);
	}
	let keyboard!: ListKeyboard;
	cleanup = $effect.root(() => {
		keyboard = new ListKeyboard({ store, rows: () => store.filtered, search: () => search, details: () => {} });
	});
	const stop = () => {
		const event = new KeyboardEvent("keydown", { key: "Backspace", metaKey: isMac, ctrlKey: !isMac, cancelable: true });
		keyboard.onkeydown(event);
	};
	return { store, request, stop };
}

// #18: ⌘⌫ must not stop a server the user never moved to.
it("stops nothing while no row has focus", () => {
	const { request, stop } = list();
	document.body.focus();
	stop();
	expect(request).not.toHaveBeenCalled();
});

it("stops the server whose row has focus, even one reached with Tab", () => {
	const { store, request, stop } = list();
	const [, second] = store.filtered;
	document.querySelector<HTMLElement>(`[data-row-id="${second.id}"]`)?.focus();
	stop();
	expect(request).toHaveBeenCalledWith("stop", second);
});
