import { tick, untrack } from "svelte";
import { canStop } from "./format";
import { isEditable, listCommand, step, typedText } from "./keyboard";
import type { PortsStore } from "./ports-store.svelte";
import type { Server } from "./types";

const rowSelector = (id: string) => `[data-row-id="${CSS.escape(id)}"]`;

interface Options {
	store: PortsStore;
	rows: () => Server[];
	search: () => HTMLInputElement | null;
	details: (server: Server) => void;
}

export class ListKeyboard {
	#options: Options;
	#chosen = $state<string | null>(null);
	#index = 0;

	activeId = $derived.by(() => {
		const ids = this.#options.rows().map((server) => server.id);
		if (!ids.length) return null;
		if (this.#chosen !== null && ids.includes(this.#chosen)) return this.#chosen;
		return ids[Math.min(this.#index, ids.length - 1)];
	});

	constructor(options: Options) {
		this.#options = options;
		$effect(() => {
			void options.store.query;
			untrack(() => {
				this.#chosen = null;
				this.#index = 0;
			});
		});
		let previous: string | null = null;
		$effect(() => {
			const id = this.activeId;
			const lost = previous !== null && document.activeElement === document.body;
			const gone = previous !== null && !document.querySelector(rowSelector(previous));
			previous = id;
			if (id && lost && gone) this.focus(id);
		});
	}

	get active(): Server | null {
		return this.#options.rows().find((server) => server.id === this.activeId) ?? null;
	}

	choose = (id: string) => {
		this.#chosen = id;
		this.#index = Math.max(
			0,
			this.#options.rows().findIndex((server) => server.id === id),
		);
	};

	focus = (id: string) => {
		void tick().then(() => document.querySelector<HTMLElement>(rowSelector(id))?.focus());
	};

	restore = (id: string | null) => {
		if (id && this.#options.rows().some((server) => server.id === id)) this.choose(id);
		if (this.activeId) this.focus(this.activeId);
	};

	#move(delta: 1 | -1, onRow: boolean) {
		const ids = this.#options.rows().map((server) => server.id);
		const next = onRow ? step(ids, this.activeId, delta) : this.activeId;
		if (onRow && delta === -1 && ids.indexOf(this.activeId ?? "") === 0) {
			this.#options.search()?.focus();
			return;
		}
		if (!next) return;
		this.choose(next);
		this.focus(next);
	}

	onkeydown = (event: KeyboardEvent) => {
		if (event.defaultPrevented) return;
		if (document.querySelector('[role="alertdialog"], [role="menu"]')) return;
		const { store } = this.#options;
		const focused = document.activeElement;
		const search = this.#options.search();
		const onRow = focused instanceof HTMLElement && focused.matches("[data-row-id]");
		if (onRow && focused.dataset.rowId) this.choose(focused.dataset.rowId);
		const inSearch = focused !== null && focused === search;
		const editing = isEditable(focused);
		const detailsOpen = store.selected !== null || store.selectedProject !== null;
		const target = store.selected ?? (onRow ? this.active : null);
		switch (listCommand(event)) {
			case "open":
				if (!target || (editing && !inSearch)) return;
				event.preventDefault();
				void store.open(target);
				return;
			case "stop":
				if (!target || editing || !canStop(target)) return;
				event.preventDefault();
				void store.request("stop", target);
				return;
			case "next":
			case "previous":
				if (detailsOpen || (editing && !inSearch)) return;
				event.preventDefault();
				this.#move(event.key === "ArrowDown" ? 1 : -1, onRow);
				return;
			case "details": {
				const active = this.active;
				if (detailsOpen || !active || !(onRow || inSearch)) return;
				event.preventDefault();
				this.#options.details(active);
				return;
			}
		}
		const text = typedText(event);
		if (!text || detailsOpen || editing || !search) return;
		event.preventDefault();
		store.query += text;
		search.focus();
	};
}
