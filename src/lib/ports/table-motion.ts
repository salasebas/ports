import type { TransitionConfig } from "svelte/transition";

export function fadeRowOut(row: HTMLTableRowElement, { duration }: { duration: number }): TransitionConfig {
	if (!duration) return { duration: 0 };
	const table = row.closest("table");
	const frame = table?.closest<HTMLElement>("[data-table-frame]");
	const header = table?.tHead?.rows[0];
	if (!table || !frame || !header) throw new Error("Animated rows require a table frame and header");

	const bounds = row.getBoundingClientRect();
	const origin = frame.getBoundingClientRect();
	const opacity = getComputedStyle(row).opacity;
	const widths = Array.from(header.cells, (cell) => cell.getBoundingClientRect().width);
	const ghost = document.createElement("table");
	ghost.className = table.className;
	ghost.classList.add("bg-card");
	ghost.dataset.tableExit = "";
	ghost.setAttribute("aria-hidden", "true");
	ghost.inert = true;
	Object.assign(ghost.style, {
		position: "absolute",
		left: `${bounds.left - origin.left - frame.clientLeft}px`,
		top: `${bounds.top - origin.top - frame.clientTop}px`,
		width: `${bounds.width}px`,
		height: `${bounds.height}px`,
		pointerEvents: "none",
	});
	const columns = document.createElement("colgroup");
	for (const width of widths) {
		const column = document.createElement("col");
		column.style.width = `${width}px`;
		columns.append(column);
	}
	ghost.append(columns);
	const body = ghost.createTBody();
	body.append(row.cloneNode(true));
	const snapshot = body.rows[0];
	for (const property of ["position", "width", "height", "transform"]) snapshot.style.removeProperty(property);
	for (const element of ghost.querySelectorAll("[id], [data-row-id]")) {
		element.removeAttribute("id");
		element.removeAttribute("data-row-id");
	}
	frame.append(ghost);
	const animation = ghost.animate([{ opacity }, { opacity: 0 }], { duration });
	void animation.finished.then(
		() => ghost.remove(),
		() => ghost.remove(),
	);
	return { duration: 0 };
}
