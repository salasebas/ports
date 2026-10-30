import { afterEach, expect, it, vi } from "vitest";
import { fadeRowOut } from "./table-motion";

afterEach(() => {
	document.body.replaceChildren();
	Reflect.deleteProperty(HTMLTableElement.prototype, "animate");
});

it("keeps an exiting snapshot outside the live table and keyboard selectors until its fade finishes", async () => {
	document.body.innerHTML = `
		<div data-table-frame><div><table data-slot="table">
			<thead><tr><th>Port</th><th>Server</th><th>Framework</th><th>Uptime</th><th>CPU</th><th>Memory</th><th>Actions</th></tr></thead>
			<tbody><tr data-row-id="port:3000"><td>:3000</td><td>Store</td><td>SvelteKit</td><td>1m</td><td>0%</td><td>8 MB</td><td><button id="open-3000">Open</button></td></tr></tbody>
		</table></div></div>`;
	const finished = Promise.withResolvers<void>();
	Object.defineProperty(HTMLTableElement.prototype, "animate", {
		configurable: true,
		value: vi.fn(() => ({ finished: finished.promise })),
	});
	const table = document.querySelector<HTMLTableElement>("[data-slot=table]")!;
	const row = table.tBodies[0].rows[0];

	const transition = fadeRowOut(row, { duration: 120 });

	expect(transition.duration).toBe(0);
	const snapshot = document.querySelector<HTMLTableElement>("[data-table-exit]")!;
	expect(snapshot.parentElement).toBe(table.closest("[data-table-frame]"));
	expect(snapshot.getAttribute("aria-hidden")).toBe("true");
	expect(snapshot.inert).toBe(true);
	expect(snapshot.querySelectorAll("col")).toHaveLength(7);
	expect(snapshot.textContent).toContain("Store");
	expect(snapshot.querySelector("[id], [data-row-id], [data-group-key]")).toBeNull();
	row.remove();
	expect(document.querySelector('[data-row-id="port:3000"]')).toBeNull();
	expect(document.getElementById("open-3000")).toBeNull();
	finished.resolve();
	await vi.waitFor(() => expect(document.querySelector("[data-table-exit]")).toBeNull());
});
