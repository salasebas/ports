import { isMac } from "$lib/platform";

export type ListCommand = "next" | "previous" | "details" | "open" | "stop";

export function isEditable(element: Element | null): boolean {
	if (element instanceof HTMLInputElement) {
		return !["button", "checkbox", "color", "file", "radio", "range", "reset", "submit"].includes(element.type);
	}
	return (
		element instanceof HTMLTextAreaElement ||
		element instanceof HTMLSelectElement ||
		(element instanceof HTMLElement && element.isContentEditable === true)
	);
}

export function listCommand(event: KeyboardEvent, mac = isMac): ListCommand | null {
	if (event.altKey || event.shiftKey || event.isComposing) return null;
	const command = mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
	if (command) {
		if (event.key.toLowerCase() === "o") return "open";
		if (event.key === "Backspace" || event.key === "Delete") return "stop";
		return null;
	}
	if (event.metaKey || event.ctrlKey) return null;
	if (event.key === "ArrowDown") return "next";
	if (event.key === "ArrowUp") return "previous";
	if (event.key === "Enter") return "details";
	return null;
}

export function typedText(event: KeyboardEvent): string | null {
	if (event.metaKey || event.ctrlKey || event.altKey || event.isComposing) return null;
	return event.key.length === 1 && event.key !== " " ? event.key : null;
}

export function step(ids: readonly string[], current: string | null, delta: 1 | -1): string | null {
	if (!ids.length) return null;
	const index = current === null ? -1 : ids.indexOf(current);
	if (index === -1) return ids[0];
	return ids[Math.min(ids.length - 1, Math.max(0, index + delta))];
}
